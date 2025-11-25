use std::sync::{Arc, Mutex};

use anyhow::{Context, Error};
use ash::vk::{self, Handle};
use wgpu::{
    Device, Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureUses, TextureView, TextureViewDescriptor, TextureViewDimension,
    hal::{Api, api::Vulkan},
};

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
) -> Result<(SwapchainHandle, Vec<(Texture, TextureView)>), Error> {
    let swapchain_resolution = vk::Extent2D {
        width: desc.width,
        height: desc.height,
    };
    let swapchain_info = openxr::SwapchainCreateInfo {
        create_flags: openxr::SwapchainCreateFlags::EMPTY,
        usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT
            | openxr::SwapchainUsageFlags::SAMPLED,
        format: vk::Format::R8G8B8A8_SRGB.as_raw() as _,
        sample_count: 1,
        width: swapchain_resolution.width,
        height: swapchain_resolution.height,
        face_count: 1,
        array_size: desc.view_count,
        mip_count: 1,
    };
    let xr_swapchain = xr_session.create_swapchain(&swapchain_info)?;
    let xr_swapchain_handle = Arc::new(Mutex::new(xr_swapchain));

    let swapchain_textures = create_swapchain_textures(device, desc, &xr_swapchain_handle)?;

    Ok((xr_swapchain_handle, swapchain_textures))
}

fn create_swapchain_textures(
    device: &Device,
    desc: &SwapchainDescriptor,
    xr_swapchain_handle: &SwapchainHandle,
) -> Result<Vec<(Texture, TextureView)>, Error> {
    let hal_device = unsafe {
        device
            .as_hal::<Vulkan>()
            .context("wgpu device backend not vulkan")?
    };

    let xr_swapchain = xr_swapchain_handle
        .lock()
        .ok()
        .context("failed to lock swapchain")?;
    let swapchain_images = xr_swapchain.enumerate_images().unwrap();

    let swapchain_textures: Vec<_> = swapchain_images
        .into_iter()
        .map(|color_image| {
            let texture = unsafe {
                create_swapchain_texture(
                    device,
                    &hal_device,
                    desc,
                    xr_swapchain_handle.clone(),
                    color_image,
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

    Ok(swapchain_textures)
}

/// # Safety
/// - `color_image` must be valid for the information in `desc`.
/// - `color_image` lifetime is not managed by the returned `Texture`.
/// - `color_image` must be valid for as long as `xr_swapchain` is valid.
unsafe fn create_swapchain_texture(
    device: &Device,
    hal_device: &<Vulkan as Api>::Device,
    desc: &SwapchainDescriptor,
    xr_swapchain_handle: SwapchainHandle,
    color_image: u64,
) -> Texture {
    let color_image = vk::Image::from_raw(color_image);

    let hal_texture_desc = wgpu::hal::TextureDescriptor {
        label: Some("openxr swapchain texture"),
        size: Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: desc.view_count,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUses::COLOR_TARGET | TextureUses::COPY_DST,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };

    // This callback is very important! If not specified, WGPU will take ownership of the image,
    // which is not correct as it's owned by the OpenXR runtime.
    // This also keeps the underlying OpenXR swapchain alive.
    let drop_callback = move || {
        println!("swapchain texture wgpu drop called");
        drop(xr_swapchain_handle);
    };

    let wgpu_hal_texture = unsafe {
        hal_device.texture_from_raw(
            color_image,
            &hal_texture_desc,
            Some(Box::new(drop_callback)),
        )
    };

    let texture_desc = TextureDescriptor {
        label: Some("openxr swapchain texture"),
        size: Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: desc.view_count,
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
