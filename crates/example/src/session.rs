use indite::{SwapchainDescriptor, SwapchainHandle};
use wgpu::{Texture, TextureFormat, TextureUsages, TextureView};

use crate::{rendering::RenderContext, VIEW_COUNT, VIEW_TYPE};

pub struct SessionBundle {
    pub session: openxr::Session<openxr::Vulkan>,
    pub frame_wait: openxr::FrameWaiter,
    pub frame_stream: openxr::FrameStream<openxr::Vulkan>,

    pub swapchain_desc: SwapchainDescriptor,
    pub swapchain_handle: SwapchainHandle,
    pub swapchain_textures: Vec<(Texture, TextureView)>,
    pub stage: openxr::Space,

    pub multisampled_framebuffer: TextureView,
}

pub fn create_session(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    render_context: &RenderContext,
) -> SessionBundle {
    // A session represents this application's desire to display things! This is where we hook
    // up our graphics API. This does not start the session; for that, you'll need a call to
    // Session::begin, which we do in 'main_loop below.
    let (xr_session, frame_wait, frame_stream) = indite::create_session(
        xr_instance,
        xr_system,
        &render_context.instance,
        &render_context.device,
    )
    .unwrap();

    // Find all the viewpoints for the view type we're using.
    let xr_view_configs = xr_instance
        .enumerate_view_configuration_views(xr_system, VIEW_TYPE)
        .unwrap();
    assert_eq!(xr_view_configs.len(), VIEW_COUNT as usize);

    // We're using plain multiview rendering right now, no foviated features, so the views should be
    // equal.
    assert_eq!(xr_view_configs[0], xr_view_configs[1]);

    // Create the swapchain for the session
    let swapchain_desc = indite::SwapchainDescriptor {
        width: xr_view_configs[0].recommended_image_rect_width,
        height: xr_view_configs[0].recommended_image_rect_height,
        view_count: VIEW_COUNT,
    };
    let (swapchain_handle, swapchain_textures) =
        indite::create_swapchain(&render_context.device, &xr_session, &swapchain_desc).unwrap();

    // OpenXR uses a couple different types of reference frames for positioning content; we need
    // to choose one for displaying our content! STAGE would be relative to the center of your
    // guardian system's bounds, and LOCAL would be relative to your device's starting location.
    let stage = xr_session
        .create_reference_space(openxr::ReferenceSpaceType::STAGE, openxr::Posef::IDENTITY)
        .unwrap();

    let multisampled_framebuffer =
        create_multisampled_framebuffer(&render_context.device, &swapchain_desc);

    SessionBundle {
        session: xr_session,
        frame_wait,
        frame_stream,

        swapchain_desc,
        swapchain_handle,
        swapchain_textures,
        stage,

        multisampled_framebuffer,
    }
}

fn create_multisampled_framebuffer(
    device: &wgpu::Device,
    swapchain_desc: &SwapchainDescriptor,
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: swapchain_desc.width,
        height: swapchain_desc.height,
        depth_or_array_layers: swapchain_desc.view_count,
    };
    let desc = &wgpu::TextureDescriptor {
        label: Some("multisampled-framebuffer"),
        size,
        mip_level_count: 1,
        sample_count: 4,
        dimension: wgpu::TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TRANSIENT,
        view_formats: &[],
    };

    device
        .create_texture(desc)
        .create_view(&wgpu::TextureViewDescriptor::default())
}
