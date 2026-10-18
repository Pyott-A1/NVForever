// Screen blur of image space modifiers (imagespace.rs): a Gaussian blur in two passes,
// across and then down, before the grade (docs/spec-notes/image-modifiers.md §5).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

// The grade pass's uniform (imagespace.wgsl); only the blur radius is read here.
struct Grade {
    tint: vec4<f32>,
    fade: vec4<f32>,
    radial: vec4<f32>,
    radial2: vec4<f32>,
    saturation: f32,
    contrast: f32,
    contrast_avg_lum: f32,
    brightness: f32,
    blur: f32,
    time: f32,
    sharp: f32,
    vats: vec4<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> grade: Grade;
#ifdef MULTISAMPLED
@group(0) @binding(3) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(3) var depth: texture_depth_2d;
#endif

// Whether the picture at `uv` is nearer than `grade.sharp` (reverse depth: larger is
// nearer), so neither blurred nor blurred into what is behind it.
fn near(uv: vec2<f32>) -> bool {
    if grade.sharp <= 0.0 {
        return false;
    }
    let dims = vec2<i32>(textureDimensions(depth));
    let p = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2(0), dims - 1);
    return textureLoad(depth, p, 0) > grade.sharp;
}

// A blur radius of 1 reaches this part of the screen height to each side [hypothesis:
// the wiki gives 0 as none and 7 as the most; image-modifiers.md §5].
const REACH_PER_RADIUS: f32 = 1.0 / 270.0;
const MAX_PAIRS: i32 = 48;

fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen));
    let centre = textureSampleLevel(screen, screen_sampler, uv, 0.0);
    // The reach is two standard deviations of the Gaussian.
    let sigma = 0.5 * grade.blur * dims.y * REACH_PER_RADIUS;
    if sigma < 0.25 || near(uv) {
        return centre;
    }
    let k = -0.5 / (sigma * sigma);
    let pairs = min(i32(ceil(1.25 * sigma)), MAX_PAIRS);
    var acc = centre.rgb;
    var total = 1.0;
    // Pixels 1 and 2, 3 and 4, ... to each side, each pair read as one filtered sample
    // between them.
    for (var p = 0; p < pairs; p++) {
        let a = f32(2 * p + 1);
        let wa = exp(k * a * a);
        let wb = exp(k * (a + 1.0) * (a + 1.0));
        let w = wa + wb;
        let off = dir * ((a * wa + (a + 1.0) * wb) / w) / dims;
        // What is near is left out of the blur behind it.
        let wp = select(w, 0.0, near(uv + off));
        let wm = select(w, 0.0, near(uv - off));
        acc += wp * textureSampleLevel(screen, screen_sampler, uv + off, 0.0).rgb + wm * textureSampleLevel(screen, screen_sampler, uv - off, 0.0).rgb;
        total += wp + wm;
    }
    return vec4(acc / total, centre.a);
}

@fragment
fn blur_across(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2(1.0, 0.0));
}

@fragment
fn blur_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2(0.0, 1.0));
}
