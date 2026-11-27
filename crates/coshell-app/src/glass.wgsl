// Frosted glass: what is behind a panel, made small, blurred both ways, and drawn back under it
// a little brighter and more colourful, the way macOS draws its sidebars and sheets.

struct Pass {
    // xy: one texel of the source; zw: the direction to blur in (blur passes)
    step: vec4<f32>,
    // x: corner radius (px), y: how much of it shows (0 → 1), zw: the panel's size (px)
    shape: vec4<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var smooth_sampler: sampler;
@group(0) @binding(2) var<uniform> pass_: Pass;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

// Half the size: four texels averaged, each read between four more by the sampler.
@fragment
fn fs_down(in: Out) -> @location(0) vec4<f32> {
    let t = pass_.step.xy;
    var c = textureSample(source, smooth_sampler, in.uv + vec2<f32>(-t.x, -t.y));
    c += textureSample(source, smooth_sampler, in.uv + vec2<f32>(t.x, -t.y));
    c += textureSample(source, smooth_sampler, in.uv + vec2<f32>(-t.x, t.y));
    c += textureSample(source, smooth_sampler, in.uv + vec2<f32>(t.x, t.y));
    return c * 0.25;
}

// A wide gaussian along one direction, nine reads standing for seventeen texels.
@fragment
fn fs_blur(in: Out) -> @location(0) vec4<f32> {
    let d = pass_.step.xy * pass_.step.zw;
    let w = array<f32, 5>(0.2270270270, 0.3162162162, 0.0702702703, 0.0162162162, 0.0013513514);
    let o = array<f32, 5>(0.0, 1.3846153846, 3.2307692308, 5.0769230769, 6.9230769231);
    var c = textureSample(source, smooth_sampler, in.uv) * w[0];
    for (var i = 1; i < 5; i++) {
        c += textureSample(source, smooth_sampler, in.uv + d * o[i] * 1.6) * w[i];
        c += textureSample(source, smooth_sampler, in.uv - d * o[i] * 1.6) * w[i];
    }
    return c;
}

// Distance from a rounded rectangle of half size `b` and corner `r` (negative inside).
fn rounded(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// The blurred backdrop, drawn into the panel's rounded shape.
@fragment
fn fs_glass(in: Out) -> @location(0) vec4<f32> {
    let size = pass_.shape.zw;
    var c = textureSample(source, smooth_sampler, in.uv).rgb;
    // Livelier colour, as glass over a lit scene looks.
    let grey = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3<f32>(grey), c, 1.45), vec3<f32>(0.0));
    // A little light caught in it, more toward the top.
    c = c * 1.08 + vec3<f32>(0.018, 0.017, 0.024) * (1.4 - in.uv.y);
    let p = (in.uv - vec2<f32>(0.5)) * size;
    let d = rounded(p, size * 0.5, pass_.shape.x);
    let inside = clamp(0.5 - d, 0.0, 1.0) * pass_.shape.y;
    return vec4<f32>(c * inside, inside);
}
