mod actions;
mod math;
mod rendering;
mod session;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{bail, Error};

use crate::rendering::RenderContext;

const VIEW_TYPE: openxr::ViewConfigurationType = openxr::ViewConfigurationType::PRIMARY_STEREO;

const VIEW_COUNT: u32 = 2;

pub fn main() -> Result<(), Error> {
    let ctrlc_request_exit = create_ctrlc_handler();

    let xr_entry = openxr::Entry::linked();
    let xr_instance = create_openxr_instance(&xr_entry)?;
    let _debug_utils = indite::DebugUtils::new(&xr_entry, &xr_instance);

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
    let mut session_bundle = session::create_session(&xr_instance, xr_system, &render_context);
    let action_set_bundle = actions::create_action_set(&xr_instance, &session_bundle.xr_session);

    // Main loop
    let mut event_storage = openxr::EventDataBuffer::new();
    let mut session_running = false;

    loop {
        let should_continue = handle_ctrlc(&ctrlc_request_exit, &session_bundle.xr_session);
        if !should_continue {
            break;
        }

        let should_continue = handle_instance_events(
            &xr_instance,
            &session_bundle.xr_session,
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
            &render_context,
            &mut session_bundle,
            &action_set_bundle,
        );
    }

    println!("exiting cleanly");
    Ok(())
}

fn create_openxr_instance(xr_entry: &openxr::Entry) -> Result<openxr::Instance, Error> {
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
        bail!("vulkan openxr extension not available");
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

    Ok(xr_instance)
}

fn create_ctrlc_handler() -> Arc<AtomicBool> {
    let ctrlc_request_exit = Arc::new(AtomicBool::new(false));
    let r = ctrlc_request_exit.clone();

    let handler = move || r.store(true, Ordering::Relaxed);
    ctrlc::set_handler(handler).unwrap();

    ctrlc_request_exit
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
        match event {
            openxr::Event::SessionStateChanged(e) => {
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
            openxr::Event::InstanceLossPending(_) => {
                return false;
            }
            openxr::Event::EventsLost(e) => {
                println!("lost {} events", e.lost_event_count());
            }
            _ => {}
        }
    }

    true
}
