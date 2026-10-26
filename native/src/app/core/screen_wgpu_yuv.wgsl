// Screen viewer (SCR-M7, U25-5): planes of a decoded picture to RGB.
// Limited range; `params.coeff` holds the factors of the matrix
// (`coefficients` in screen_wgpu_yuv.rs); `params.mode.x` is 1 for I420.
struct Params {
    coeff: vec4<f32>,
    mode: vec4<f32>,
    view: vec4<f32>,
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

// Exact source-area overlap, not a fixed four-sample minifier. Enlargement
// uses Keys/Catmull-Rom cubic convolution (a=-0.5) in each YUV signal plane.
fn cubic(x0: f32) -> f32 {
    let x = abs(x0);
    if x < 1.0 { return 1.5*x*x*x - 2.5*x*x + 1.0; }
    if x < 2.0 { return -0.5*x*x*x + 2.5*x*x - 4.0*x + 2.0; }
    return 0.0;
}
fn reconstruct(tex: texture_2d<f32>, uv: vec2<f32>, uv_step: vec2<f32>) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(tex));
    let span = uv_step * dims;
    let center = uv * dims - vec2<f32>(0.5);
    let area = max(span, vec2<f32>(1.0));
    let lo = uv * dims - area * 0.5;
    let hi = uv * dims + area * 0.5;
    let reducing = span >= vec2<f32>(1.0);
    let start = vec2<i32>(select(floor(center)-vec2<f32>(1.0), floor(lo), reducing));
    let end = vec2<i32>(select(floor(center)+vec2<f32>(3.0), ceil(hi), reducing));
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var y = start.y; y < end.y; y += 1) {
        let wy = select(cubic(center.y-f32(y)),
            max(0.0,min(hi.y,f32(y+1))-max(lo.y,f32(y))), reducing.y);
        for (var x = start.x; x < end.x; x += 1) {
            let wx = select(cubic(center.x-f32(x)),
                max(0.0,min(hi.x,f32(x+1))-max(lo.x,f32(x))), reducing.x);
            let p = clamp(vec2<i32>(x,y),vec2<i32>(0),vec2<i32>(dims)-vec2<i32>(1));
            let weight = wx*wy;
            sum += textureLoad(tex,p,0)*weight;
            total += weight;
        }
    }
    return sum / max(total,0.00001);
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = params.view.xy + in.uv*params.view.zw;
    let step = abs(dpdx(uv)) + abs(dpdy(uv));
    let luma = (reconstruct(tex_y,uv,step).r - 16.0 / 255.0) * (255.0 / 219.0);
    let su = reconstruct(tex_u,uv,step);
    let sv = reconstruct(tex_v,uv,step);
    let u = su.r;
    let v = select(su.g,sv.r,params.mode.x > 0.5);
    let cb = (u - 128.0 / 255.0) * (255.0 / 224.0);
    let cr = (v - 128.0 / 255.0) * (255.0 / 224.0);
    let r = luma + params.coeff.x * cr;
    let g = luma - params.coeff.y * cb - params.coeff.z * cr;
    let b = luma + params.coeff.w * cb;
    return vec4<f32>(clamp(vec3<f32>(r,g,b),vec3<f32>(0.0),vec3<f32>(1.0)),1.0);
}
