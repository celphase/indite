mod actions;
mod math;
mod rendering;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::rendering::RenderContext;

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

    let render_context = RenderContext::new(&xr_instance, xr_system);

    // Create and run the OpenXR session
    run_session(
        &xr_instance,
        xr_system,
        environment_blend_mode,
        &render_context,
    );

    // DebugUtils must be cleaned up before cleaning up OpenXR
    println!("cleaning openxr debug utils");
    drop(debug_utils);

    // Explicitly clean up OpenXR resources, just to make ordering clear
    println!("cleaning openxr instance");
    drop((xr_entry, xr_instance, xr_system));

    // Drop WGPU remaining WGPU types explicitly here, again just to make ordering clear
    println!("cleaning wgpu");
    drop(render_context);

    println!("exiting cleanly");
}

fn run_session(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    environment_blend_mode: openxr::EnvironmentBlendMode,
    render_context: &RenderContext,
) {
    // Prepare the ctrl-c handler for the loop
    let ctrlc_request_exit = Arc::new(AtomicBool::new(false));
    let r = ctrlc_request_exit.clone();
    let handler = move || r.store(true, Ordering::Relaxed);
    ctrlc::set_handler(handler).unwrap();

    // A session represents this application's desire to display things! This is where we hook
    // up our graphics API. This does not start the session; for that, you'll need a call to
    // Session::begin, which we do in 'main_loop below.
    let (xr_session, mut frame_wait, mut frame_stream) = indite::create_session(
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
    let (xr_swapchain_handle, swapchain_textures) =
        indite::create_swapchain(&render_context.device, &xr_session, &swapchain_desc).unwrap();

    let action_set_bundle = actions::create_action_set(xr_instance, &xr_session);

    // OpenXR uses a couple different types of reference frames for positioning content; we need
    // to choose one for displaying our content! STAGE would be relative to the center of your
    // guardian system's bounds, and LOCAL would be relative to your device's starting location.
    let xr_stage = xr_session
        .create_reference_space(openxr::ReferenceSpaceType::STAGE, openxr::Posef::IDENTITY)
        .unwrap();

    // Main loop
    let mut event_storage = openxr::EventDataBuffer::new();
    let mut session_running = false;

    loop {
        let should_continue = handle_ctrlc(&ctrlc_request_exit, &xr_session);
        if !should_continue {
            break;
        }

        let should_continue = handle_instance_events(
            xr_instance,
            &xr_session,
            &mut event_storage,
            &mut session_running,
        );
        if !should_continue {
            break;
        }

        if !session_running {
            // Don't hotloop the CPU
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        rendering::render_frame(
            environment_blend_mode,
            render_context,
            &xr_session,
            &mut frame_wait,
            &mut frame_stream,
            &swapchain_desc,
            &xr_swapchain_handle,
            &swapchain_textures,
            &action_set_bundle,
            &xr_stage,
        );
    }

    // Clean up anything WGPU that references OpenXR managed resources
    println!("cleaning wgpu handles for openxr swapchain");
    for (texture, _) in swapchain_textures {
        texture.destroy();
    }

    // Wait until WGPU is done processing everything, so we can start cleaning resources
    render_context.instance.poll_all(true);
}

fn handle_ctrlc(
    ctrlc_request_exit: &Arc<AtomicBool>,
    xr_session: &openxr::Session<openxr::Vulkan>,
) -> bool {
    // Check for ctrl-c
    if ctrlc_request_exit.load(Ordering::Relaxed) {
        println!("ctrl-c requesting exit");

        // The OpenXR runtime may want to perform a smooth transition between scenes, so we
        // can't necessarily exit instantly. Instead, we must notify the runtime of our
        // intent and wait for it to tell us when we're actually done.
        match xr_session.request_exit() {
            Ok(()) => {}
            Err(openxr::sys::Result::ERROR_SESSION_NOT_RUNNING) => return false,
            Err(e) => panic!("{}", e),
        }
    }

    true
}

fn handle_instance_events(
    xr_instance: &openxr::Instance,
    xr_session: &openxr::Session<openxr::Vulkan>,
    event_storage: &mut openxr::EventDataBuffer,
    session_running: &mut bool,
) -> bool {
    while let Some(event) = xr_instance.poll_event(event_storage).unwrap() {
        use openxr::Event::*;
        match event {
            SessionStateChanged(e) => {
                // Session state change is where we can begin and end sessions, as well as
                // find quit messages!
                println!("entered state {:?}", e.state());
                match e.state() {
                    openxr::SessionState::READY => {
                        xr_session.begin(VIEW_TYPE).unwrap();
                        *session_running = true;
                    }
                    openxr::SessionState::STOPPING => {
                        xr_session.end().unwrap();
                        *session_running = false;
                    }
                    openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                        return false;
                    }
                    _ => {}
                }
            }
            InstanceLossPending(_) => {
                return false;
            }
            EventsLost(e) => {
                println!("lost {} events", e.lost_event_count());
            }
            _ => {}
        }
    }

    true
}
