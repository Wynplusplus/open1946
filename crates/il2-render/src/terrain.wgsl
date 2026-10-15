// Terrain: the map's land-type grid selects field textures, blended across cells.
struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    fog: vec4<f32>, // rgb, density per metre
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    model: mat4x4<f32>,
};
@group(1) @binding(0) var<uniform> instance: Instance;
@group(1) @binding(1) var<storage, read> nodes: array<mat4x4<f32>>;

struct TerrainInfo {
    // x: cell size (m), y: columns, z: rows
    grid: vec4<f32>,
    // repeat length (m) per field slot
    tile: array<vec4<f32>, 8>,
};
@group(2) @binding(0) var types: texture_2d<u32>;
@group(2) @binding(1) var fields: texture_2d_array<f32>;
@group(2) @binding(2) var samp: sampler;
@group(2) @binding(3) var<uniform> info: TerrainInfo;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) node: u32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world_xy: vec2<f32>,
    @location(2) view: vec3<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    let rel = instance.model * vec4<f32>(v.pos, 1.0);
    o.clip = globals.view_proj * rel;
    o.normal = v.normal;
    o.world_xy = v.pos.xy;
    o.view = rel.xyz;
    return o;
}

fn tile_len(slot: u32) -> f32 {
    return info.tile[slot / 4u][slot % 4u];
}

// rgb: field colour, a: 1 for water slots (28..31)
fn field(col: i32, row: i32, xy: vec2<f32>) -> vec4<f32> {
    let cols = i32(info.grid.y);
    let rows = i32(info.grid.z);
    let c = clamp(col, 0, cols - 1);
    let r = clamp(row, 0, rows - 1);
    let slot = textureLoad(types, vec2<i32>(c, r), 0).r & 31u;
    let rgb = textureSample(fields, samp, xy / tile_len(slot), i32(slot)).rgb;
    return vec4<f32>(rgb, select(0.0, 1.0, slot >= 28u));
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let cell = info.grid.x;
    // Cell coordinates with row 0 at the north edge; blend between the 4 nearest cell centres.
    let fx = i.world_xy.x / cell - 0.5;
    let fy = (info.grid.z - 1.0) - i.world_xy.y / cell + 0.5;
    let c0 = floor(fx);
    let r0 = floor(fy);
    let t = vec2<f32>(fx - c0, fy - r0);
    let ci = i32(c0);
    let ri = i32(r0);
    let a = field(ci, ri, i.world_xy);
    let b = field(ci + 1, ri, i.world_xy);
    let c = field(ci, ri + 1, i.world_xy);
    let d = field(ci + 1, ri + 1, i.world_xy);
    let col = mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
    let lambert = max(dot(normalize(i.normal), -globals.light_dir.xyz), 0.0);
    var lit = col.rgb * (0.4 + 0.6 * lambert);
    // Water reflects the sky, more so at grazing angles.
    let v = normalize(-i.view);
    let fresnel = 0.1 + 0.9 * pow(1.0 - max(v.z, 0.0), 5.0);
    lit = mix(lit, globals.fog.rgb * 0.95, col.a * fresnel);
    let f = 1.0 - exp(-globals.fog.w * length(i.view));
    return vec4<f32>(mix(lit, globals.fog.rgb, f), 1.0);
}
