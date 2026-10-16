struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    fog: vec4<f32>, // rgb, density per metre
    // rgb: sky colour at the zenith
    zenith: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    model: mat4x4<f32>,
};
@group(1) @binding(0) var<uniform> instance: Instance;
// Per-node transforms of an animated hierarchy (node 0 is identity for plain meshes).
@group(1) @binding(1) var<storage, read> nodes: array<mat4x4<f32>>;

@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;
struct MaterialParams {
    // x: alpha-test cutoff (0 disables the test), y: 1 for blended surfaces, z: 1 for self-lit
    params: vec4<f32>,
};
@group(2) @binding(2) var<uniform> material: MaterialParams;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) node: u32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) view: vec3<f32>,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    let m = instance.model * nodes[v.node];
    let world = m * vec4<f32>(v.pos, 1.0);
    o.clip = globals.view_proj * world;
    o.normal = normalize((m * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    o.view = world.xyz;
    o.color = vec4<f32>(1.0);
    return o;
}

struct InstIn {
    @location(4) r0: vec4<f32>,
    @location(5) r1: vec4<f32>,
    @location(6) r2: vec4<f32>,
    @location(7) color: vec4<f32>,
};

// Instanced draws: a per-instance affine transform between the model and node transforms.
@vertex
fn vs_inst(v: VsIn, i: InstIn) -> VsOut {
    var o: VsOut;
    let inst = mat4x4<f32>(
        vec4<f32>(i.r0.x, i.r1.x, i.r2.x, 0.0),
        vec4<f32>(i.r0.y, i.r1.y, i.r2.y, 0.0),
        vec4<f32>(i.r0.z, i.r1.z, i.r2.z, 0.0),
        vec4<f32>(i.r0.w, i.r1.w, i.r2.w, 1.0),
    );
    let m = instance.model * inst * nodes[v.node];
    let world = m * vec4<f32>(v.pos, 1.0);
    o.clip = globals.view_proj * world;
    o.normal = normalize((m * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    o.view = world.xyz;
    o.color = i.color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv);
    // The alpha test applies to the texture; the instance colour (particle fade) multiplies after.
    if (material.params.x > 0.0 && t.a < material.params.x) { discard; }
    let c = t * i.color;
    let lambert = max(dot(normalize(i.normal), -globals.light_dir.xyz), 0.0);
    // Opaque surfaces must write alpha 1: skin alpha is a gloss map, not transparency.
    let a = select(1.0, c.a, material.params.y > 0.0);
    let f = 1.0 - exp(-globals.fog.w * length(i.view));
    let shade = select((0.35 + 0.65 * lambert) * globals.light_dir.w, 1.0, material.params.z > 0.0);
    return vec4<f32>(mix(c.rgb * shade, globals.fog.rgb, f), a);
}
