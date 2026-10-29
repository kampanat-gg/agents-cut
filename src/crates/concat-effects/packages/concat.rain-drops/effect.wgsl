struct Params { amount: f32, speed: f32, angle: f32, length: f32 }

// Rain in three depths, each a grid of columns with at most one streak in
// each cell: its place, its phase and whether it falls at all hashed from
// the cell, so the pattern never repeats on screen and the same frame
// always draws the same rain. The far layer is finer, slower and dimmer.
fn streaks(uv: vec2<f32>, depth: f32, density: f32) -> f32 {
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let lean = radians(params.angle);
    // Slant the frame so the streaks fall along it.
    var p = vec2<f32>(uv.x * aspect + uv.y * tan(lean), uv.y);
    let columns = mix(18.0, 60.0, depth);
    let rows = mix(3.0, 7.0, depth);
    let fall = frame.time * (params.speed / 100.0) * mix(2.6, 1.4, depth);
    p = vec2<f32>(p.x * columns, (p.y - fall) * rows);
    let cell = floor(p);
    let local = fract(p);
    let pick = hash(cell, 11.0 + depth * 7.0);
    if (pick > density) {
        return 0.0;
    }
    let x = 0.2 + hash(cell, 3.0 + depth) * 0.6;
    let y = hash(cell, 5.0 + depth);
    let span = mix(0.15, 0.8, params.length / 100.0);
    let width = mix(0.06, 0.03, depth);
    let across = 1.0 - smoothstep(0.0, width, abs(local.x - x));
    let along = smoothstep(y, y + 0.05, local.y) * (1.0 - smoothstep(y + span - 0.1, y + span, local.y));
    return across * along * mix(0.9, 0.35, depth);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    if (amount <= 0.0) {
        return c;
    }
    var rain = 0.0;
    for (var i: i32 = 0; i < 3; i++) {
        let depth = f32(i) / 2.0;
        rain += streaks(uv, depth, amount * 0.55);
    }
    // Rain lifts the picture toward a cool grey where it falls, and dims
    // the rest a touch: a wet, overcast light.
    let tint = vec3<f32>(0.75, 0.82, 0.9);
    let base = c.rgb * (1.0 - 0.12 * amount);
    let out = base + tint * min(rain, 1.0) * 0.6;
    return vec4<f32>(out, c.a);
}
