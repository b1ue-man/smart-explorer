// Screen viewer (SCR-M7, U25-5): planes of a decoded picture to RGB.
// Limited range; `params.coeff` holds the factors of the matrix
// (`coefficients` in screen_wgpu_yuv.rs); `params.mode.x` is 1 for I420.
struct Params {
    coeff: vec4<f32>,
    mode: vec4<f32>,
}
@group(0) @binding(0) var tex_y: texture_2d<f32>;
@group(0) @binding(1) var tex_u: texture_2d<f32>;
@group(0) @binding(2) var tex_v: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: VsOut;
    out.uv = vec2<f32>(x, y);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let luma = (textureSample(tex_y, samp, in.uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let su = textureSample(tex_u, samp, in.uv);
    let sv = textureSample(tex_v, samp, in.uv);
    let u = su.r;
    let v = select(su.g, sv.r, params.mode.x > 0.5);
    let cb = (u - 128.0 / 255.0) * (255.0 / 224.0);
    let cr = (v - 128.0 / 255.0) * (255.0 / 224.0);
    let r = luma + params.coeff.x * cr;
    let g = luma - params.coeff.y * cb - params.coeff.z * cr;
    let b = luma + params.coeff.w * cb;
    return vec4<f32>(clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
