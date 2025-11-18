pub struct ActionSetBundle {
    action_set: openxr::ActionSet,
    left_action: openxr::Action<openxr::Posef>,
    right_action: openxr::Action<openxr::Posef>,
    left_space: openxr::Space,
    right_space: openxr::Space,
}

pub fn create_action_set(
    xr_instance: &openxr::Instance,
    xr_session: &openxr::Session<openxr::Vulkan>,
) -> ActionSetBundle {
    // Create an action set to encapsulate our actions
    let action_set = xr_instance
        .create_action_set("input", "input pose information", 0)
        .unwrap();

    let left_action = action_set
        .create_action::<openxr::Posef>("left_hand", "Left Hand Controller", &[])
        .unwrap();
    let right_action = action_set
        .create_action::<openxr::Posef>("right_hand", "Right Hand Controller", &[])
        .unwrap();

    // Bind our actions to input devices using the given profile
    // If you want to access inputs specific to a particular device you may specify a different
    // interaction profile
    let bindings = [
        openxr::Binding::new(
            &left_action,
            xr_instance
                .string_to_path("/user/hand/left/input/grip/pose")
                .unwrap(),
        ),
        openxr::Binding::new(
            &right_action,
            xr_instance
                .string_to_path("/user/hand/right/input/grip/pose")
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
    xr_session.attach_action_sets(&[&action_set]).unwrap();

    // Create an action space for each device we want to locate
    let left_space = left_action
        .create_space(
            xr_session.clone(),
            openxr::Path::NULL,
            openxr::Posef::IDENTITY,
        )
        .unwrap();
    let right_space = right_action
        .create_space(
            xr_session.clone(),
            openxr::Path::NULL,
            openxr::Posef::IDENTITY,
        )
        .unwrap();

    ActionSetBundle {
        action_set,
        left_action,
        right_action,
        left_space,
        right_space,
    }
}

pub fn read_actions(
    xr_session: &openxr::Session<openxr::Vulkan>,
    action_set_bundle: &ActionSetBundle,
    xr_stage: &openxr::Space,
    xr_frame_state: &openxr::FrameState,
) {
    let active_action_set = (&action_set_bundle.action_set).into();
    xr_session.sync_actions(&[active_action_set]).unwrap();

    // Find where our controllers are located in the Stage space
    let left_location = action_set_bundle
        .left_space
        .locate(xr_stage, xr_frame_state.predicted_display_time)
        .unwrap();

    let right_location = action_set_bundle
        .right_space
        .locate(xr_stage, xr_frame_state.predicted_display_time)
        .unwrap();

    let mut printed = false;
    if action_set_bundle
        .left_action
        .is_active(xr_session, openxr::Path::NULL)
        .unwrap()
    {
        print!(
            "left Hand: ({:0<12},{:0<12},{:0<12}), ",
            left_location.pose.position.x,
            left_location.pose.position.y,
            left_location.pose.position.z
        );
        printed = true;
    }

    if action_set_bundle
        .right_action
        .is_active(xr_session, openxr::Path::NULL)
        .unwrap()
    {
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
