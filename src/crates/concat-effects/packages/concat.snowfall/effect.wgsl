struct Params { amount: f32, size: f32, speed: f32, wind: f32 }

// Snow in three depths: in each cell of a grid, at most one soft flake,
// drifting down and swaying, its place and phase hashed from the cell.
fn flakes(uv: vec2<f32>, depth: f32, density: f32) -> f32 {
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let cells = mix(6.0, 16.0, depth);
    let fall = frame.time * (params.speed / 100.0) * mix(0.16, 0.07, depth);
    let drift = frame.time * (params.wind / 100.0) * mix(0.12, 0.05, depth);
    var p = vec2<f32>(uv.x * aspect - drift, uv.y - fall) * cells;
    let cell = floor(p);
    if (hash(cell, 13.0 + depth * 5.0) > density) {
        return 0.0;
    }
    let sway = sin(frame.time * (0.8 + hash(cell, 2.0)) + hash(cell, 4.0) * 6.28) * 0.15;
    let centre = vec2<f32>(0.25 + hash(cell, 3.0 + depth) * 0.5 + sway, 0.25 + hash(cell, 7.0 + depth) * 0.5);
    let radius = mix(0.05, 0.16, params.size / 100.0) * mix(1.0, 0.55, depth);
    let d = length(fract(p) - centre);
    return (1.0 - smoothstep(radius * 0.4, radius, d)) * mix(1.0, 0.5, depth);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    if (amount <= 0.0) {
        return c;
    }
    var snow = 0.0;
    for (var i: i32 = 0; i < 3; i++) {
        snow += flakes(uv, f32(i) / 2.0, amount * 0.7);
    }
    let out = mix(c.rgb, vec3<f32>(0.95, 0.97, 1.0), min(snow, 1.0) * 0.9);
    return vec4<f32>(out, c.a);
}
