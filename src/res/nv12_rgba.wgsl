struct ColorInfo {
    color_space: mat3x3<f32>,
    color_offset: vec3<f32>,
    others: vec4<f32>,
}

@group(0) @binding(0)
var y_tex: texture_2d<f32>;

@group(0) @binding(1)
var uv_tex: texture_2d<f32>;

@group(0) @binding(2)
var out_tex: texture_storage_2d<rgba8unorm, write>;

@group(0) @binding(3)
var sampl: sampler;

@group(0) @binding(4)
var<uniform> color_space: mat3x3<f32>;
@group(0) @binding(5)
var<uniform> color_offset: vec3<f32>;

@compute @workgroup_size(8, 8)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>
) {
    let out_size = textureDimensions(out_tex);

    if (gid.x >= out_size.x || gid.y >= out_size.y) {
        return;
    }

    let uv_coord = (vec2<f32>(gid.xy) + vec2<f32>(0.5, 0.5)) / vec2<f32>(out_size);

    let y = textureSampleLevel(y_tex, sampl, uv_coord, 0.0).r;
    let uv = textureSampleLevel(uv_tex, sampl, uv_coord, 0.0).rg;

    let rgb = color_space * (vec3<f32>(y, uv.x, uv.y) + color_offset);

    textureStore(
        out_tex,
        vec2<u32>(gid.xy),
        vec4<f32>(rgb, 1.0)
    );
}