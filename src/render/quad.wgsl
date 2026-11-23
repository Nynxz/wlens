// One instanced primitive for everything that isn't text: rounded boxes,
// borders, soft shadows, and textured images (with checkerboard).
// Shapes are signed distance fields, so corners/borders are resolution-free
// and anti-aliased. Output is premultiplied alpha.

struct Globals {
    // xy: surface size px, z: checker size px
    screen: vec4<f32>,
    checker_a: vec4<f32>,
    checker_b: vec4<f32>,
    window_per_texel: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,         // x, y, w, h
    @location(1) fill: vec4<f32>,         // linear rgba, straight alpha
    @location(2) border_color: vec4<f32>,
    @location(3) uv0: vec4<f32>,          // screen -> uv, row 0 (xyz)
    @location(4) uv1: vec4<f32>,
    @location(5) params: vec4<f32>,       // radius, border width, blur, opacity
    @location(6) flags: vec4<f32>,        // textured, checker, ring, ring progress
    @location(7) clip: vec4<f32>,         // x, y, w, h; w <= 0: no clip
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half: vec2<f32>,
    @location(2) @interpolate(flat) fill: vec4<f32>,
    @location(3) @interpolate(flat) border_color: vec4<f32>,
    @location(4) @interpolate(flat) uv0: vec4<f32>,
    @location(5) @interpolate(flat) uv1: vec4<f32>,
    @location(6) @interpolate(flat) params: vec4<f32>,
    @location(7) @interpolate(flat) flags: vec4<f32>,
    @location(8) @interpolate(flat) clip: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    // two triangles: (0,0) (1,0) (0,1) / (0,1) (1,0) (1,1)
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let c = corners[vi];
    let half = inst.rect.zw * 0.5;
    let center = inst.rect.xy + half;
    // Grow the quad so soft shadows and AA fringes aren't clipped.
    let grow = inst.params.z * 2.0 + 1.0;
    let local = (c * 2.0 - 1.0) * (half + grow);
    let p = center + local;

    var o: VOut;
    o.pos = vec4<f32>(p.x / g.screen.x * 2.0 - 1.0, 1.0 - p.y / g.screen.y * 2.0, 0.0, 1.0);
    o.local = local;
    o.half = half;
    o.fill = inst.fill;
    o.border_color = inst.border_color;
    o.uv0 = inst.uv0;
    o.uv1 = inst.uv1;
    o.params = inst.params;
    o.flags = inst.flags;
    o.clip = inst.clip;
    return o;
}

fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let rr = min(r, min(half.x, half.y));
    let q = abs(p) - half + rr;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rr;
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let radius = in.params.x;
    let bw = in.params.y;
    let blur = in.params.z;
    let opacity = in.params.w;
    let screen_pos = in.pos.xy * g.window_per_texel.xy;

    // Scrolling areas clip what's inside them.
    if (in.clip.z > 0.0) {
        let p = screen_pos;
        if (p.x < in.clip.x || p.y < in.clip.y || p.x > in.clip.x + in.clip.z || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }

    if (in.flags.z > 0.5) {
        // Progress ring: a circular band (width = border width), filled
        // clockwise from the top up to flags.w; the rest in border_color.
        let r = min(in.half.x, in.half.y);
        let len = length(in.local);
        let d_ring = max(len - r, (r - bw) - len);
        let cov = clamp(0.5 - d_ring, 0.0, 1.0);
        if (cov <= 0.0) {
            discard;
        }
        var a = atan2(in.local.x, -in.local.y);
        if (a < 0.0) {
            a = a + 6.2831853;
        }
        // params.x: where the arc starts, in turns (spinners rotate it).
        let t = fract(a / 6.2831853 - radius + 1.0);
        let filled = t <= in.flags.w;
        let col = select(in.border_color, in.fill, filled);
        return premul(col) * cov * opacity;
    }

    let d = sd_round_box(in.local, in.half, radius);

    if (blur > 0.0) {
        // Soft shadow: smooth falloff either side of the edge.
        let a = 1.0 - smoothstep(-blur, blur, d);
        return premul(in.fill) * a * opacity;
    }

    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    var c = premul(in.fill);
    if (in.flags.x > 0.5) {
        // The screen-to-UV rows give the exact gradients in physical pixels.
        // Explicit gradients let us sample only the fragments that use an image.
        let s = vec3<f32>(screen_pos, 1.0);
        let uv = vec2<f32>(dot(in.uv0.xyz, s), dot(in.uv1.xyz, s));
        let dx = vec2(in.uv0.x, in.uv1.x) * g.window_per_texel.x;
        let dy = vec2(in.uv0.y, in.uv1.y) * g.window_per_texel.y;
        let texel = textureSampleGrad(tex, samp, uv, dx, dy);
        var under = c;
        if (in.flags.y > 0.5) {
            let cell = floor(screen_pos / g.screen.z);
            let odd = (i32(cell.x) + i32(cell.y)) & 1;
            under = premul(select(g.checker_a, g.checker_b, odd == 1));
        }
        c = premul(texel) + under * (1.0 - texel.a);
    }
    if (bw > 0.0) {
        // 1 inside the border band, 0 in the interior, AA'd at its inner edge.
        let band = clamp(d + bw + 0.5, 0.0, 1.0);
        c = mix(c, premul(in.border_color), band);
    }
    return c * coverage * opacity;
}
