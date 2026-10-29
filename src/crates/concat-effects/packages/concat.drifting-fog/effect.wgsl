struct Params { amount: f32, scale: f32, speed: f32, height: f32 }

// Smooth value noise from the shared hash, and four octaves of it.
fn soft_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i, 17.0);
    let b = hash(i + vec2<f32>(1.0, 0.0), 17.0);
    let c = hash(i + vec2<f32>(0.0, 1.0), 17.0);
    let d = hash(i + vec2<f32>(1.0, 1.0), 17.0);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn clouds(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var weight = 0.5;
    var q = p;
    for (var i: i32 = 0; i < 4; i++) {
        sum += soft_noise(q) * weight;
        q = q * 2.03 + vec2<f32>(3.1, 1.7);
        weight *= 0.5;
    }
    return sum;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    if (amount <= 0.0) {
        return c;
    }
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let scale = mix(1.5, 6.0, 1.0 - params.scale / 100.0);
    let t = frame.time * (params.speed / 100.0) * 0.05;
    let p = vec2<f32>(uv.x * aspect * scale + t * scale, uv.y * scale * 1.6);
    let density = smoothstep(0.35, 0.75, clouds(p) + clouds(p * 0.5 - vec2<f32>(t, 0.0)) * 0.5 - 0.25);
    // Thicker toward the bottom for a negative height, the top for a
    // positive one, even across the frame at zero.
    let h = params.height / 100.0;
    let bias = clamp(mix(1.0, select(uv.y, 1.0 - uv.y, h > 0.0) * 1.6, abs(h)), 0.0, 1.0);
    let fog = density * bias * amount;
    let out = mix(c.rgb, vec3<f32>(0.85, 0.87, 0.9), fog * 0.85);
    return vec4<f32>(out, c.a);
}
