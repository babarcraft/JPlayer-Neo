#version 430

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(binding = 0) uniform sampler2D texY;

// Interleaved UV plane
layout(binding = 1) uniform sampler2D texUV;

// Output RGBA texture
layout(rgba8, binding = 2) writeonly uniform image2D outImage;

void main() {
    ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);

    ivec2 outSize = imageSize(outImage);

    if (pixel.x >= outSize.x || pixel.y >= outSize.y)
    return;

    //--------------------------------------------------
    // Fetch Y
    //--------------------------------------------------

    // Y texture is full resolution
    float y = texelFetch(texY, pixel, 0).r;

    //--------------------------------------------------
    // Fetch UV
    //--------------------------------------------------

    // NV12 UV plane is half resolution
    ivec2 uvCoord = pixel / 2;

    vec2 uv = texelFetch(texUV, uvCoord, 0).rg;

    float u = uv.x;
    float v = uv.y;

    //--------------------------------------------------
    // Convert from video range
    //--------------------------------------------------

    // NV12 usually uses limited range:
    //
    // Y:  [16,235]
    // UV: [16,240]
    //
    // Normalize properly.

    y = (y * 255.0 - 16.0) / 219.0;

    u = (u * 255.0 - 128.0) / 224.0;
    v = (v * 255.0 - 128.0) / 224.0;

    //--------------------------------------------------
    // BT.709 conversion
    //--------------------------------------------------

    float r = y + 1.5748 * v;
    float g = y - 0.1873 * u - 0.4681 * v;
    float b = y + 1.8556 * u;

    //--------------------------------------------------
    // Clamp
    //--------------------------------------------------

    vec4 rgba = vec4(
    clamp(r, 0.0, 1.0),
    clamp(g, 0.0, 1.0),
    clamp(b, 0.0, 1.0),
    1.0
    );

    imageStore(outImage, pixel, rgba);
}