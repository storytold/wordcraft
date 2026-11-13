//! Linux: draw on the GPU that drives the screen.
//!
//! egui-wgpu picks the discrete GPU by default (`PowerPreference::HighPerformance`). On a laptop
//! whose integrated GPU drives the panel, every frame drawn on the discrete one is copied across
//! to it, and the discrete GPU leaves its idle state whenever the window redraws: on an Intel HD
//! 530 + NVIDIA Quadro M4000M laptop, from 7 W at P8 to over 20 W at P0/P1. Forcing the integrated
//! GPU instead breaks desktops whose screen is on the discrete card (storytold/pdfcraft#96), and a
//! desktop with a monitor on each card needs the discrete one (storytold/pdfcraft#445). So the
//! choice follows the connected displays, as in craft-anything (storytold/craft-anything#571):
//!
//! 1. `WGPU_ADAPTER_NAME` (part of the adapter's name) or `WGPU_POWER_PREF` (`high`, `low`,
//!    `none`), when set, decide;
//! 2. a connected built-in panel: the GPU behind it;
//! 3. displays on only one GPU: that GPU;
//! 4. otherwise (displays on several GPUs, or nothing readable): the discrete GPU, as before.

mod drm;

use drm::DisplayCard;
use eframe::wgpu::{DeviceType, PowerPreference};

/// What we know of one adapter wgpu offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    /// `0000:05:00.0`, lowercase; empty when the driver doesn't say (always, under GL).
    pub pci_slot: String,
    pub device_type: DeviceType,
    pub gl: bool,
}

/// What the person asked for through wgpu's environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Asked {
    pub adapter_name: Option<String>,
    pub power: Option<PowerPreference>,
}

/// Why an adapter was chosen, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    AskedByName,
    AskedPower,
    BuiltInPanel,
    OnlyGpuWithDisplays,
    Fallback,
}

/// The index of the adapter to draw with, and why; `None` when there is none.
pub(crate) fn choose(candidates: &[Candidate], cards: &[DisplayCard], asked: &Asked) -> Option<(usize, Why)> {
    if let Some(want) = asked.adapter_name.as_deref().map(str::to_lowercase).filter(|w| !w.is_empty()) {
        // As wgpu's own `WGPU_ADAPTER_NAME`: part of the name, any case. No match: the usual choice.
        if let Some(i) = candidates.iter().position(|c| c.name.to_lowercase().contains(&want)) {
            return Some((i, Why::AskedByName));
        }
        log::warn!("WGPU_ADAPTER_NAME={want}: no graphics adapter has that in its name");
    }
    if let Some(power) = asked.power {
        return by_kind(candidates, power).map(|i| (i, Why::AskedPower));
    }
    let panels: Vec<&DisplayCard> = cards.iter().filter(|c| c.built_in_panel).collect();
    let with_displays: Vec<&DisplayCard> = cards.iter().filter(|c| c.displays > 0).collect();
    let target = if !panels.is_empty() {
        Some((panels, Why::BuiltInPanel))
    } else if with_displays.len() == 1 {
        Some((with_displays, Why::OnlyGpuWithDisplays))
    } else {
        None
    };
    if let Some((wanted, why)) = target {
        // Vulkan before GL for the same card; otherwise wgpu's order.
        let found = candidates.iter().enumerate().filter(|(_, c)| wanted.iter().any(|card| drives(card, c, cards))).min_by_key(|(_, c)| c.gl);
        if let Some((i, _)) = found {
            return Some((i, why));
        }
    }
    by_kind(candidates, PowerPreference::HighPerformance).map(|i| (i, Why::Fallback))
}

/// Whether `card` is the GPU behind `adapter`: by PCI address when the driver gives one, else by
/// PCI ids, which count only when no other card has the same ids (two cards of one model); GL
/// gives only a vendor, which counts only when no other card has that vendor.
fn drives(card: &DisplayCard, adapter: &Candidate, all: &[DisplayCard]) -> bool {
    if !adapter.pci_slot.is_empty() && !card.pci_slot.is_empty() {
        return adapter.pci_slot == card.pci_slot;
    }
    if adapter.device != 0 {
        return adapter.vendor == card.vendor
            && adapter.device == card.device
            && all.iter().filter(|c| c.vendor == card.vendor && c.device == card.device).count() == 1;
    }
    adapter.vendor != 0 && adapter.vendor == card.vendor && all.iter().filter(|c| c.vendor == card.vendor).count() == 1
}

/// The first adapter in wgpu's own order for that power preference.
fn by_kind(candidates: &[Candidate], power: PowerPreference) -> Option<usize> {
    let rank = |t: DeviceType| match (power, t) {
        (PowerPreference::None, _) => 0,
        (PowerPreference::LowPower, DeviceType::IntegratedGpu) | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 1,
        (_, DeviceType::IntegratedGpu | DeviceType::DiscreteGpu) => 2,
        (_, DeviceType::Other) => 3,
        (_, DeviceType::VirtualGpu) => 4,
        (_, DeviceType::Cpu) => 5,
    };
    candidates.iter().enumerate().min_by_key(|(_, c)| rank(c.device_type)).map(|(i, _)| i)
}

#[cfg(target_os = "linux")]
impl Candidate {
    fn of(info: &eframe::wgpu::AdapterInfo) -> Self {
        Candidate {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            pci_slot: info.device_pci_bus_id.to_ascii_lowercase(),
            device_type: info.device_type,
            gl: info.backend == eframe::wgpu::Backend::Gl,
        }
    }
}

#[cfg(target_os = "linux")]
impl Asked {
    fn from_env() -> Self {
        Asked { adapter_name: std::env::var("WGPU_ADAPTER_NAME").ok(), power: PowerPreference::from_env() }
    }
}

/// The adapter that draws the window, among those that can draw to it.
#[cfg(target_os = "linux")]
pub(crate) fn pick_adapter(adapters: &[eframe::wgpu::Adapter], surface: Option<&eframe::wgpu::Surface<'_>>) -> Result<eframe::wgpu::Adapter, String> {
    let usable: Vec<&eframe::wgpu::Adapter> = adapters.iter().filter(|a| surface.is_none_or(|s| a.is_surface_supported(s))).collect();
    let candidates: Vec<Candidate> = usable.iter().map(|a| Candidate::of(&a.get_info())).collect();
    let cards = drm::display_cards();
    let picked = choose(&candidates, &cards, &Asked::from_env()).and_then(|(i, why)| Some((*usable.get(i)?, why)));
    match picked {
        Some((a, why)) => {
            log::info!("drawing with {:?} ({why:?}); displays: {cards:?}", a.get_info());
            Ok(a.clone())
        }
        None => Err(format!("None of the {} graphics adapters found can draw this window.", adapters.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMD: (u32, u32, &str) = (0x1002, 0x1638, "0000:05:00.0");
    const NVIDIA: (u32, u32, &str) = (0x10de, 0x25a2, "0000:01:00.0");
    const INTEL: (u32, u32, &str) = (0x8086, 0xa780, "0000:00:02.0");

    fn vulkan(name: &str, (vendor, device, slot): (u32, u32, &str), device_type: DeviceType) -> Candidate {
        Candidate { name: name.into(), vendor, device, pci_slot: slot.into(), device_type, gl: false }
    }

    fn gl(name: &str, vendor: u32, device_type: DeviceType) -> Candidate {
        Candidate { name: name.into(), vendor, device: 0, pci_slot: String::new(), device_type, gl: true }
    }

    fn card((vendor, device, slot): (u32, u32, &str), displays: u32, built_in_panel: bool) -> DisplayCard {
        DisplayCard { vendor, device, pci_slot: slot.into(), displays, built_in_panel }
    }

    /// What wgpu offers on a laptop whose AMD iGPU drives the panel, in its order (Vulkan, then GL).
    fn laptop_adapters() -> Vec<Candidate> {
        vec![
            vulkan("AMD Radeon Graphics (RADV RENOIR)", AMD, DeviceType::IntegratedGpu),
            vulkan("NVIDIA GeForce RTX 3050 Laptop GPU", NVIDIA, DeviceType::DiscreteGpu),
            gl("AMD Radeon Graphics (radeonsi, renoir)", 0x1002, DeviceType::IntegratedGpu),
        ]
    }

    /// The iGPU drives the panel; egui-wgpu's default drew on the discrete GPU.
    #[test]
    fn a_laptop_draws_on_the_gpu_behind_its_panel() {
        let cards = [card(NVIDIA, 0, false), card(AMD, 1, true)];
        assert_eq!(choose(&laptop_adapters(), &cards, &Asked::default()), Some((0, Why::BuiltInPanel)));
        // The same with the discrete GPU listed first.
        let mut swapped = laptop_adapters();
        swapped.swap(0, 1);
        assert_eq!(choose(&swapped, &cards, &Asked::default()), Some((1, Why::BuiltInPanel)));
    }

    /// The panel's GPU stays the choice with an external monitor on the discrete card as well.
    #[test]
    fn the_panel_wins_over_a_monitor_on_the_other_card() {
        let cards = [card(NVIDIA, 1, false), card(AMD, 1, true)];
        assert_eq!(choose(&laptop_adapters(), &cards, &Asked::default()), Some((0, Why::BuiltInPanel)));
    }

    /// A desktop with its monitor on the discrete card and the iGPU left on (a low-power default
    /// draws on the iGPU there, and the window can't start: the case the panel rule must not break).
    #[test]
    fn a_desktop_draws_on_the_only_card_with_a_monitor() {
        let adapters =
            [vulkan("Intel(R) Graphics (RPL-S)", INTEL, DeviceType::IntegratedGpu), vulkan("NVIDIA GeForce RTX", NVIDIA, DeviceType::DiscreteGpu)];
        let cards = [card(INTEL, 0, false), card(NVIDIA, 1, false)];
        assert_eq!(choose(&adapters, &cards, &Asked::default()), Some((1, Why::OnlyGpuWithDisplays)));
        // And on the iGPU when that's where the only monitor is.
        let cards = [card(INTEL, 2, false), card(NVIDIA, 0, false)];
        assert_eq!(choose(&adapters, &cards, &Asked::default()), Some((0, Why::OnlyGpuWithDisplays)));
    }

    /// A desktop with a monitor on each card: the discrete GPU, as before (an iGPU choice there
    /// lost its surface as soon as it was set up).
    #[test]
    fn monitors_on_two_cards_keep_the_discrete_gpu() {
        let adapters =
            [vulkan("Intel(R) Graphics (RPL-S)", INTEL, DeviceType::IntegratedGpu), vulkan("NVIDIA GeForce RTX", NVIDIA, DeviceType::DiscreteGpu)];
        let cards = [card(INTEL, 1, false), card(NVIDIA, 1, false)];
        assert_eq!(choose(&adapters, &cards, &Asked::default()), Some((1, Why::Fallback)));
    }

    /// Nothing readable (no `/sys`, or a card the drivers don't name): the discrete GPU, as before.
    #[test]
    fn without_display_facts_the_old_choice_stands() {
        assert_eq!(choose(&laptop_adapters(), &[], &Asked::default()), Some((1, Why::Fallback)));
        let unknown = [card((0x1234, 0x1, "0000:09:00.0"), 1, true)];
        assert_eq!(choose(&laptop_adapters(), &unknown, &Asked::default()), Some((1, Why::Fallback)));
        assert_eq!(choose(&[], &[card(AMD, 1, true)], &Asked::default()), None);
    }

    /// The environment variables still decide: prime-run style requests reach the NVIDIA card.
    #[test]
    fn wgpu_environment_variables_decide_when_set() {
        let cards = [card(NVIDIA, 0, false), card(AMD, 1, true)];
        let asked = |name: Option<&str>, power: Option<PowerPreference>| Asked { adapter_name: name.map(Into::into), power };
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(None, Some(PowerPreference::HighPerformance))), Some((1, Why::AskedPower)));
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(None, Some(PowerPreference::LowPower))), Some((0, Why::AskedPower)));
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(None, Some(PowerPreference::None))), Some((0, Why::AskedPower)));
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(Some("nvidia"), None)), Some((1, Why::AskedByName)));
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(Some("RadeonSI"), Some(PowerPreference::HighPerformance))), Some((2, Why::AskedByName)));
        // A name nothing has: the usual choice, not a failure.
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(Some("matrox"), None)), Some((0, Why::BuiltInPanel)));
        assert_eq!(choose(&laptop_adapters(), &cards, &asked(Some(""), None)), Some((0, Why::BuiltInPanel)));
    }

    /// Under `WGPU_BACKEND=gl` there's no PCI address or device id: the vendor alone finds the
    /// card, but only when no other card shares it.
    #[test]
    fn gl_adapters_are_matched_by_vendor_only_when_unambiguous() {
        let adapters =
            [gl("NVIDIA GeForce RTX 3050", 0x10de, DeviceType::Other), gl("AMD Radeon Graphics (radeonsi)", 0x1002, DeviceType::IntegratedGpu)];
        let cards = [card(NVIDIA, 0, false), card(AMD, 1, true)];
        assert_eq!(choose(&adapters, &cards, &Asked::default()), Some((1, Why::BuiltInPanel)));
        // Two AMD cards: the vendor can't tell them apart, so the old choice stands.
        let two_amd = [card((0x1002, 0x73df, "0000:03:00.0"), 0, false), card(AMD, 1, true)];
        let adapters = [gl("AMD Radeon RX 6700", 0x1002, DeviceType::Other), gl("AMD Radeon Graphics", 0x1002, DeviceType::IntegratedGpu)];
        assert_eq!(choose(&adapters, &two_amd, &Asked::default()), Some((1, Why::Fallback)));
    }

    /// Two adapters for the panel's card (Vulkan and GL): Vulkan, whatever the order.
    #[test]
    fn vulkan_comes_before_gl_for_the_same_card() {
        let adapters = [
            gl("AMD Radeon Graphics (radeonsi)", 0x1002, DeviceType::IntegratedGpu),
            vulkan("AMD Radeon Graphics (RADV RENOIR)", AMD, DeviceType::IntegratedGpu),
        ];
        assert_eq!(choose(&adapters, &[card(AMD, 1, true)], &Asked::default()), Some((1, Why::BuiltInPanel)));
    }

    /// A laptop with the lid closed and a monitor on the NVIDIA card: the switched-off panel isn't
    /// read as one (`drm` skips it), so the card with the monitor draws.
    #[test]
    fn a_closed_laptop_draws_on_the_card_with_the_monitor() {
        let cards = [card(NVIDIA, 1, false), card(AMD, 0, false)];
        assert_eq!(choose(&laptop_adapters(), &cards, &Asked::default()), Some((1, Why::OnlyGpuWithDisplays)));
    }

    /// Two cards of one model have the same PCI ids: only the PCI address tells them apart. Without
    /// one from the driver the display rule gives way to the old choice instead of guessing.
    #[test]
    fn identical_cards_are_told_apart_only_by_pci_address() {
        const SECOND: (u32, u32, &str) = (0x10de, 0x25a2, "0000:02:00.0");
        let cards = [card(NVIDIA, 0, false), card(SECOND, 1, false)];
        let with_slots = [vulkan("NVIDIA A", NVIDIA, DeviceType::DiscreteGpu), vulkan("NVIDIA B", SECOND, DeviceType::DiscreteGpu)];
        assert_eq!(choose(&with_slots, &cards, &Asked::default()), Some((1, Why::OnlyGpuWithDisplays)));
        let without: Vec<Candidate> = with_slots.iter().cloned().map(|c| Candidate { pci_slot: String::new(), ..c }).collect();
        assert_eq!(choose(&without, &cards, &Asked::default()), Some((0, Why::Fallback)));
    }

    /// The laptop this was measured on: Intel HD 530 behind the panel (`card2-eDP-1`), a Quadro
    /// M4000M with nothing connected, and the Quadro offered first by wgpu.
    #[test]
    fn an_intel_and_nvidia_laptop_draws_on_intel() {
        const HD530: (u32, u32, &str) = (0x8086, 0x191b, "0000:00:02.0");
        const M4000M: (u32, u32, &str) = (0x10de, 0x13f9, "0000:01:00.0");
        let adapters = [
            vulkan("Quadro M4000M", M4000M, DeviceType::DiscreteGpu),
            vulkan("Intel(R) HD Graphics 530 (SKL GT2)", HD530, DeviceType::IntegratedGpu),
        ];
        let cards = [card(M4000M, 0, false), card(HD530, 1, true)];
        assert_eq!(choose(&adapters, &cards, &Asked::default()), Some((1, Why::BuiltInPanel)));
    }
}
