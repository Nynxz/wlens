// Fullscreen passes for mip generation and native-pixel presentation.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src, samp, in.uv);
}

@group(1) @binding(0) var<uniform> screen: vec4<f32>;

// Copy exactly one texel per output pixel, starting at the bottom row of
// the rendered viewport. The intermediate texture may have spare capacity.
@fragment
fn fs_bottom_up(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.pos.xy);
    return textureLoad(src, vec2<i32>(pixel.x, i32(screen.y) - 1 - pixel.y), 0);
}
