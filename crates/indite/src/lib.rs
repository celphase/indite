mod context;
mod debug_utils;
mod swapchain;

use ash::vk::Handle;
use wgpu::{Device, Instance, hal::api::Vulkan};

pub use self::{
    context::{create_device, create_instance},
    debug_utils::DebugUtils,
    swapchain::{SwapchainDescriptor, SwapchainHandle, create_swapchain},
};

pub fn create_session(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    instance: &Instance,
    device: &Device,
) -> (
    openxr::Session<openxr::Vulkan>,
    openxr::FrameWaiter,
    openxr::FrameStream<openxr::Vulkan>,
) {
    let hal_instance = unsafe { instance.as_hal::<Vulkan>().unwrap() };
    let vk_instance = hal_instance.shared_instance().raw_instance();
    let hal_device = unsafe { device.as_hal::<Vulkan>().unwrap() };

    let create_info = openxr::vulkan::SessionCreateInfo {
        instance: vk_instance.handle().as_raw() as _,
        physical_device: hal_device.raw_physical_device().as_raw() as _,
        device: hal_device.raw_device().handle().as_raw() as _,
        queue_family_index: hal_device.queue_family_index(),
        queue_index: 0,
    };

    // Keep dependencies alive
    let guard = Box::new((instance.clone(), device.clone()));

    unsafe {
        xr_instance
            .create_session_with_guard::<openxr::Vulkan>(xr_system, &create_info, guard)
            .unwrap()
    }
}
