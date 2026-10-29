struct Params { strength: f32, scale: f32, speed: f32, horizon: f32 }

// Moves where each pixel below the horizon reads from: slow horizontal
// sways and smaller vertical bobs, growing toward the bottom of the frame,
// where the water is nearest.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let strength = params.strength / 100.0;
    let horizon = params.horizon / 100.0;
    if (strength <= 0.0 || uv.y <= horizon) {
        return sample(uv);
    }
    let near = smoothstep(horizon, 1.0, uv.y);
    let t = frame.time * (params.speed / 100.0);
    let k = mix(60.0, 12.0, params.scale / 100.0);
    // Waves crowd together toward the horizon, as they do in perspective.
    let row = (uv.y - horizon) / max(1.0 - horizon, 0.001);
    let phase = k * sqrt(max(row, 0.0));
    let dx = sin(phase * 2.0 + uv.x * 6.0 + t * 1.3) * 0.006
        + sin(phase * 3.7 - uv.x * 9.0 + t * 0.8) * 0.003;
    let dy = sin(phase * 2.6 + t * 1.7 + uv.x * 3.0) * 0.004;
    let offset = vec2<f32>(dx, dy) * strength * (0.3 + near * 1.7);
    return sample(clamp(uv + offset, vec2<f32>(0.0), vec2<f32>(1.0)));
}
