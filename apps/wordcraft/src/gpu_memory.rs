//! GPU memory hints for the wgpu device: small allocation blocks on Windows and macOS.

use eframe::egui_wgpu::WgpuSetupCreateNew;
use eframe::wgpu::{DeviceDescriptor, MemoryHints};

/// Switches a device descriptor to small GPU memory blocks; its other fields stay.
pub fn with_small_memory_blocks(descriptor: DeviceDescriptor<'static>) -> DeviceDescriptor<'static> {
    DeviceDescriptor { memory_hints: MemoryHints::MemoryUsage, ..descriptor }
}

/// Wraps the device descriptor eframe builds, so its limits stay as they are.
/// Call after the craft GPU setup, which may replace the descriptor.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub fn configure_memory_hints(create: &mut WgpuSetupCreateNew) {
    let describe = std::sync::Arc::clone(&create.device_descriptor);
    create.device_descriptor = std::sync::Arc::new(move |adapter| with_small_memory_blocks(describe(adapter)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_allocates_gpu_memory_in_small_blocks() {
        let descriptor = with_small_memory_blocks(DeviceDescriptor { label: Some("kept"), ..Default::default() });
        assert!(matches!(descriptor.memory_hints, MemoryHints::MemoryUsage));
        assert_eq!(descriptor.label, Some("kept"));
    }
}
