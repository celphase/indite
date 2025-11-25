struct UniformData {
    matrices: array<mat4x4<f32>, 2>,
}

@group(0)
@binding(0)
var<uniform> u_data: UniformData;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32, @builtin(view_index) view_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(in_vertex_index) - 1);
    let y = f32(i32(in_vertex_index & 1u) * 2 - 1);
    let position = vec4<f32>(x, y, 0.0, 1.0);
    let matrix = u_data.matrices[view_index];
    return matrix * position;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
