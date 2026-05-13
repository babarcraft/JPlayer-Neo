struct Params {
    stride_y: u32,
    stride_uv: u32,
    width: u32,
    height: u32,
};

@group(0) @binding(0) var<storage, read> y_buffer: array<u32>;
@group(0) @binding(1) var<storage, read> uv_buffer: array<u32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

@group(0) @binding(3) var<uniform> color_space: mat3x3<f32>;
@group(0) @binding(4) var<uniform> color_offset: vec3<f32>;
@group(0) @binding(5) var<uniform> params: Params;

// Instead of passing the buffer, we pass the packed U32 value
// and the byte index within that specific U32.
fn extract_byte(packed_val: u32, byte_index: u32) -> f32 {
    let byte_offset = byte_index % 4u;
    let byte_val = (packed_val >> (byte_offset * 8u)) & 0xFFu;
    return f32(byte_val) / 255.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let y_idx = gid.y * params.stride_y + gid.x;
    let y_val = extract_byte(y_buffer[y_idx / 4u], y_idx);

    let uv_row = gid.y / 2u;
    let uv_col_base = (gid.x / 2u) * 2u; // Start of the UV pair

    let u_idx = uv_row * params.stride_uv + uv_col_base;
    let v_idx = u_idx + 1u;

    let u = extract_byte(uv_buffer[u_idx / 4u], u_idx);
    let v = extract_byte(uv_buffer[v_idx / 4u], v_idx);

    let yuv = vec3<f32>(y_val, u, v);
    let rgb = color_space * (yuv + color_offset);

    textureStore(out_tex, vec2<u32>(gid.xy), vec4<f32>(rgb, 1.0));
}