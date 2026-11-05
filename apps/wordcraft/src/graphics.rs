//! Graphics instance configuration applied before wgpu can initialize any driver: Windows
//! backends and the DirectX 12 shader compiler, and the Intel Mac indirect-validation skip.
// On macOS only the indirect-validation items are used outside tests. Elsewhere it is the other
// way round, so those two carry their own `allow`.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use eframe::{
    NativeOptions,
    egui_wgpu::WgpuSetup,
    wgpu::{Backends, DeviceType, Dx12Compiler, InstanceFlags},
};

/// wgpu compiles its indirect-call validation compute shaders inside `request_device`. On old
/// Intel Macs (HD Graphics 5000 on Big Sur, #2652) Metal's Intel
/// compiler aborts on them, the device is lost and eframe exits before the window opens. WordCraft
/// issues no indirect draws or dispatches, so Intel Macs skip them; every other GPU keeps wgpu's
/// default. Same fix as PhotoCraft's storytold/PhotoCraft#3171.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn skips_indirect_validation(macos: bool, intel: bool) -> bool {
    macos && intel
}

/// Leaves wgpu's indirect-call validation out of the instance eframe creates when `skip`.
/// `WGPU_VALIDATION_INDIRECT_CALL=1` still turns it back on.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn configure_indirect_validation(options: &mut NativeOptions, skip: bool) {
    if !skip {
        return;
    }
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.flags = create.instance_descriptor.flags.difference(InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
    }
}

/// What the start-up probe learned about one DirectX 12 adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dx12Adapter {
    /// `Cpu` is WARP (Microsoft's software rasteriser): it works, but every frame is drawn on the
    /// processor, which makes the app lag on older machines (#316).
    pub device_type: DeviceType,
    /// Whether the adapter offers the limits eframe will ask for when it creates the device.
    pub meets_limits: bool,
}

/// The backends to initialize on Windows.
///
/// An explicit `backend_override` (normally `Backends::from_env()`, i.e. `WGPU_BACKEND`) always
/// wins. Otherwise DirectX 12 only, as long as `dx12_graphics` is set and `dx12_adapters` (what a
/// DirectX 12-only probe found) contains a usable hardware GPU. Without one, OpenGL is the way to
/// the GPU: graphics chips whose driver has no DirectX 12 support (Intel 4th generation on Windows
/// 11, #316) still have an OpenGL driver, and GL can't reach the AMD driver crash described in
/// [`configure_backends`], because an AMD GPU with a driver shows up as a DirectX 12 adapter and
/// keeps us on DirectX 12. When DirectX 12 offers only WARP, it stays enabled next to GL as a last
/// resort for machines without an OpenGL driver either (virtual machines): wgpu prefers any GPU
/// over a software adapter, so it's only picked when GL has nothing better.
///
/// `dx12_graphics` is false on 32-bit Windows. wgpu creates the graphics pipeline through
/// `ID3D12Device2::CreatePipelineState`, and that stream is aligned to 8 bytes. The 32-bit runtime
/// expects pointer alignment (4), so creation returns `E_INVALIDARG` ("The parameter is incorrect")
/// and the process exits before the first frame. Compute pipelines use the older API and succeed,
/// which is why startup gets as far as the egui pipeline and then quits. OpenGL does not use that
/// stream.
pub fn choose_backends(backend_override: Option<Backends>, dx12_adapters: &[Dx12Adapter], dx12_graphics: bool) -> Backends {
    if let Some(backends) = backend_override {
        return backends;
    }
    if !dx12_graphics {
        return Backends::GL;
    }
    let hardware = |a: &&Dx12Adapter| a.device_type != DeviceType::Cpu;
    if dx12_adapters.iter().filter(hardware).any(|a| a.meets_limits) {
        Backends::DX12
    } else if !dx12_adapters.is_empty() && !dx12_adapters.iter().any(|a| hardware(&a)) {
        Backends::DX12 | Backends::GL
    } else {
        // No DirectX 12 adapter at all, or only GPUs that can't run the app: GL alone, so a
        // DirectX 12 GPU that would fail device creation isn't picked over the OpenGL one.
        Backends::GL
    }
}

/// Restricts the wgpu instance eframe creates to `backends`.
pub fn configure_backends(options: &mut NativeOptions, backends: Backends) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        // eframe's default includes GL. Creating that backend can crash inside AMD's
        // atio6axx.dll before adapter selection or any Rust error handling runs, so the
        // window never appears. Choosing an adapter afterwards is too late: only leaving GL
        // out of the instance avoids it. This matches the verified WGPU_BACKEND=dx12
        // workaround without mutating process environment variables.
        create.instance_descriptor.backends = backends;
    }
}

/// The DirectX 12 shader compiler: FXC (`d3dcompiler_47.dll`, part of Windows) unless `env`
/// (normally `WGPU_DX12_COMPILER`, wgpu's own variable: `fxc`, `dxc`, `auto`…) names another.
///
/// #603: wgpu's default (`Auto`) uses DXC, loading whichever `dxcompiler.dll` the DLL search path
/// finds first. WordCraft ships none, so that copy belongs to another program. Without a matching
/// `dxil.dll` beside it, DXC warns that the DXIL is unsigned; wgpu takes the warning for a compile
/// error, its own indirect-validation pipeline fails, the device is lost and eframe exits before
/// the first frame. FXC is always there and compiles everything egui's shaders need.
pub fn dx12_compiler(env: Option<&str>) -> Dx12Compiler {
    let Some(value) = env else { return Dx12Compiler::Fxc };
    value.parse().unwrap_or_else(|e| {
        log::warn!("graphics: WGPU_DX12_COMPILER={value:?} ignored ({e}), using FXC");
        Dx12Compiler::Fxc
    })
}

/// Makes the wgpu instance eframe creates compile DirectX 12 shaders with `compiler`.
pub fn configure_dx12_compiler(options: &mut NativeOptions, compiler: Dx12Compiler) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backend_options.dx12.shader_compiler = compiler;
    }
}

/// Chooses the backends ([`choose_backends`]) and applies them to `options`. Without a
/// `backend_override` this first probes DirectX 12 with a short-lived instance of its own, which
/// is dropped before eframe creates the real one.
#[cfg(target_os = "windows")]
pub fn configure(options: &mut NativeOptions, backend_override: Option<Backends>) {
    // First, so the probe's instance doesn't load a stray `dxcompiler.dll` either.
    let compiler = dx12_compiler(std::env::var("WGPU_DX12_COMPILER").ok().as_deref());
    log::info!("graphics: DirectX 12 shader compiler {compiler:?}");
    configure_dx12_compiler(options, compiler);
    let backends = match backend_override {
        Some(backends) => {
            log::info!("graphics: WGPU_BACKEND override, backends {backends:?}");
            backends
        }
        None => {
            // 32-bit: skip the DirectX 12 probe. A capable GPU would be selected, then
            // CreatePipelineState fails and the app exits with no window.
            let dx12_graphics = cfg!(not(target_arch = "x86"));
            let adapters = if dx12_graphics { probe_dx12(options) } else { Vec::new() };
            let backends = choose_backends(None, &adapters, dx12_graphics);
            if dx12_graphics {
                log::info!("graphics: {} DirectX 12 adapter(s) {adapters:?}, backends {backends:?}", adapters.len());
            } else {
                log::info!("graphics: 32-bit Windows uses OpenGL; DirectX 12 graphics pipeline creation fails");
            }
            backends
        }
    };
    configure_backends(options, backends);
}

/// Lists the DirectX 12 adapters, checking each against the limits eframe's device descriptor
/// will require. Empty when this build has no DirectX 12 backend.
#[cfg(target_os = "windows")]
fn probe_dx12(options: &NativeOptions) -> Vec<Dx12Adapter> {
    use eframe::wgpu::{Instance, InstanceDescriptor};
    // `Instance::new` panics when the build has no backend for the platform.
    if !Instance::enabled_backend_features().contains(Backends::DX12) {
        return Vec::new();
    }
    let WgpuSetup::CreateNew(create) = &options.wgpu_options.wgpu_setup else {
        return Vec::new();
    };
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::DX12,
        backend_options: create.instance_descriptor.backend_options.clone(),
        ..InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.enumerate_adapters(Backends::DX12))
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            let required = (create.device_descriptor)(adapter).required_limits;
            let meets_limits = required.check_limits(&adapter.limits());
            log::info!("graphics: DirectX 12 adapter {:?} ({:?}, driver {:?}), limits ok: {meets_limits}", info.name, info.device_type, info.driver);
            Dx12Adapter { device_type: info.device_type, meets_limits }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_macs_skip_the_indirect_validation_shaders() {
        // #2652: Metal's Intel compiler aborted on them, losing the device before the window opened.
        assert!(skips_indirect_validation(true, true));
        // Apple silicon, Windows and Linux keep wgpu's validation.
        for (macos, intel) in [(true, false), (false, true), (false, false)] {
            assert!(!skips_indirect_validation(macos, intel), "macos={macos} intel={intel}");
        }
        let mut options = NativeOptions::default();
        let WgpuSetup::CreateNew(before) = &options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        let before = before.instance_descriptor.flags;
        configure_indirect_validation(&mut options, false);
        let WgpuSetup::CreateNew(create) = &options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert_eq!(create.instance_descriptor.flags, before, "flags untouched when not skipping");
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
            configure_indirect_validation(&mut options, true);
            let WgpuSetup::CreateNew(create) = &options.wgpu_options.wgpu_setup else {
                panic!("default native options must create a wgpu instance");
            };
            assert!(!create.instance_descriptor.flags.contains(InstanceFlags::VALIDATION_INDIRECT_CALL));
        }
    }

    fn configured_backends(backends: Backends) -> Backends {
        let mut options = NativeOptions::default();
        configure_backends(&mut options, backends);
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        create.instance_descriptor.backends
    }

    const fn adapter(device_type: DeviceType, meets_limits: bool) -> Dx12Adapter {
        Dx12Adapter { device_type, meets_limits }
    }

    #[test]
    fn windows_default_initializes_only_dx12() {
        // eframe's own default (PRIMARY | GL) would initialize OpenGL as well.
        let WgpuSetup::CreateNew(eframe_default) = NativeOptions::default().wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(eframe_default.instance_descriptor.backends.contains(Backends::GL));
        let gpu = [adapter(DeviceType::IntegratedGpu, true)];
        assert_eq!(configured_backends(choose_backends(None, &gpu, true)), Backends::DX12);
    }

    #[test]
    fn explicit_backend_override_is_preserved() {
        let warp_only = [adapter(DeviceType::Cpu, true)];
        for backends in [Backends::VULKAN, Backends::GL, Backends::DX12, Backends::VULKAN | Backends::DX12, Backends::empty()] {
            // Whatever the probe would have found, the override wins (the probe doesn't run then).
            assert_eq!(choose_backends(Some(backends), &[], true), backends);
            assert_eq!(choose_backends(Some(backends), &warp_only, true), backends);
            assert_eq!(configured_backends(backends), backends);
        }
    }

    /// #603: DirectX 12 shaders compile with FXC, never a `dxcompiler.dll` found on the search
    /// path, unless `WGPU_DX12_COMPILER` asks for it.
    #[test]
    fn dx12_compiles_with_fxc_unless_asked() {
        assert!(matches!(dx12_compiler(None), Dx12Compiler::Fxc));
        assert!(matches!(dx12_compiler(Some("bogus")), Dx12Compiler::Fxc));
        assert!(matches!(dx12_compiler(Some("FXC")), Dx12Compiler::Fxc));
        assert!(matches!(dx12_compiler(Some("dxc")), Dx12Compiler::DynamicDxc { .. }));
        assert!(matches!(dx12_compiler(Some("auto")), Dx12Compiler::Auto));
        let mut options = NativeOptions::default();
        configure_dx12_compiler(&mut options, dx12_compiler(None));
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(matches!(create.instance_descriptor.backend_options.dx12.shader_compiler, Dx12Compiler::Fxc));
    }

    #[test]
    fn gl_when_dx12_has_no_usable_gpu() {
        // #316: no DirectX 12 driver, so DirectX 12 offers only WARP (or nothing) and lags.
        let warp_only = [adapter(DeviceType::Cpu, true)];
        assert_eq!(choose_backends(None, &warp_only, true), Backends::GL | Backends::DX12);
        assert_eq!(choose_backends(None, &[], true), Backends::GL);
        // A GPU that can't create eframe's device: GL alone, so it isn't picked over the GL one.
        let weak_gpu = [adapter(DeviceType::IntegratedGpu, false), adapter(DeviceType::Cpu, true)];
        assert_eq!(choose_backends(None, &weak_gpu, true), Backends::GL);
        // Any usable GPU keeps DirectX 12 only, next to WARP or a weaker GPU.
        let mixed = [adapter(DeviceType::Cpu, true), adapter(DeviceType::IntegratedGpu, false), adapter(DeviceType::DiscreteGpu, true)];
        assert_eq!(choose_backends(None, &mixed, true), Backends::DX12);
    }

    /// 32-bit Windows: a usable DirectX 12 GPU still starts on OpenGL. `WGPU_BACKEND` still wins.
    #[test]
    fn x86_uses_opengl_even_with_a_usable_gpu() {
        let gpu = [adapter(DeviceType::IntegratedGpu, true)];
        assert_eq!(choose_backends(None, &gpu, false), Backends::GL);
        assert_eq!(choose_backends(Some(Backends::DX12), &gpu, false), Backends::DX12);
        assert_eq!(choose_backends(Some(Backends::VULKAN), &[], false), Backends::VULKAN);
        assert_eq!(configured_backends(choose_backends(None, &gpu, false)), Backends::GL);
    }
}
