// Sky gradient drawn behind everything: haze colour at the horizon, deeper blue towards the zenith.
struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    fog: vec4<f32>,
    // rgb: sky colour at the zenith
    zenith: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: VsOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    // View direction from the inverse of the (camera-at-origin) view-projection.
    let inv = transpose_inverse_dir(i.ndc);
    let up = clamp(inv.z, -1.0, 1.0);
    let zenith = globals.zenith.rgb;
    let t = pow(max(up, 0.0), 0.6);
    let sun = pow(max(dot(inv, -globals.light_dir.xyz), 0.0), 400.0);
    return vec4<f32>(mix(globals.fog.rgb, zenith, t) + vec3<f32>(1.0, 0.95, 0.8) * sun * step(-0.05, -globals.light_dir.z), 1.0);
}

// Direction through a screen point for a view-projection whose eye is at the origin.
fn transpose_inverse_dir(ndc: vec2<f32>) -> vec3<f32> {
    let m = globals.view_proj;
    // Solve M * (d, 1) ∝ (ndc, 1, 1) for the direction d (far plane).
    let a = mat3x3<f32>(
        vec3<f32>(m[0].x - ndc.x * m[0].w, m[0].y - ndc.y * m[0].w, m[0].z - m[0].w),
        vec3<f32>(m[1].x - ndc.x * m[1].w, m[1].y - ndc.y * m[1].w, m[1].z - m[1].w),
        vec3<f32>(m[2].x - ndc.x * m[2].w, m[2].y - ndc.y * m[2].w, m[2].z - m[2].w),
    );
    let b = -vec3<f32>(m[3].x - ndc.x * m[3].w, m[3].y - ndc.y * m[3].w, m[3].z - m[3].w);
    // Cramer's rule.
    let det = determinant(a);
    let ax = mat3x3<f32>(b, a[1], a[2]);
    let ay = mat3x3<f32>(a[0], b, a[2]);
    let az = mat3x3<f32>(a[0], a[1], b);
    return normalize(vec3<f32>(determinant(ax), determinant(ay), determinant(az)) / det);
}
