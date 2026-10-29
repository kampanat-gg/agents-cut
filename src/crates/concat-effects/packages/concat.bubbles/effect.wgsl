struct Params { amount: f32, size: f32, speed: f32 }

// In each cell of a grid at most one bubble, rising and wobbling, drawn as
// a thin rim with a small highlight up and to the left.
fn bubble(uv: vec2<f32>, depth: f32, density: f32) -> f32 {
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let cells = mix(5.0, 12.0, depth);
    let rise = frame.time * (params.speed / 100.0) * mix(0.18, 0.08, depth);
    var p = vec2<f32>(uv.x * aspect, uv.y + rise) * cells;
    let cell = floor(p);
    if (hash(cell, 19.0 + depth * 3.0) > density) {
        return 0.0;
    }
    let wobble = sin(frame.time * 2.0 + hash(cell, 6.0) * 6.28) * 0.08;
    let centre = vec2<f32>(0.3 + hash(cell, 8.0) * 0.4 + wobble, 0.3 + hash(cell, 9.0) * 0.4);
    let radius = mix(0.08, 0.3, params.size / 100.0) * (0.5 + hash(cell, 10.0) * 0.5);
    let d = length(fract(p) - centre);
    let rim = smoothstep(radius * 0.75, radius * 0.92, d) * (1.0 - smoothstep(radius * 0.92, radius, d));
    let glint = 1.0 - smoothstep(0.0, radius * 0.18, length(fract(p) - centre + vec2<f32>(radius * 0.35, radius * 0.35)));
    return (rim * 0.8 + glint) * mix(1.0, 0.5, depth);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    if (amount <= 0.0) {
        return c;
    }
    var light = 0.0;
    for (var i: i32 = 0; i < 2; i++) {
        light += bubble(uv, f32(i), amount * 0.6);
    }
    let out = c.rgb + vec3<f32>(0.8, 0.95, 1.0) * min(light, 1.0) * 0.7;
    return vec4<f32>(out, c.a);
}
