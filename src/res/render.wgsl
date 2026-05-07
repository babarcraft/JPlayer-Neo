struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec3<f32>
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>
}

@vertex
fn vertex_main(vertex: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(vertex.position, 0, 0);
    out.color = vertex.color;
    return out;
}

@fragment
fn fragment_main(vertex: VertexOutput) -> @location(0) vec4<f32> {
    return vertex.color;
}