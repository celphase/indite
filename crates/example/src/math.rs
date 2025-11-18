use glam::{Affine3A, Mat4, Quat, Vec3};

pub fn matrix_from_view(view: &openxr::View) -> Mat4 {
    let (translation, rotation) = openxr_pose_to_glam(&view.pose);
    let view_matrix = Affine3A::from_rotation_translation(rotation, translation).inverse();
    let projection_matrix = openxr_projection_to_glam(view);
    projection_matrix * view_matrix
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

fn openxr_projection_to_glam(view: &openxr::View) -> Mat4 {
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
