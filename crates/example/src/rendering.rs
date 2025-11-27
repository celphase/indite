use std::{borrow::Cow, num::NonZero};

use glam::Mat4;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferDescriptor, BufferSize,
    BufferUsages, Color, CommandBuffer, CommandEncoderDescriptor, Device, FragmentState, Instance,
    LoadOp, Operations, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, TextureFormat, TextureView, VertexState,
};

use crate::{
    actions::{self, ActionSetBundle},
    math,
    session::SessionBundle,
    VIEW_TYPE,
};

pub struct RenderContext {
    pub instance: Instance,
    pub device: Device,
    pub queue: Queue,

    pub uniform_layout: BindGroupLayout,
    pub render_pipeline: RenderPipeline,
}

impl RenderContext {
    pub fn new(xr_instance: &openxr::Instance, xr_system: openxr::SystemId) -> Self {
        let instance = indite::create_instance(xr_instance, xr_system).unwrap();
        let (device, queue) = indite::create_device(xr_instance, xr_system, &instance).unwrap();

        // Create WPGU render pipeline
        let uniform_layout = create_uniform_layout(&device);
        let render_pipeline = create_render_pipeline(&device, &uniform_layout);

        Self {
            instance,
            device,
            queue,

            uniform_layout,
            render_pipeline,
        }
    }
}

fn create_uniform_layout(device: &Device) -> BindGroupLayout {
    let size = std::mem::size_of::<Mat4>() * 2;
    let transforms = BindGroupLayoutEntry {
        binding: 0,
        visibility: ShaderStages::VERTEX,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: BufferSize::new(size as u64),
        },
        count: None,
    };
    let desc = BindGroupLayoutDescriptor {
        label: Some("uniform-layout"),
        entries: &[transforms],
    };
    device.create_bind_group_layout(&desc)
}

fn create_uniform_bind_group(device: &Device, layout: &BindGroupLayout) -> (Buffer, BindGroup) {
    let desc = BufferDescriptor {
        label: Some("uniform-buffer"),
        size: (std::mem::size_of::<Mat4>() * 2) as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: true,
    };
    let buffer = device.create_buffer(&desc);

    let entry = BindGroupEntry {
        binding: 0,
        resource: buffer.as_entire_binding(),
    };

    let desc = BindGroupDescriptor {
        label: Some("uniform-bind-group"),
        layout,
        entries: &[entry],
    };
    let bind_group = device.create_bind_group(&desc);

    (buffer, bind_group)
}

fn create_render_pipeline(
    wgpu_device: &Device,
    uniform_layout: &BindGroupLayout,
) -> RenderPipeline {
    let shader = wgpu_device.create_shader_module(ShaderModuleDescriptor {
        label: None,
        source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
    });

    let pipeline_layout = wgpu_device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[uniform_layout],
        push_constant_ranges: &[],
    });

    wgpu_device.create_render_pipeline(&RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(TextureFormat::Rgba8UnormSrgb.into())],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 4,
            ..Default::default()
        },
        multiview_mask: NonZero::new(0b11),
        cache: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    environment_blend_mode: openxr::EnvironmentBlendMode,
    render_context: &RenderContext,
    session_bundle: &mut SessionBundle,
    action_set_bundle: &ActionSetBundle,
) {
    // Block until the previous frame is finished displaying, and is ready for another one.
    // Also returns a prediction of when the next frame will be displayed, for use with
    // predicting locations of controllers, viewpoints, etc.
    let xr_frame_state = session_bundle.frame_wait.wait().unwrap();

    // Must be called before any rendering is done!
    session_bundle.frame_stream.begin().unwrap();

    if !xr_frame_state.should_render {
        session_bundle
            .frame_stream
            .end(
                xr_frame_state.predicted_display_time,
                environment_blend_mode,
                &[],
            )
            .unwrap();
        return;
    }

    // We need to ask which swapchain image to use for rendering! Which one will we get?
    // Who knows! It's up to the runtime to decide.
    let image_index = session_bundle
        .swapchain_handle
        .lock()
        .unwrap()
        .acquire_image()
        .unwrap();

    // Get the view for this frame
    let (_, view) = &session_bundle.swapchain_textures[image_index as usize];

    // Record the command buffer
    let (uniform_buffer, uniform_bind_group) =
        create_uniform_bind_group(&render_context.device, &render_context.uniform_layout);
    let command_buffer = record_command_buffer(
        &render_context.device,
        &render_context.render_pipeline,
        &session_bundle.multisampled_framebuffer,
        view,
        &uniform_bind_group,
    );

    actions::read_actions(
        &session_bundle.session,
        action_set_bundle,
        &session_bundle.stage,
        &xr_frame_state,
    );

    // Fetch the view transforms. To minimize latency, we intentionally do this *after*
    // recording commands to render the scene, i.e. at the last possible moment before
    // rendering begins in earnest on the GPU. Uniforms dependent on this data can be sent
    // to the GPU just-in-time by writing them to per-frame host-visible memory which the
    // GPU will only read once the command buffer is submitted.
    let (_, xr_views) = session_bundle
        .session
        .locate_views(
            VIEW_TYPE,
            xr_frame_state.predicted_display_time,
            &session_bundle.stage,
        )
        .unwrap();

    // Update bind group buffer with the eyes' matrices, as late as possible
    write_uniform_buffer(&uniform_buffer, &xr_views);

    // Wait until the image is available to render to before beginning work on the GPU. The
    // compositor could still be reading from it.
    let mut xr_swapchain = session_bundle.swapchain_handle.lock().unwrap();
    xr_swapchain.wait_image(openxr::Duration::INFINITE).unwrap();

    // Submit the previously prepared command buffer
    render_context.queue.submit(Some(command_buffer));

    xr_swapchain.release_image().unwrap();
    end_frame(
        environment_blend_mode,
        &mut session_bundle.frame_stream,
        &session_bundle.swapchain_desc,
        &xr_swapchain,
        &session_bundle.stage,
        &xr_views,
        &xr_frame_state,
    );
}

fn record_command_buffer(
    device: &Device,
    render_pipeline: &RenderPipeline,
    multisampled_framebuffer: &TextureView,
    view: &TextureView,
    uniform_bind_group: &BindGroup,
) -> CommandBuffer {
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });

    {
        let attachment = RenderPassColorAttachment {
            view: multisampled_framebuffer,
            depth_slice: None,
            resolve_target: Some(view),
            ops: Operations {
                load: LoadOp::Clear(Color::GREEN),
                store: StoreOp::Discard,
            },
        };
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: NonZero::new(0b11),
        });

        render_pass.set_pipeline(render_pipeline);
        render_pass.set_bind_group(0, uniform_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    encoder.finish()
}

fn write_uniform_buffer(buffer: &Buffer, xr_views: &[openxr::View]) {
    let transform_0 = math::matrix_from_view(&xr_views[0]);
    let transform_1 = math::matrix_from_view(&xr_views[1]);
    let transforms = [transform_0, transform_1];

    let contents = bytemuck::bytes_of(&transforms);
    let size = contents.len();

    buffer.slice(..).get_mapped_range_mut()[..size].copy_from_slice(contents);
    buffer.unmap();
}

fn end_frame(
    environment_blend_mode: openxr::EnvironmentBlendMode,
    frame_stream: &mut openxr::FrameStream<openxr::Vulkan>,
    swapchain_desc: &indite::SwapchainDescriptor,
    xr_swapchain: &openxr::Swapchain<openxr::Vulkan>,
    xr_stage: &openxr::Space,
    xr_views: &[openxr::View],
    xr_frame_state: &openxr::FrameState,
) {
    // Tell OpenXR what to present for this frame
    let rect = openxr::Rect2Di {
        offset: openxr::Offset2Di { x: 0, y: 0 },
        extent: openxr::Extent2Di {
            width: swapchain_desc.width as _,
            height: swapchain_desc.height as _,
        },
    };
    let views = [
        openxr::CompositionLayerProjectionView::new()
            .pose(xr_views[0].pose)
            .fov(xr_views[0].fov)
            .sub_image(
                openxr::SwapchainSubImage::new()
                    .swapchain(xr_swapchain)
                    .image_array_index(0)
                    .image_rect(rect),
            ),
        openxr::CompositionLayerProjectionView::new()
            .pose(xr_views[1].pose)
            .fov(xr_views[1].fov)
            .sub_image(
                openxr::SwapchainSubImage::new()
                    .swapchain(xr_swapchain)
                    .image_array_index(1)
                    .image_rect(rect),
            ),
    ];
    let layer = openxr::CompositionLayerProjection::new()
        .space(xr_stage)
        .views(&views);
    frame_stream
        .end(
            xr_frame_state.predicted_display_time,
            environment_blend_mode,
            &[&layer],
        )
        .unwrap();
}
