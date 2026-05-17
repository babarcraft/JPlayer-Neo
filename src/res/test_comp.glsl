#version 430

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(rgba8, binding = 0) writeonly uniform image2D dest;

uniform float coef;

void main() {
    ivec2 current = ivec2(gl_GlobalInvocationID.xy);
    ivec2 size = imageSize(dest);
    if(current.x >= size.x || current.y >= size.y) return;
    vec4 value = vec4(0.0, 0.0, 0.0, 1.0);
    value.x = abs(cos((float(current.x) / gl_NumWorkGroups.x) * 0.5) + coef);
    value.y = abs(cos((float(current.x) / gl_NumWorkGroups.x) * 0.2) + coef);
    value.z = abs(cos((float(current.x) / gl_NumWorkGroups.x) * 0.1) + coef);
    imageStore(dest, current, value);
}