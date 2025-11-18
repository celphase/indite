use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};
use wgpu::{
    Device, Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureUses, TextureView, TextureViewDescriptor, TextureViewDimension,
};
use wgpu_hal::api::Vulkan;

pub struct SwapchainDescriptor {
    pub width: u32,
    pub height: u32,
    pub view_count: u32,
}

pub type SwapchainHandle = Arc<Mutex<openxr::Swapchain<openxr::Vulkan>>>;

/// Creates a swapchain for the OpenXR session.
///
/// The swapchain itself is returned with a mutex guard, because the WGPU textures that reference it
/// need to keep it alive.
pub fn create_swapchain(
    device: &Device,
    xr_session: &openxr::Session<openxr::Vulkan>,
    desc: &SwapchainDescriptor,
) -> (SwapchainHandle, Vec<(Texture, TextureView)>) {
    // Create a swapchain for the viewpoints! A swapchain is a set of texture buffers
    // used for displaying to screen, typically this is a backbuffer and a front buffer,
    // one for rendering data to, and one for displaying on-screen.
    let swapchain_resolution = vk::Extent2D {
        width: desc.width,
        height: desc.height,
    };
    let swapchain_info = openxr::SwapchainCreateInfo {
        create_flags: openxr::SwapchainCreateFlags::EMPTY,
        usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT
            | openxr::SwapchainUsageFlags::SAMPLED,
        format: vk::Format::R8G8B8A8_SRGB.as_raw() as _,
        // The Vulkan graphics pipeline we create is not set up for multisampling,
        // so we hardcode this to 1. If we used a proper multisampling setup, we
        // could set this to `views[0].recommended_swapchain_sample_count`.
        sample_count: 1,
        width: swapchain_resolution.width,
        height: swapchain_resolution.height,
        face_count: 1,
        array_size: desc.view_count,
        mip_count: 1,
    };
    let xr_swapchain = xr_session.create_swapchain(&swapchain_info).unwrap();
    let xr_swapchain = Arc::new(Mutex::new(xr_swapchain));

    let swapchain_images = xr_swapchain.lock().unwrap().enumerate_images().unwrap();
    let swapchain_textures: Vec<_> = swapchain_images
        .into_iter()
        .map(|color_image| {
            let texture = unsafe {
                create_swapchain_texture(
                    color_image,
                    device,
                    swapchain_resolution,
                    xr_swapchain.clone(),
                )
            };
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                array_layer_count: Some(desc.view_count),
                ..Default::default()
            });
            (texture, view)
        })
        .collect();

    (xr_swapchain, swapchain_textures)
}

unsafe fn create_swapchain_texture(
    color_image: u64,
    device: &Device,
    resolution: vk::Extent2D,
    xr_swapchain: SwapchainHandle,
) -> Texture {
    let color_image = vk::Image::from_raw(color_image);

    let hal_device = unsafe { device.as_hal::<Vulkan>().unwrap() };

    let hal_texture_desc = wgpu_hal::TextureDescriptor {
        label: Some("openxr swapchain"),
        size: Extent3d {
            width: resolution.width,
            height: resolution.height,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUses::COLOR_TARGET | TextureUses::COPY_DST,
        memory_flags: wgpu_hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };

    // This callback is very important! If not specified, WGPU will take ownership of the image,
    // which is not correct as it's owned by the OpenXR runtime.
    // This also keeps the underlying OpenXR swapchain alive.
    let drop_callback = move || {
        println!("swapchain texture wgpu drop called");
        drop(xr_swapchain);
    };

    let wgpu_hal_texture = unsafe {
        hal_device.texture_from_raw(
            color_image,
            &hal_texture_desc,
            Some(Box::new(drop_callback)),
        )
    };

    let texture_desc = TextureDescriptor {
        label: Some("openxr swapchain"),
        size: Extent3d {
            width: resolution.width,
            height: resolution.height,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
        view_formats: &[],
    };
    unsafe { device.create_texture_from_hal::<Vulkan>(wgpu_hal_texture, &texture_desc) }
}
