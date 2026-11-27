// The AI's light: an orb of glass with a voice inside it, and the wave over the conversation.
// Every pixel adds up the light that reaches it, so nothing here has an edge.

struct Light {
    // x: seconds, y: voice (0 sea → 1 speaking), z: surge (1 → 0 on arrival), w: 0 orb, 1 wave
    a: vec4<f32>,
    // x: 1 when the target stores linear light
    b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Light;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    // One triangle over the whole of the widget.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

const PI: f32 = 3.14159265;
const PURPLE: vec3<f32> = vec3<f32>(0.72, 0.36, 1.0);
const BLUE: vec3<f32> = vec3<f32>(0.16, 0.5, 1.0);
const TEAL: vec3<f32> = vec3<f32>(0.3, 0.86, 1.0);
const ROSE: vec3<f32> = vec3<f32>(1.0, 0.42, 0.62);
const EMBER: vec3<f32> = vec3<f32>(1.0, 0.5, 0.26);

fn tint(k: i32) -> vec3<f32> {
    switch k {
        case 0 { return PURPLE; }
        case 1 { return BLUE; }
        case 2 { return TEAL; }
        default { return ROSE; }
    }
}

// How loud a voice is at this moment: smooth syllables, never a jump.
fn syllable(t: f32, rate: f32, seed: f32) -> f32 {
    let s = sin(rate * t + seed) * sin(0.47 * rate * t + 1.7 * seed);
    let breath = 0.5 + 0.5 * sin(0.9 * t + 2.3 * seed);
    return s * s * (0.55 + 0.45 * breath);
}

// ---------------------------------------------------------------- orb

fn orb(uv: vec2<f32>) -> vec4<f32> {
    let t = u.a.x;
    let voice = u.a.y;
    // The glass keeps a little of its box free for the light it gives off.
    let R = 0.9;
    let p = (uv * 2.0 - 1.0) / R;
    let r = length(p);
    let px = fwidth(p.x);
    let inside = 1.0 - smoothstep(1.0 - 1.5 * px, 1.0 + 0.5 * px, r);

    // Depth of the glass: a ball seen from the front.
    let z = sqrt(max(1.0 - r * r, 0.0));

    // The dark of the glass, a little lighter where the light comes from.
    var col = mix(vec3<f32>(0.016, 0.014, 0.04), vec3<f32>(0.085, 0.075, 0.17), smoothstep(1.3, -0.9, p.x + p.y));
    // A faint light at its heart.
    col += vec3<f32>(0.1, 0.09, 0.2) * exp(-r * r * 3.2) * (0.7 + 0.5 * voice);

    // The voice: the conversation's own wave, fine threads of light crossing the glass, each
    // with a glow round it and a breath of light down to the middle.
    let x = p.x;
    let room = sqrt(max(1.0 - x * x, 0.0));
    let py = fwidth(p.y);
    let fine = max(1.1 * py, 0.016);
    var lit = vec3<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let b = f32(k);
        var colour = tint(k);
        if (k == 3) { colour = EMBER; }
        let share = 1.0 - 0.13 * b;
        // Never taller than the glass is there; tall crests lean in instead of being cut.
        let at = 0.8 * room * tanh(ribbon(0.5 + 0.5 * x, t + 0.7 * b, k, voice, 0.42) * share * 2.4);
        let d = abs(p.y - at);
        let thread = exp(-pow(d / fine, 2.0));
        let glow = exp(-d / (5.0 * fine)) * 0.2;
        let between = step(min(at, 0.0), p.y) * step(p.y, max(at, 0.0));
        let body = between * (0.1 + 0.2 * abs(p.y) / max(abs(at), 0.001)) * smoothstep(0.0, 0.1, abs(at));
        lit += colour * (thread * 0.8 + glow + body) * (1.0 - 0.1 * b) * (0.4 + 0.6 * smoothstep(0.0, 0.2, abs(at)));
    }
    // Out of nothing at the glass's edge.
    lit *= smoothstep(0.0, 0.75, room);
    // Light adds up, then rolls off toward white where it is crowded.
    lit *= 0.8 + 0.2 * z;
    col += vec3<f32>(1.0) - exp(-lit * 1.05);

    // The glass itself: darker toward its edge, a rim that catches colour, a soft highlight.
    col *= 0.5 + 0.5 * smoothstep(0.0, 0.55, z);
    let fres = pow(1.0 - z, 2.6);
    let around = 0.5 + 0.5 * sin(atan2(p.y, p.x) * 1.0 + 0.9);
    let rim = mix(mix(PURPLE, TEAL, around), BLUE, smoothstep(0.0, 1.0, p.x * 0.5 + p.y * 0.5 + 0.5));
    col += rim * fres * (0.55 + 0.25 * voice);
    let hp = (p - vec2<f32>(-0.3, -0.5)) / vec2<f32>(0.5, 0.24);
    col += vec3<f32>(0.85, 0.9, 1.0) * exp(-dot(hp, hp) * 1.6) * 0.13;
    let lp = (p - vec2<f32>(0.25, 0.66)) / vec2<f32>(0.5, 0.16);
    col += mix(PURPLE, BLUE, 0.5) * exp(-dot(lp, lp) * 1.8) * 0.1;

    // The light it gives off, fading to nothing before its box ends.
    let out_r = max(r - 1.0, 0.0);
    let reach = (1.0 / R - 1.0);
    let fade = 1.0 - smoothstep(0.0, reach, out_r);
    let aura = mix(PURPLE, BLUE, smoothstep(-1.0, 1.0, p.x)) * fade * fade * (0.2 + 0.28 * voice);

    // Glass over what is behind it; the aura only adds light.
    return vec4<f32>(col * inside + aura * (1.0 - inside), inside);
}

// ---------------------------------------------------------------- wave

// Where ribbon `k` is at `x` (0 → 1), from the middle, in shares of the half height.
// `lengths` scales how many waves fit across (1 for the wide sheet, fewer in the orb).
fn ribbon(x: f32, t: f32, k: i32, voice: f32, lengths: f32) -> f32 {
    let b = f32(k);
    let cycles = (1.25 + 0.42 * b) * lengths;
    let shift = 2.1 * b;
    let tau = 2.0 * PI;
    // The sea: waves of different lengths at different speeds, one against the others.
    let sea = 0.56 * sin(x * cycles * tau - 0.85 * t + shift)
        + 0.3 * sin(x * cycles * 2.3 * tau + 0.5 * t + 2.0 * shift)
        + 0.14 * sin(x * cycles * 3.7 * tau - 1.3 * t + 0.5 * shift);
    // A voice: swells that drift along, each rising and falling like something said.
    var loud = 0.3;
    for (var j = 0; j < 3; j++) {
        let f = f32(j);
        let centre = 0.5 + 0.4 * sin((0.21 + 0.07 * f) * t + 2.1 * f + shift);
        let d = (x - centre) / (0.16 / sqrt(lengths));
        loud += 2.1 * syllable(t, 2.8 + 0.8 * f, 1.7 * f + shift) * exp(-d * d);
    }
    let ends = pow(abs(sin(x * PI)), 1.4);
    return sea * ends * mix(0.62, min(loud, 2.3) * 0.72, voice);
}

fn wave(uv: vec2<f32>) -> vec4<f32> {
    let t = u.a.x;
    let voice = u.a.y;
    let surge = u.a.z;
    let y = uv.y * 2.0 - 1.0;
    // A pixel, in shares of the half height.
    let px = fwidth(y);
    let swell = (0.95 + 0.5 * surge);
    var lit = vec3<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let b = f32(k);
        var colour = tint(k);
        if (k == 3) { colour = EMBER; }
        let share = 1.0 - 0.13 * b;
        // Tall crests lean in toward the edge instead of being cut off flat.
        let at = 0.9 * tanh(ribbon(uv.x, t + 0.7 * b, k, voice, 1.0) * swell * share / 0.9 * 1.6);
        let d = abs(y - at);
        // A fine bright thread, the glow round it, and a breath of light down to the middle.
        let thread = exp(-pow(d / (1.1 * px), 2.0));
        let glow = exp(-d / (7.0 * px)) * 0.13;
        let between = step(min(at, 0.0), y) * step(y, max(at, 0.0));
        let body = between * (0.1 + 0.16 * abs(y) / max(abs(at), 0.001)) * smoothstep(0.0, 0.12, abs(at));
        lit += colour * (thread * 0.7 + glow + body) * (1.0 - 0.1 * b) * (0.35 + 0.65 * smoothstep(0.0, 0.25, abs(at)));
    }
    // Out of nothing at each end, and never a hard line at the top or bottom.
    let ends = smoothstep(0.0, 0.24, uv.x) * smoothstep(1.0, 0.76, uv.x);
    let edge = smoothstep(1.0, 0.8, abs(y));
    let col = (vec3<f32>(1.0) - exp(-lit * 1.15)) * ends * edge;
    // Only light: it adds to what is behind it.
    return vec4<f32>(col, 0.0);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    if (u.a.w < 0.5) {
        c = orb(in.uv);
    } else {
        c = wave(in.uv);
    }
    if (u.b.x > 0.5) {
        c = vec4<f32>(pow(c.rgb, vec3<f32>(2.2)), c.a);
    }
    return c;
}
