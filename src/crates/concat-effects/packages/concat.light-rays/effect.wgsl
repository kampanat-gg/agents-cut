struct Params { amount: f32, angle: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    if (amount <= 0.0) {
        return c;
    }
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let lean = radians(params.angle);
    // Distance across the rays, measured along the slant from the top.
    let across = uv.x * aspect + uv.y * tan(lean);
    let t = frame.time * (params.speed / 100.0);
    var rays = 0.0;
    rays += pow(0.5 + 0.5 * sin(across * 9.0 + t * 0.7), 6.0);
    rays += pow(0.5 + 0.5 * sin(across * 15.0 - t * 0.9 + 1.7), 8.0) * 0.7;
    rays += pow(0.5 + 0.5 * sin(across * 23.0 + t * 1.3 + 4.1), 10.0) * 0.5;
    // Strongest near the top, gone by the bottom.
    let fade = 1.0 - smoothstep(0.0, 0.95, uv.y);
    let light = rays * fade * amount * 0.45;
    let out = c.rgb + vec3<f32>(1.0, 0.95, 0.82) * light;
    return vec4<f32>(out, c.a);
}
