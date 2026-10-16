// Screen-space overlay (HUD): textured, coloured quads in pixels, alpha blended.
struct Screen {
    size: vec4<f32>,
};
@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = vec4<f32>(v.pos.x / screen.size.x * 2.0 - 1.0, 1.0 - v.pos.y / screen.size.y * 2.0, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    // The atlas holds coverage in alpha; white texels for solid shapes.
    let t = textureSample(tex, samp, i.uv);
    return vec4<f32>(i.color.rgb * t.rgb, i.color.a * t.a);
}
