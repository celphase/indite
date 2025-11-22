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
    let z_near = 0.1;
    let z_far = 100.0;

    let left = convert_angle(view.fov.angle_left, z_near);
    let right = convert_angle(view.fov.angle_right, z_near);
    let down = convert_angle(view.fov.angle_down, z_near);
    let up = convert_angle(view.fov.angle_up, z_near);

    Mat4::frustum_rh(left, right, down, up, z_near, z_far)
}

fn convert_angle(v: f32, z_near: f32) -> f32 {
    f32::tan(v) * z_near
}
