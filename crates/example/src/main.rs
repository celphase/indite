use std::{
    borrow::Cow,
    num::NonZero,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use glam::{Affine3A, Mat4, Quat, Vec3};
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BufferBindingType, BufferSize, BufferUsages, Color,
    CommandBuffer, CommandEncoderDescriptor, Device, FragmentState, Instance, LoadOp,
    MultisampleState, Operations, PipelineLayoutDescriptor, PrimitiveState, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TextureFormat, TextureView,
    VertexState,
};

const VIEW_TYPE: openxr::ViewConfigurationType = openxr::ViewConfigurationType::PRIMARY_STEREO;

const VIEW_COUNT: u32 = 2;

pub fn main() {
    let xr_entry = openxr::Entry::linked();

    // OpenXR will fail to initialize if we ask for an extension that OpenXR can't provide! So we
    // need to check all our extensions before initializing OpenXR with them. Note that even if the
    // extension is present, it's still possible you may not be able to use it. For example: the
    // hand tracking extension may be present, but the hand sensor might not be plugged in or turned
    // on. There are often additional checks that should be made before using certain features!
    let available_extensions = xr_entry.enumerate_extensions().unwrap();

    // If a required extension isn't present, you want to ditch out here! It's possible something
    // like your rendering API might not be provided by the active runtime. APIs like OpenGL don't
    // have universal support.
    if !available_extensions.khr_vulkan_enable2 {
        println!("vulkan openxr extension not available");
        return;
    }

    // Initialize OpenXR with the extensions we've found!
    let mut enabled_extensions = openxr::ExtensionSet::default();
    enabled_extensions.khr_vulkan_enable2 = true;

    // TODO: Only enable debugging when given debugging flags
    enabled_extensions.ext_debug_utils = true;
    let core_validation_layer_name = "XR_APILAYER_LUNARG_core_validation";

    let xr_instance = xr_entry
        .create_instance(
            &openxr::ApplicationInfo {
                application_name: "indite example",
                application_version: 0,
                engine_name: "indite example",
                engine_version: 0,
                api_version: openxr::Version::new(1, 0, 0),
            },
            &enabled_extensions,
            &[core_validation_layer_name],
        )
        .unwrap();
    let instance_props = xr_instance.properties().unwrap();
    println!(
        "loaded openxr runtime: {} {}",
        instance_props.runtime_name, instance_props.runtime_version
    );

    let debug_utils = indite::DebugUtils::new(&xr_entry, &xr_instance);

    // Request a form factor from the device (HMD, Handheld, etc.)
    let xr_system = xr_instance
        .system(openxr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .unwrap();

    // Check what blend mode is valid for this device (opaque vs transparent displays). We'll just
    // take the first one available!
    let environment_blend_mode = xr_instance
        .enumerate_environment_blend_modes(xr_system, VIEW_TYPE)
        .unwrap()[0];

    // OpenXR wants to ensure apps are using the correct graphics card and Vulkan features and
    // extensions, so the instance and device MUST be set up before Instance::create_session.

    let vk_target_version_xr = openxr::Version::new(1, 1, 0);

    let reqs = xr_instance
        .graphics_requirements::<openxr::Vulkan>(xr_system)
        .unwrap();

    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        panic!(
            "openxr runtime requires vulkan version > {}, <= {}",
            reqs.min_api_version_supported,
            reqs.max_api_version_supported.major()
        );
    }

    let instance = indite::create_instance(&xr_instance, xr_system).unwrap();
    let (device, queue) = indite::create_device(&xr_instance, xr_system, &instance).unwrap();

    // Create WPGU render pipeline
    let uniform_layout = create_uniform_layout(&device);
    let render_pipeline = create_render_pipeline(&device, &uniform_layout);

    // Create and run the OpenXR session
    run_session(
        &xr_instance,
        xr_system,
        environment_blend_mode,
        &instance,
        &device,
        &queue,
        &uniform_layout,
        &render_pipeline,
    );

    // DebugUtils must be cleaned up before cleaning up OpenXR
    println!("cleaning openxr debug utils");
    drop(debug_utils);

    // Explicitly clean up OpenXR resources, just to make ordering clear
    println!("cleaning openxr instance");
    drop((xr_entry, xr_instance, xr_system));

    // Drop WGPU remaining WGPU types explicitly here, again just to make ordering clear
    println!("cleaning wgpu");
    device.destroy();
    drop((instance, device, queue, render_pipeline));

    println!("exiting cleanly");
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
        multisample: MultisampleState::default(),
        multiview: NonZero::new(VIEW_COUNT),
        cache: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    environment_blend_mode: openxr::EnvironmentBlendMode,
    instance: &Instance,
    device: &Device,
    queue: &Queue,
    uniform_layout: &BindGroupLayout,
    render_pipeline: &RenderPipeline,
) {
    // Prepare the ctrl-c handler for the loop
    let ctrlc_request_exit = Arc::new(AtomicBool::new(false));
    let r = ctrlc_request_exit.clone();
    let handler = move || r.store(true, Ordering::Relaxed);
    ctrlc::set_handler(handler).unwrap();

    // A session represents this application's desire to display things! This is where we hook
    // up our graphics API. This does not start the session; for that, you'll need a call to
    // Session::begin, which we do in 'main_loop below.
    let (session, mut frame_wait, mut frame_stream) =
        indite::create_session(xr_instance, xr_system, instance, device);

    // Find all the viewpoints for the view type we're using.
    let xr_views = xr_instance
        .enumerate_view_configuration_views(xr_system, VIEW_TYPE)
        .unwrap();
    assert_eq!(xr_views.len(), VIEW_COUNT as usize);

    // We're using plain multiview rendering right now, no foviated features, so the views should be
    // equal.
    assert_eq!(xr_views[0], xr_views[1]);

    // Create the swapchain for the session
    let swapchain_desc = indite::SwapchainDescriptor {
        width: xr_views[0].recommended_image_rect_width,
        height: xr_views[0].recommended_image_rect_height,
        view_count: VIEW_COUNT,
    };
    let (xr_swapchain, swapchain_textures) =
        indite::create_swapchain(device, &session, &swapchain_desc);

    // Create an action set to encapsulate our actions
    let action_set = xr_instance
        .create_action_set("input", "input pose information", 0)
        .unwrap();

    let right_action = action_set
        .create_action::<openxr::Posef>("right_hand", "Right Hand Controller", &[])
        .unwrap();
    let left_action = action_set
        .create_action::<openxr::Posef>("left_hand", "Left Hand Controller", &[])
        .unwrap();

    // Bind our actions to input devices using the given profile
    // If you want to access inputs specific to a particular device you may specify a different
    // interaction profile
    let bindings = [
        openxr::Binding::new(
            &right_action,
            xr_instance
                .string_to_path("/user/hand/right/input/grip/pose")
                .unwrap(),
        ),
        openxr::Binding::new(
            &left_action,
            xr_instance
                .string_to_path("/user/hand/left/input/grip/pose")
                .unwrap(),
        ),
    ];
    xr_instance
        .suggest_interaction_profile_bindings(
            xr_instance
                .string_to_path("/interaction_profiles/khr/simple_controller")
                .unwrap(),
            &bindings,
        )
        .unwrap();

    // Attach the action set to the session
    session.attach_action_sets(&[&action_set]).unwrap();

    // Create an action space for each device we want to locate
    let right_space = right_action
        .create_space(session.clone(), openxr::Path::NULL, openxr::Posef::IDENTITY)
        .unwrap();
    let left_space = left_action
        .create_space(session.clone(), openxr::Path::NULL, openxr::Posef::IDENTITY)
        .unwrap();

    // OpenXR uses a couple different types of reference frames for positioning content; we need
    // to choose one for displaying our content! STAGE would be relative to the center of your
    // guardian system's bounds, and LOCAL would be relative to your device's starting location.
    let stage = session
        .create_reference_space(openxr::ReferenceSpaceType::STAGE, openxr::Posef::IDENTITY)
        .unwrap();

    // Main loop
    let mut event_storage = openxr::EventDataBuffer::new();
    let mut session_running = false;

    'main_loop: loop {
        // Check for ctrl-c
        if ctrlc_request_exit.load(Ordering::Relaxed) {
            println!("ctrl-c requesting exit");

            // The OpenXR runtime may want to perform a smooth transition between scenes, so we
            // can't necessarily exit instantly. Instead, we must notify the runtime of our
            // intent and wait for it to tell us when we're actually done.
            match session.request_exit() {
                Ok(()) => {}
                Err(openxr::sys::Result::ERROR_SESSION_NOT_RUNNING) => break,
                Err(e) => panic!("{}", e),
            }
        }

        while let Some(event) = xr_instance.poll_event(&mut event_storage).unwrap() {
            use openxr::Event::*;
            match event {
                SessionStateChanged(e) => {
                    // Session state change is where we can begin and end sessions, as well as
                    // find quit messages!
                    println!("entered state {:?}", e.state());
                    match e.state() {
                        openxr::SessionState::READY => {
                            session.begin(VIEW_TYPE).unwrap();
                            session_running = true;
                        }
                        openxr::SessionState::STOPPING => {
                            session.end().unwrap();
                            session_running = false;
                        }
                        openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                            break 'main_loop;
                        }
                        _ => {}
                    }
                }
                InstanceLossPending(_) => {
                    break 'main_loop;
                }
                EventsLost(e) => {
                    println!("lost {} events", e.lost_event_count());
                }
                _ => {}
            }
        }

        if !session_running {
            // Don't hotloop the CPU
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Block until the previous frame is finished displaying, and is ready for another one.
        // Also returns a prediction of when the next frame will be displayed, for use with
        // predicting locations of controllers, viewpoints, etc.
        let xr_frame_state = frame_wait.wait().unwrap();

        // Must be called before any rendering is done!
        frame_stream.begin().unwrap();

        if !xr_frame_state.should_render {
            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[],
                )
                .unwrap();
            continue;
        }

        // We need to ask which swapchain image to use for rendering! Which one will we get?
        // Who knows! It's up to the runtime to decide.
        let image_index = xr_swapchain.lock().unwrap().acquire_image().unwrap();

        // Get the view for this frame
        let (_, view) = &swapchain_textures[image_index as usize];

        // Record the command buffer
        // TODO: See comment below
        //let command_buffer = record_command_buffer(device, &render_pipeline, &view);

        read_actions(
            &session,
            &action_set,
            &right_action,
            &left_action,
            &right_space,
            &left_space,
            &stage,
            &xr_frame_state,
        );

        // Fetch the view transforms. To minimize latency, we intentionally do this *after*
        // recording commands to render the scene, i.e. at the last possible moment before
        // rendering begins in earnest on the GPU. Uniforms dependent on this data can be sent
        // to the GPU just-in-time by writing them to per-frame host-visible memory which the
        // GPU will only read once the command buffer is submitted.
        let (_, views) = session
            .locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)
            .unwrap();

        // TODO: Temporarily, rendering is moved after getting views, because we need to figure out
        // how to late-update the buffers.
        let uniform_bind_group = create_uniform_bind_group(device, uniform_layout, &views);
        let command_buffer =
            record_command_buffer(device, render_pipeline, view, &uniform_bind_group);

        // Wait until the image is available to render to before beginning work on the GPU. The
        // compositor could still be reading from it.
        let mut xr_swapchain_locked = xr_swapchain.lock().unwrap();
        xr_swapchain_locked
            .wait_image(openxr::Duration::INFINITE)
            .unwrap();

        // Submit the previously prepared command buffer
        queue.submit(Some(command_buffer));

        xr_swapchain_locked.release_image().unwrap();

        // Tell OpenXR what to present for this frame
        let rect = openxr::Rect2Di {
            offset: openxr::Offset2Di { x: 0, y: 0 },
            extent: openxr::Extent2Di {
                width: xr_views[0].recommended_image_rect_width as _,
                height: xr_views[1].recommended_image_rect_height as _,
            },
        };
        let views = [
            openxr::CompositionLayerProjectionView::new()
                .pose(views[0].pose)
                .fov(views[0].fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&xr_swapchain_locked)
                        .image_array_index(0)
                        .image_rect(rect),
                ),
            openxr::CompositionLayerProjectionView::new()
                .pose(views[1].pose)
                .fov(views[1].fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&xr_swapchain_locked)
                        .image_array_index(1)
                        .image_rect(rect),
                ),
        ];
        let layer = openxr::CompositionLayerProjection::new()
            .space(&stage)
            .views(&views);
        frame_stream
            .end(
                xr_frame_state.predicted_display_time,
                environment_blend_mode,
                &[&layer],
            )
            .unwrap();
    }

    // Clean up anything WGPU that references OpenXR managed resources
    println!("cleaning wgpu handles for openxr swapchain");
    for (texture, _) in swapchain_textures {
        texture.destroy();
    }

    // Wait until WGPU is done processing everything, so we can start cleaning resources
    instance.poll_all(true);

    // OpenXR MUST be allowed to clean up before we destroy WGPU resources it could touch.
    // We're at the end of the function so this should happen automatically, but let's do it just in
    // case.
    // TODO: This is missing some values that have important drop handlers.
    // Since everything gets cleaned up automatically anyways that's not a big issue right now.
    println!("cleaning openxr session");
    drop((
        session,
        frame_wait,
        frame_stream,
        stage,
        action_set,
        left_space,
        right_space,
        left_action,
        right_action,
    ));
}

fn record_command_buffer(
    device: &Device,
    render_pipeline: &RenderPipeline,
    view: &TextureView,
    uniform_bind_group: &BindGroup,
) -> CommandBuffer {
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });

    {
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::GREEN),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(render_pipeline);
        render_pass.set_bind_group(0, uniform_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
    encoder.finish()
}

#[allow(clippy::too_many_arguments)]
fn read_actions(
    session: &openxr::Session<openxr::Vulkan>,
    action_set: &openxr::ActionSet,
    right_action: &openxr::Action<openxr::Posef>,
    left_action: &openxr::Action<openxr::Posef>,
    right_space: &openxr::Space,
    left_space: &openxr::Space,
    stage: &openxr::Space,
    xr_frame_state: &openxr::FrameState,
) {
    session.sync_actions(&[action_set.into()]).unwrap();

    // Find where our controllers are located in the Stage space
    let right_location = right_space
        .locate(stage, xr_frame_state.predicted_display_time)
        .unwrap();

    let left_location = left_space
        .locate(stage, xr_frame_state.predicted_display_time)
        .unwrap();

    let mut printed = false;
    if left_action.is_active(session, openxr::Path::NULL).unwrap() {
        print!(
            "left Hand: ({:0<12},{:0<12},{:0<12}), ",
            left_location.pose.position.x,
            left_location.pose.position.y,
            left_location.pose.position.z
        );
        printed = true;
    }

    if right_action.is_active(session, openxr::Path::NULL).unwrap() {
        print!(
            "right Hand: ({:0<12},{:0<12},{:0<12})",
            right_location.pose.position.x,
            right_location.pose.position.y,
            right_location.pose.position.z
        );
        printed = true;
    }
    if printed {
        println!();
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

fn create_uniform_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    views: &[openxr::View],
) -> BindGroup {
    let transform_0 = {
        let (translation, rotation) = openxr_pose_to_glam(&views[0].pose);
        let view = Affine3A::from_rotation_translation(rotation, translation).inverse();
        let proj = openxr_proj_to_glam(&views[0]);
        proj * view
    };
    let transform_1 = {
        let (translation, rotation) = openxr_pose_to_glam(&views[1].pose);
        let view = Affine3A::from_rotation_translation(rotation, translation).inverse();
        let proj = openxr_proj_to_glam(&views[1]);
        proj * view
    };
    let transforms = [transform_0, transform_1];

    let desc = BufferInitDescriptor {
        label: Some("uniform-buffer"),
        contents: bytemuck::bytes_of(&transforms),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    };
    let buffer = device.create_buffer_init(&desc);

    let entry = BindGroupEntry {
        binding: 0,
        resource: buffer.as_entire_binding(),
    };

    let desc = BindGroupDescriptor {
        label: Some("uniform-bind-group"),
        layout,
        entries: &[entry],
    };
    device.create_bind_group(&desc)
}

fn openxr_pose_to_glam(pose: &openxr::Posef) -> (Vec3, Quat) {
    let rotation = {
        let mut quat = Quat::from_xyzw(
            pose.orientation.x,
            pose.orientation.y,
            pose.orientation.z,
            pose.orientation.w,
        );

        if quat.length() == 0.0 {
            quat = Quat::IDENTITY;
        }

        if !quat.is_normalized() {
            quat = quat.normalize();
        }

        quat
    };

    let translation = glam::vec3(pose.position.x, pose.position.y, pose.position.z);

    (translation, rotation)
}

fn openxr_proj_to_glam(view: &openxr::View) -> Mat4 {
    // TODO: frustum_rh?
    let z_near = 0.1;
    let z_far = 100.0;

    let [tan_left, tan_right, tan_down, tan_up] = [
        view.fov.angle_left,
        view.fov.angle_right,
        view.fov.angle_down,
        view.fov.angle_up,
    ]
    .map(f32::tan);

    let tan_width = tan_right - tan_left;
    let tan_height = tan_up - tan_down;

    let a11 = 2.0 / tan_width;
    let a22 = 2.0 / tan_height;

    let a31 = (tan_right + tan_left) / tan_width;
    let a32 = (tan_up + tan_down) / tan_height;
    let a33 = -z_far / (z_far - z_near);

    let a43 = -(z_far * z_near) / (z_far - z_near);

    glam::Mat4::from_cols_array(&[
        a11, 0.0, 0.0, 0.0, //
        0.0, a22, 0.0, 0.0, //
        a31, a32, a33, -1.0, //
        0.0, 0.0, a43, 0.0, //
    ])
}
