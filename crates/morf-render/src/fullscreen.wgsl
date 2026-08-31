// The vertex half of every pass that covers the whole target.
//
// Prepended to the blur, composite and clear shaders at pipeline creation.
// Three copies of this used to be pasted into three files, two of them
// byte-identical and the third gratuitously different.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// One triangle large enough to cover the target, so there is no second
/// triangle and no seam down the diagonal between them.
@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return output;
}
