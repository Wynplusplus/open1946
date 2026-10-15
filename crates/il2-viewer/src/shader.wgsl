struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    model: mat4x4<f32>,
};
@group(1) @binding(0) var<uniform> instance: Instance;

@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;
struct MaterialParams {
    // x: alpha-test cutoff (0 disables the test), y: 1 for blended surfaces
    params: vec4<f32>,
};
@group(2) @binding(2) var<uniform> material: MaterialParams;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    let world = instance.model * vec4<f32>(v.pos, 1.0);
    o.clip = globals.view_proj * world;
    o.normal = normalize((instance.model * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (material.params.x > 0.0 && c.a < material.params.x) { discard; }
    let lambert = max(dot(normalize(i.normal), -globals.light_dir.xyz), 0.0);
    // Opaque surfaces must write alpha 1: skin alpha is a gloss map, not transparency.
    let a = select(1.0, c.a, material.params.y > 0.0);
    return vec4<f32>(c.rgb * (0.35 + 0.65 * lambert), a);
}
