//! Which graphics cards drive displays, as the system reports them (Linux: `/sys/class/drm`).
//!
//! A window drawn on one card and shown on a screen attached to another is copied across on every
//! frame, so the app draws on the card that drives the screen (see the parent module).

use std::io::Read as _;
use std::path::Path;

/// A graphics card and the displays connected to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayCard {
    /// PCI vendor id: 0x1002 AMD, 0x10de NVIDIA, 0x8086 Intel.
    pub vendor: u32,
    /// PCI device id.
    pub device: u32,
    /// PCI address in the form `0000:05:00.0`, lowercase; empty when the system doesn't give one.
    pub pci_slot: String,
    /// Connected displays that are switched on, of any kind, built-in panel included.
    pub displays: u32,
    /// A connected, switched-on built-in panel (eDP, LVDS or DSI): this card drives a laptop's
    /// own screen.
    pub built_in_panel: bool,
}

/// Connector kinds that are a computer's own screen rather than a monitor plugged into it.
const BUILT_IN_PANELS: &[&str] = &["eDP", "LVDS", "DSI"];

/// At most this many entries are read from the DRM folder.
const MAX_ENTRIES: usize = 512;

/// At most this many bytes are read from one sysfs file.
const MAX_FILE: u64 = 4096;

/// The graphics cards on this computer and what's connected to them. Empty where the system
/// doesn't say (a sandbox without `/sys`).
#[cfg(target_os = "linux")]
pub(crate) fn display_cards() -> Vec<DisplayCard> {
    display_cards_in(Path::new("/sys/class/drm"))
}

/// [`display_cards`] read from a DRM folder laid out like `/sys/class/drm`: a `cardN` entry per
/// card, with the card's PCI ids in `cardN/device/uevent`, and a `cardN-<kind>-<n>` entry per
/// connector, with `connected` or `disconnected` in its `status` and, where the kernel gives it,
/// `enabled` or `disabled` in its `enabled`. Cards without PCI ids (a firmware framebuffer) are
/// left out.
pub(crate) fn display_cards_in(drm: &Path) -> Vec<DisplayCard> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries.take(MAX_ENTRIES).filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    names.sort();
    let cards = names.iter().filter(|n| is_card(n));
    cards
        .filter_map(|card| {
            let uevent = read_small(&drm.join(card).join("device").join("uevent"))?;
            let (vendor, device) = pci_ids(&uevent)?;
            let pci_slot = uevent_value(&uevent, "PCI_SLOT_NAME").unwrap_or_default().to_ascii_lowercase();
            let prefix = format!("{card}-");
            let mut out = DisplayCard { vendor, device, pci_slot, displays: 0, built_in_panel: false };
            for connector in names.iter().filter_map(|n| n.strip_prefix(&prefix)) {
                let dir = drm.join(format!("{prefix}{connector}"));
                let status = read_small(&dir.join("status")).unwrap_or_default();
                // A panel can be connected but switched off (a closed lid with a monitor in use):
                // `enabled` then reads `disabled`, and it drives nothing. No `enabled`: as before.
                let enabled = read_small(&dir.join("enabled")).unwrap_or_default();
                if status.trim() != "connected" || enabled.trim() == "disabled" {
                    continue;
                }
                out.displays = out.displays.saturating_add(1);
                if BUILT_IN_PANELS.contains(&connector_kind(connector)) {
                    out.built_in_panel = true;
                }
            }
            Some(out)
        })
        .collect()
}

/// `card0`, `card12`: a card, not a connector (`card0-eDP-1`) or a render node.
fn is_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `eDP-1` → `eDP`, `HDMI-A-1` → `HDMI-A`: the connector's name without its number.
fn connector_kind(connector: &str) -> &str {
    match connector.rsplit_once('-') {
        Some((kind, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => kind,
        _ => connector,
    }
}

/// `PCI_ID=1002:1638` → (0x1002, 0x1638).
fn pci_ids(uevent: &str) -> Option<(u32, u32)> {
    let (vendor, device) = uevent_value(uevent, "PCI_ID")?.split_once(':')?;
    Some((u32::from_str_radix(vendor.trim(), 16).ok()?, u32::from_str_radix(device.trim(), 16).ok()?))
}

fn uevent_value<'a>(uevent: &'a str, key: &str) -> Option<&'a str> {
    uevent.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim)
}

fn read_small(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_FILE).read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A folder of its own under the system's temporary folder, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("wordcraft-drm-test-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A fake `/sys/class/drm`: cards as (name, PCI_ID, PCI_SLOT_NAME), connectors as
    /// (name, status).
    fn drm(cards: &[(&str, &str, &str)], connectors: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new();
        for (card, id, slot) in cards {
            let device = dir.path().join(card).join("device");
            fs::create_dir_all(&device).expect("card folder");
            fs::write(device.join("uevent"), format!("DRIVER=x\nPCI_CLASS=30000\nPCI_ID={id}\nPCI_SLOT_NAME={slot}\n")).expect("uevent");
        }
        for (connector, status) in connectors {
            let c = dir.path().join(connector);
            fs::create_dir_all(&c).expect("connector folder");
            fs::write(c.join("status"), format!("{status}\n")).expect("status");
        }
        fs::create_dir_all(dir.path().join("renderD128")).expect("render node");
        fs::write(dir.path().join("version"), "drm 1.1.0\n").expect("version");
        dir
    }

    /// A laptop whose AMD iGPU drives the panel, with an NVIDIA card that has no outputs.
    #[test]
    fn a_laptop_panel_is_found_on_the_integrated_gpu() {
        let d = drm(
            &[("card0", "10DE:25A2", "0000:01:00.0"), ("card1", "1002:1638", "0000:05:00.0")],
            &[("card1-DP-1", "disconnected"), ("card1-eDP-1", "connected"), ("card1-HDMI-A-1", "disconnected")],
        );
        let cards = display_cards_in(d.path());
        assert_eq!(
            cards,
            vec![
                DisplayCard { vendor: 0x10de, device: 0x25a2, pci_slot: "0000:01:00.0".into(), displays: 0, built_in_panel: false },
                DisplayCard { vendor: 0x1002, device: 0x1638, pci_slot: "0000:05:00.0".into(), displays: 1, built_in_panel: true },
            ]
        );
    }

    /// A desktop with a monitor on each card: displays on both, no built-in panel.
    #[test]
    fn monitors_on_two_cards_are_counted_on_each() {
        let d = drm(
            &[("card0", "8086:A780", "0000:00:02.0"), ("card1", "10de:2206", "0000:01:00.0")],
            &[("card0-HDMI-A-1", "connected"), ("card1-DP-1", "connected"), ("card1-DP-2", "connected"), ("card1-DP-3", "disconnected")],
        );
        let cards = display_cards_in(d.path());
        assert_eq!(cards.iter().map(|c| (c.displays, c.built_in_panel)).collect::<Vec<_>>(), vec![(1, false), (2, false)]);
    }

    /// `card10` is a different card from `card1`, and its connectors aren't `card1`'s.
    #[test]
    fn card_numbers_are_not_prefixes_of_each_other() {
        let d = drm(&[("card1", "1002:1638", "0000:05:00.0"), ("card10", "10de:25a2", "0000:01:00.0")], &[("card10-DP-1", "connected")]);
        let cards = display_cards_in(d.path());
        assert_eq!(cards.iter().map(|c| (c.vendor, c.displays)).collect::<Vec<_>>(), vec![(0x1002, 0), (0x10de, 1)]);
    }

    /// LVDS and DSI panels are built in too; HDMI-A is a monitor, whatever its number.
    #[test]
    fn built_in_panels_are_told_from_monitors_by_connector_kind() {
        assert_eq!(connector_kind("eDP-1"), "eDP");
        assert_eq!(connector_kind("HDMI-A-1"), "HDMI-A");
        assert_eq!(connector_kind("DSI-12"), "DSI");
        assert_eq!(connector_kind("Virtual"), "Virtual");
        let d = drm(&[("card0", "8086:9a49", "0000:00:02.0")], &[("card0-LVDS-1", "connected")]);
        assert!(display_cards_in(d.path()).iter().all(|c| c.built_in_panel));
    }

    /// A firmware framebuffer has no PCI ids; unreadable or odd entries are skipped, never a panic.
    #[test]
    fn what_cant_be_read_is_left_out() {
        let d = drm(&[("card0", "nonsense", ""), ("card1", "1002:", "")], &[("card0-Virtual-1", "connected")]);
        fs::create_dir_all(d.path().join("card2")).expect("card without device");
        fs::create_dir_all(d.path().join("card")).expect("bare name");
        assert!(display_cards_in(d.path()).is_empty());
        assert!(display_cards_in(&d.path().join("missing")).is_empty());
        assert_eq!(pci_ids("PCI_ID=10de:25a2"), Some((0x10de, 0x25a2)));
        assert_eq!(pci_ids("PCI_IDX=10de:25a2"), None);
    }

    /// A status of `unknown` (some virtual outputs) isn't a connected display.
    #[test]
    fn only_connected_counts() {
        let d = drm(&[("card0", "1af4:1050", "0000:00:01.0")], &[("card0-Virtual-1", "unknown")]);
        assert_eq!(display_cards_in(d.path()).first().map(|c| c.displays), Some(0));
    }

    /// A laptop with its lid closed and a monitor on the discrete card: the panel still reads
    /// `connected` but is switched off, so it neither counts nor marks its card as the panel's.
    #[test]
    fn a_switched_off_panel_does_not_count() {
        let d = drm(
            &[("card0", "10de:25a2", "0000:01:00.0"), ("card1", "1002:1638", "0000:05:00.0")],
            &[("card0-HDMI-A-1", "connected"), ("card1-eDP-1", "connected")],
        );
        fs::write(d.path().join("card0-HDMI-A-1").join("enabled"), "enabled\n").expect("enabled");
        fs::write(d.path().join("card1-eDP-1").join("enabled"), "disabled\n").expect("enabled");
        let cards = display_cards_in(d.path());
        assert_eq!(cards.iter().map(|c| (c.vendor, c.displays, c.built_in_panel)).collect::<Vec<_>>(), vec![(0x10de, 1, false), (0x1002, 0, false)]);
    }
}
