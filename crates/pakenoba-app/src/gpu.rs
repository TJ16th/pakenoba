//! wgpu での箱の描画。
//!
//! egui の画面とは別のテクスチャに描き（4 倍のマルチサンプルと奥行きバッファつき）、
//! 結果を egui に画像として渡す。あとで動画書き出しに転用しやすい形。

use pakenoba_i18n::t;
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use pakenoba_core::material::{Coat, Foil, Material, Paper};
use pakenoba_core::math::Mat4;
use pakenoba_core::template::Vertex;

use crate::env::{self, Prepared};

// egui は「ガンマ補正済みの値がそのまま入ったテクスチャ」を前提にしているので、sRGB 形式にはしない。
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// 立体の表示の地の色（PhotoCraft の絵を置く場所と同じ #282828）。
const BACKGROUND: wgpu::Color = wgpu::Color { r: 0.157, g: 0.157, b: 0.157, a: 1.0 };

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    // 画面の点から、箱の座標の視線を出すための逆行列。
    inv_view_proj: mat4x4<f32>,
    // x, y: 景色の回し（cos, sin）。z: 明るさの倍率。w: 景色のいちばん粗い段階の番号。
    env: vec4<f32>,
    // 抜き穴。xy が中心、zw が半分の幅と高さ（どちらも絵柄の座標）。z が 0 なら穴なし。
    holes: array<vec4<f32>, 2>,
    // 拡散の光（球面調和の 9 個の係数）。
    sh: array<vec4<f32>, 9>,
    // 紙の表、裏、切り口の色（線形）と粗さ。
    paper_front: vec4<f32>,
    paper_back: vec4<f32>,
    paper_edge: vec4<f32>,
    // コート: 強さ、粗さ、色の深まり（グロス PP で色が濃く見える）、使わない。
    coat: vec4<f32>,
    // 紙のきめ: 繊維の大きさ（mm）、表と裏の凹凸の高さ（mm）、色のむら。
    grain: vec4<f32>,
    // 展開図の大きさ（mm）と、段ボールの段の間隔（mm）と高さ（mm）。
    sheet: vec4<f32>,
    // 箔の種類ごとの、映り込みの色（線形）と粗さ。並びは Foil::ALL と同じ。
    foils: array<vec4<f32>, 8>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var art: texture_2d<f32>;
@group(1) @binding(1) var art_sampler: sampler;
@group(1) @binding(2) var back_art: texture_2d<f32>;
// 加工の版（表と裏）。R が箔の量、G が箔の種類（番号 × 32 + 16）。
@group(1) @binding(3) var finish_front: texture_2d<f32>;
@group(1) @binding(4) var finish_back: texture_2d<f32>;
@group(2) @binding(0) var env_map: texture_2d<f32>;
@group(2) @binding(1) var env_sampler: sampler;

const PI: f32 = 3.14159265;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) kind: f32,
    @location(3) clip: vec4<f32>,
    @location(4) world: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) kind: f32) -> VertexOut {
    var out: VertexOut;
    out.position = globals.view_proj * vec4<f32>(pos, 1.0);
    out.clip = out.position;
    out.world = pos;
    out.normal = normal;
    out.uv = uv;
    out.kind = kind;
    return out;
}

// 箱の座標の向きを、景色の座標へ（視点といっしょに、上下の軸のまわりに回す）。
fn to_env(d: vec3<f32>) -> vec3<f32> {
    let c = globals.env.x;
    let s = globals.env.y;
    return vec3<f32>(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
}

// 景色の、向き d から来る光。粗さに合わせて、ぼかした段階から引く。
fn radiance(d: vec3<f32>, rough: f32) -> vec3<f32> {
    let e = to_env(d);
    let uv = vec2<f32>(atan2(e.x, e.z) / (2.0 * PI) + 0.5, acos(clamp(e.y, -1.0, 1.0)) / PI);
    return textureSampleLevel(env_map, env_sampler, uv, rough * globals.env.w).rgb;
}

// 向き n の面が受ける拡散の光（白い紙の明るさ）。
fn irradiance(n0: vec3<f32>) -> vec3<f32> {
    let n = to_env(n0);
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = globals.sh[0].rgb;
    e += globals.sh[1].rgb * y + globals.sh[2].rgb * z + globals.sh[3].rgb * x;
    e += globals.sh[4].rgb * (x * y) + globals.sh[5].rgb * (y * z) + globals.sh[6].rgb * (3.0 * z * z - 1.0);
    e += globals.sh[7].rgb * (x * z) + globals.sh[8].rgb * (x * x - y * y);
    return max(e, vec3<f32>(0.0)) / PI;
}

// 映り込みの強さ（反射率 f0、粗さ、視線と面の角度から）。Karis の近似。
fn env_brdf(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// 明るすぎる所だけを丸める（Khronos PBR Neutral）。0.76 までの色は、そのまま。
fn neutral(c0: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desat = 0.15;
    let x = min(c0.r, min(c0.g, c0.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = c0 - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// 紙のきめ。値の雑音（0〜1）を、大きさを変えて重ねる。繊維の向きに少し伸ばす。
fn hash2(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 紙の表面の高さ（0〜1 くらい）。`p` は mm を繊維の大きさで割ったもの。
fn fibers(p: vec2<f32>) -> f32 {
    var v = value_noise(p) * 0.45;
    v += value_noise(p * 2.07 + vec2<f32>(17.0, 3.0)) * 0.3;
    v += value_noise(vec2<f32>(p.x * 4.3, p.y * 1.4) + vec2<f32>(31.0, 11.0)) * 0.25;
    return v;
}

// 押した箔の、小さなうねりの高さ（mm）。
fn foil_height(mm: vec2<f32>) -> f32 {
    // 格子の目が見えないよう、向きを変えた 2 つを重ねる。
    let q = vec2<f32>(mm.x * 0.8 - mm.y * 0.6, mm.x * 0.6 + mm.y * 0.8);
    return value_noise(mm / 3.4) * 0.03 + value_noise(q / 2.3 + vec2<f32>(5.0, 1.0)) * 0.025;
}

// 紙の高さ（mm）。繊維と、段ボールの段。
fn paper_height(mm: vec2<f32>, amp: f32) -> f32 {
    var h = fibers(mm / globals.grain.x) * amp;
    if globals.sheet.z > 0.0 {
        h += (0.5 + 0.5 * cos(mm.x / globals.sheet.z * 2.0 * PI)) * globals.sheet.w;
    }
    return h;
}

// 刷った色は、紙の色と掛け合わさる（インキは透ける）。白い紙ではほぼそのまま、クラフト紙では暗く茶色っぽくなる。
fn ink_on(paper: vec3<f32>) -> vec3<f32> {
    return min(paper / 0.88, vec3<f32>(1.0));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // 紙のきめのための、画面の 1 画素あたりの変化（分岐の前に求める）。
    let mm = in.uv * globals.sheet.xy;
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let dmx = dpdx(mm);
    let dmy = dpdy(mm);
    // 抜き穴の中は描かない。角の丸い四角として判定する。
    for (var i = 0; i < 2; i++) {
        let hole = globals.holes[i];
        if hole.z > 0.0 {
            let q = abs(in.uv - hole.xy) / hole.zw;
            if q.x * q.x * q.x * q.x + q.y * q.y * q.y * q.y < 1.0 {
                discard;
            }
        }
    }
    // 視線（画面の点を、近い所と遠い所へ戻して結ぶ。透視でも平行投影でも同じ式）。
    let ndc = in.clip.xy / in.clip.w;
    let near = globals.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let far = globals.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let v = normalize(near.xyz / near.w - far.xyz / far.w);
    var n0 = normalize(in.normal);
    if dot(n0, v) < 0.0 {
        n0 = -n0;
    }
    // 紙のきめで、面の向きを少しずらす（コートの膜は平らなので、膜の映り込みには使わない）。
    // 1 画素が繊維より大きく見えるほど遠いときは、ちらつかないよう、きめを薄くする。
    var amp = globals.grain.y;
    if in.kind > 0.5 && in.kind < 1.5 {
        amp = globals.grain.z;
    }
    var n = n0;
    var grain = 0.5;

    let footprint = max(length(dmx), length(dmy));
    let det = dmx.x * dmy.y - dmy.x * dmx.y;
    var t_mm = vec3<f32>(0.0);
    var b_mm = vec3<f32>(0.0);
    if abs(det) > 1e-12 {
        t_mm = normalize((dpx * dmy.y - dpy * dmx.y) / det);
        b_mm = normalize((dpy * dmx.x - dpx * dmy.x) / det);
    }
    let fade = 1.0 - smoothstep(0.35, 1.2, footprint / globals.grain.x);
    if in.kind < 1.5 && abs(det) > 1e-12 && fade > 0.0 {
        let e = globals.grain.x * 0.25;
        let gx = (paper_height(mm + vec2<f32>(e, 0.0), amp) - paper_height(mm - vec2<f32>(e, 0.0), amp)) / (2.0 * e);
        let gy = (paper_height(mm + vec2<f32>(0.0, e), amp) - paper_height(mm - vec2<f32>(0.0, e), amp)) / (2.0 * e);
        n = normalize(n0 - (gx * t_mm + gy * b_mm) * fade);
        if dot(n, v) < 0.02 {
            n = n0;
        }
        grain = mix(0.5, fibers(mm / globals.grain.x), fade);
    }
    let nv = max(dot(n, v), 1e-4);
    let r = reflect(-v, n);
    let nv0 = max(dot(n0, v), 1e-4);
    let r0 = reflect(-v, n0);

    var paper = globals.paper_front;
    var base: vec3<f32>;
    var shade = 1.0;
    if in.kind > 1.5 {
        paper = globals.paper_edge;
        base = paper.rgb;
    } else if in.kind > 0.5 {
        // 紙の裏（箱の内側の面）。裏に置いた柄が見える。箱の中は、外より暗い。
        paper = globals.paper_back;
        let back = textureSample(back_art, art_sampler, in.uv);
        base = mix(paper.rgb, back.rgb * ink_on(paper.rgb), back.a);
        shade = 0.7;
    } else {
        let texel = textureSample(art, art_sampler, in.uv);
        base = mix(paper.rgb, texel.rgb * ink_on(paper.rgb), texel.a);
        // コートで色が少し深く（グロス）、または少し白っぽく（マット）見える。
        base = pow(max(base, vec3<f32>(0.0)), vec3<f32>(1.0 + globals.coat.z));
    }
    // 繊維のむら: 色を少し変え、粗さも場所ごとに少し変える。
    // 大きなむら（数 mm の雲のような濃淡）は、遠くからも見える。細かいむらは、近くでだけ。
    let cloud_fade = 1.0 - smoothstep(0.6, 3.0, footprint / 4.0);
    let cloud = mix(0.5, value_noise(mm / 4.0) * 0.6 + value_noise(mm / 1.7 + vec2<f32>(7.0, 5.0)) * 0.4, cloud_fade);
    if in.kind < 1.5 {
        base = base * (1.0 + globals.grain.w * ((grain - 0.5) + 1.2 * (cloud - 0.5)));
    }
    let rough = clamp(paper.a + 0.12 * (grain - 0.5), 0.02, 1.0);
    let f_paper = env_brdf(vec3<f32>(0.04), rough, nv);
    var color = base * irradiance(n) * shade * (1.0 - f_paper) + radiance(r, rough) * f_paper * shade;
    // コート（表だけ）。紙の上の透明な膜の映り込み。膜で返したぶん、下の紙は暗くなる。
    if in.kind < 0.5 && globals.coat.x > 0.0 {
        let fc = env_brdf(vec3<f32>(0.04), globals.coat.y, nv0) * globals.coat.x;
        color = color * (1.0 - fc) + radiance(r0, globals.coat.y) * fc;
    }
    // 箔（いちばん上。金属として、景色を箔の色で映す。拡散の色はほとんどない）。
    var foil = vec4<f32>(0.0);
    if in.kind < 0.5 {
        foil = textureSample(finish_front, art_sampler, in.uv);
    } else if in.kind < 1.5 {
        foil = textureSample(finish_back, art_sampler, in.uv);
    }
    if foil.r > 0.003 {
        let kind = u32(clamp(floor(foil.g * 255.0 / 32.0), 0.0, 7.0));
        let spec = globals.foils[kind];
        // 箔は紙に押し付けるので、紙のきめを少し拾い、押したときの小さなうねりも持つ。
        // うねりで映る景色がずれて、明るい所と暗い所ができる（箔らしいきらめき）。
        var nf = normalize(mix(n0, n, 0.35));
        if dot(t_mm, t_mm) > 0.5 {
            let e = 0.4;
            let hx = foil_height(mm + vec2<f32>(e, 0.0)) - foil_height(mm - vec2<f32>(e, 0.0));
            let hy = foil_height(mm + vec2<f32>(0.0, e)) - foil_height(mm - vec2<f32>(0.0, e));
            // うねりの強さは、加工ごとに B に入れてある（0〜1。0.5 で、はじめに決めた強さの 1 倍）。
            nf = normalize(nf - (hx * t_mm + hy * b_mm) / (2.0 * e) * foil.b * 2.0);
        }
        let nvf = max(dot(nf, v), 1e-4);
        let rf = reflect(-v, nf);
        var f0 = spec.rgb;
        if kind == 6u {
            // ホログラム: 見る向きと場所で、虹の色が流れる。
            let phase = dot(rf, vec3<f32>(0.9, 1.3, 0.7)) * 3.0 + (mm.x * 0.7 + mm.y * 0.3) * 0.03 + value_noise(mm / 0.8) * 0.12;
            let rainbow = 0.5 + 0.5 * cos(2.0 * PI * (phase + vec3<f32>(0.0, 0.33, 0.67)));
            f0 = mix(f0, rainbow, 0.65);
        }
        // 箔の細かい凹凸で散る光（どの向きから見ても、箔の色が沈みすぎないように）。
        let metal = radiance(rf, spec.a) * env_brdf(f0, spec.a, nvf) + f0 * irradiance(nf) * 0.22;
        color = mix(color, metal, foil.r);
    }
    color = color * globals.env.z;
    return vec4<f32>(to_srgb(clamp(neutral(color), vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: Mat4,
    inv_view_proj: Mat4,
    env: [f32; 4],
    holes: [[f32; 4]; 2],
    sh: [[f32; 4]; 9],
    paper_front: [f32; 4],
    paper_back: [f32; 4],
    paper_edge: [f32; 4],
    coat: [f32; 4],
    grain: [f32; 4],
    sheet: [f32; 4],
    foils: [[f32; 4]; 8],
}

/// 箱の見た目のうち、視点のほかに描くたびに渡すもの。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub material: Material,
    /// 景色の回し（ラジアン）。視点の左右の向きに、使う人が決めた回しを足したもの。
    pub turn: f32,
    /// 明るさの倍率。
    pub exposure: f32,
    /// 展開図の大きさ（mm）。紙のきめの大きさを mm で合わせるため。
    pub sheet: [f32; 2],
}

fn linear(srgb: [u8; 3]) -> [f32; 3] {
    srgb.map(|c| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
}

fn with(rgb: [f32; 3], a: f32) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], a]
}

/// 紙の表、裏、切り口の、色と粗さ。
fn paper_values(paper: Paper) -> [[f32; 4]; 3] {
    let (front, back, edge) = match paper {
        Paper::CoatedBoard => (([244, 244, 240], 0.42), ([178, 176, 171], 0.8), ([196, 192, 184], 0.9)),
        Paper::Coated => (([246, 246, 244], 0.35), ([246, 246, 244], 0.35), ([236, 234, 229], 0.85)),
        Paper::MatteCoated => (([243, 243, 240], 0.6), ([243, 243, 240], 0.6), ([234, 232, 226], 0.85)),
        Paper::Uncoated => (([242, 240, 233], 0.85), ([242, 240, 233], 0.85), ([232, 229, 220], 0.9)),
        Paper::Kraft => (([176, 134, 92], 0.8), ([168, 127, 87], 0.85), ([150, 112, 78], 0.9)),
        Paper::Corrugated => (([184, 146, 104], 0.85), ([170, 135, 96], 0.88), ([160, 125, 88], 0.95)),
    };
    [with(linear(front.0), front.1), with(linear(back.0), back.1), with(linear(edge.0), edge.1)]
}

/// 紙のきめ: 繊維の大きさ（mm）、表と裏の凹凸の高さ（mm）、色のむら。段ボールの段の間隔と高さ（mm）。
fn grain_values(paper: Paper) -> ([f32; 4], [f32; 2]) {
    match paper {
        Paper::CoatedBoard => ([0.18, 0.004, 0.02, 0.02], [0.0, 0.0]),
        Paper::Coated => ([0.15, 0.003, 0.003, 0.01], [0.0, 0.0]),
        Paper::MatteCoated => ([0.15, 0.006, 0.006, 0.015], [0.0, 0.0]),
        Paper::Uncoated => ([0.25, 0.015, 0.015, 0.03], [0.0, 0.0]),
        Paper::Kraft => ([0.35, 0.03, 0.035, 0.12], [0.0, 0.0]),
        Paper::Corrugated => ([0.35, 0.03, 0.035, 0.12], [4.0, 0.03]),
    }
}

/// 箔の種類ごとの、映り込みの色（線形の反射率）と粗さ。並びは `Foil::ALL` と同じ。ホログラムは銀を土台に、描くときに虹色を足す。
fn foil_values() -> [[f32; 4]; 8] {
    let mut out = [[0.0f32; 4]; 8];
    for (slot, foil) in out.iter_mut().zip(Foil::ALL) {
        *slot = match foil {
            Foil::Gold => [1.0, 0.71, 0.29, 0.18],
            Foil::Silver => [0.95, 0.95, 0.96, 0.16],
            Foil::Copper => [0.95, 0.56, 0.40, 0.18],
            Foil::RedGold => [1.0, 0.62, 0.30, 0.18],
            Foil::MatteGold => [1.0, 0.71, 0.29, 0.42],
            Foil::MatteSilver => [0.92, 0.92, 0.93, 0.42],
            Foil::Hologram => [0.92, 0.92, 0.94, 0.06],
        };
    }
    out
}

/// コートの、強さ、粗さ、色の深まり。
fn coat_values(coat: Coat) -> [f32; 4] {
    match coat {
        Coat::None => [0.0, 0.5, 0.0, 0.0],
        Coat::MatteVarnish => [0.5, 0.5, 0.0, 0.0],
        Coat::GlossVarnish => [0.8, 0.12, 0.05, 0.0],
        Coat::MattePp => [1.0, 0.55, -0.03, 0.0],
        Coat::GlossPp => [1.0, 0.04, 0.10, 0.0],
        Coat::SoftTouchPp => [0.7, 0.78, -0.05, 0.0],
    }
}

/// 4x4 の逆行列（列優先）。逆にできなければ単位行列。
fn invert(m: &Mat4) -> Mat4 {
    let a: Vec<f32> = m.iter().flatten().copied().collect();
    let mut inv = [0.0f32; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15] + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15] - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15] + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14] - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15] - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15] + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15] - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14] + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15] + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15] - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15] + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14] - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11] - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11] + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11] - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10] + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det.abs() < 1e-20 {
        return [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    }
    let k = 1.0 / det;
    let mut out = [[0.0f32; 4]; 4];
    for (i, v) in inv.iter().enumerate() {
        out[i / 4][i % 4] = v * k;
    }
    out
}

struct Target {
    size: [u32; 2],
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
}

pub struct BoxRenderer {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    art_layout: wgpu::BindGroupLayout,
    art_bind: wgpu::BindGroup,
    /// 紙の表の絵と、裏の絵。
    front: wgpu::TextureView,
    back: wgpu::TextureView,
    /// 加工の版の絵（表と裏）。R が箔の量、G が箔の種類。
    finish: [wgpu::TextureView; 2],
    sampler: wgpu::Sampler,
    env_layout: wgpu::BindGroupLayout,
    env_bind: wgpu::BindGroup,
    env_sampler: wgpu::Sampler,
    /// 景色の拡散の光と、いちばん粗い段階の番号。
    sh: [[f32; 4]; 9],
    env_levels: f32,
    vertex_buf: wgpu::Buffer,
    vertex_capacity: usize,
    target: Option<Target>,
    egui_id: Option<egui::TextureId>,
}

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("box vertices"),
        size: (capacity * std::mem::size_of::<Vertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl BoxRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let device = &rs.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("box shader"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("box globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let art_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("box art"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let env_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("box environment"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("box layout"), bind_group_layouts: &[Some(&globals_layout), Some(&art_layout), Some(&env_layout)], immediate_size: 0 });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("box pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            // 紙は表も裏も見えるので、裏面を捨てない。
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("box globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("box globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("box art"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let front = Self::upload(rs, 1, 1, &[255, 255, 255, 0]);
        let back = Self::upload(rs, 1, 1, &[255, 255, 255, 0]);
        let finish = [Self::upload_plain(rs, 1, 1, &[0, 0, 0, 0]), Self::upload_plain(rs, 1, 1, &[0, 0, 0, 0])];
        let art_bind = Self::art_bind_group(rs, &art_layout, &sampler, &[&front, &back, &finish[0], &finish[1]]);
        // 景色は、左右はつながっているので繰り返し、上下は端で止める。
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("box environment"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // 景色を用意できるまでは、まわりが一様に明るい景色。
        let plain = env::prepare(&env::Equirect { w: 32, h: 16, px: vec![[1.0; 3]; 32 * 16] });
        let env_view = Self::upload_env(rs, &plain);
        let env_bind = Self::env_bind_group(rs, &env_layout, &env_sampler, &env_view);
        let vertex_capacity = 4096;
        BoxRenderer {
            pipeline,
            globals_buf,
            globals_bind,
            art_layout,
            art_bind,
            front,
            back,
            finish,
            sampler,
            env_layout,
            env_bind,
            env_sampler,
            sh: plain.sh.map(|c| [c[0], c[1], c[2], 0.0]),
            env_levels: (plain.mips.len() - 1) as f32,
            vertex_buf: vertex_buffer(device, vertex_capacity),
            vertex_capacity,
            target: None,
            egui_id: None,
        }
    }

    /// 画像を GPU に送る。
    fn upload(rs: &egui_wgpu::RenderState, w: u32, h: u32, rgba: &[u8]) -> wgpu::TextureView {
        // 絵柄は sRGB として読ませる（シェーダーには線形の値が届く）。
        let tex = texture(&rs.device, "box art", [w, h], wgpu::TextureFormat::Rgba8UnormSrgb, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// 景色の段階を、半精度の数のテクスチャにして送る。
    fn upload_env(rs: &egui_wgpu::RenderState, prepared: &Prepared) -> wgpu::TextureView {
        let top = &prepared.mips[0];
        let tex = rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("box environment"),
            size: wgpu::Extent3d { width: top.w as u32, height: top.h as u32, depth_or_array_layers: 1 },
            mip_level_count: prepared.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in prepared.mips.iter().enumerate() {
            let mut bytes = Vec::with_capacity(mip.w * mip.h * 8);
            for p in &mip.px {
                for v in [p[0], p[1], p[2], 1.0] {
                    bytes.extend_from_slice(&half::f16::from_f32(v).to_le_bytes());
                }
            }
            rs.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * mip.w as u32), rows_per_image: Some(mip.h as u32) },
                wgpu::Extent3d { width: mip.w as u32, height: mip.h as u32, depth_or_array_layers: 1 },
            );
        }
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn env_bind_group(rs: &egui_wgpu::RenderState, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, view: &wgpu::TextureView) -> wgpu::BindGroup {
        rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("box environment"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }],
        })
    }

    /// 照らす景色を差し替える。
    pub fn set_environment(&mut self, rs: &egui_wgpu::RenderState, prepared: &Prepared) {
        let view = Self::upload_env(rs, prepared);
        self.env_bind = Self::env_bind_group(rs, &self.env_layout, &self.env_sampler, &view);
        self.sh = prepared.sh.map(|c| [c[0], c[1], c[2], 0.0]);
        self.env_levels = (prepared.mips.len() - 1) as f32;
    }

    fn globals(&self, view_proj: Mat4, look: &Look, holes: [[f32; 4]; 2]) -> Globals {
        let [paper_front, paper_back, paper_edge] = paper_values(look.material.paper);
        let (grain, flute) = grain_values(look.material.paper);
        Globals {
            view_proj,
            inv_view_proj: invert(&view_proj),
            env: [look.turn.cos(), look.turn.sin(), look.exposure, self.env_levels],
            holes,
            sh: self.sh,
            paper_front,
            paper_back,
            paper_edge,
            coat: coat_values(look.material.coat),
            grain,
            sheet: [look.sheet[0].max(1.0), look.sheet[1].max(1.0), flute[0], flute[1]],
            foils: foil_values(),
        }
    }

    /// 色ではない値の絵（加工の版）を、そのままの数で送る。
    fn upload_plain(rs: &egui_wgpu::RenderState, w: u32, h: u32, rgba: &[u8]) -> wgpu::TextureView {
        let tex = texture(&rs.device, "box finish", [w, h], wgpu::TextureFormat::Rgba8Unorm, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// 加工の版の絵を差し替える（None なら加工なし）。
    pub fn set_finish(&mut self, rs: &egui_wgpu::RenderState, back: bool, w: usize, h: usize, rgba: Option<&[u8]>) {
        let view = match rgba {
            Some(rgba) if w > 0 && h > 0 && rgba.len() == w * h * 4 && w <= rs.device.limits().max_texture_dimension_2d as usize && h <= rs.device.limits().max_texture_dimension_2d as usize => Self::upload_plain(rs, w as u32, h as u32, rgba),
            _ => Self::upload_plain(rs, 1, 1, &[0, 0, 0, 0]),
        };
        self.finish[back as usize] = view;
        self.art_bind = Self::art_bind_group(rs, &self.art_layout, &self.sampler, &[&self.front, &self.back, &self.finish[0], &self.finish[1]]);
    }

    fn art_bind_group(rs: &egui_wgpu::RenderState, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, views: &[&wgpu::TextureView; 4]) -> wgpu::BindGroup {
        let [front, back, finish_front, finish_back] = *views;
        rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("box art"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(front) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(back) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(finish_front) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(finish_back) },
            ],
        })
    }

    /// 箱に貼る画像を差し替える。`back` が真なら、紙の裏（箱の内側の面）の画像。
    pub fn set_art(&mut self, rs: &egui_wgpu::RenderState, back: bool, w: usize, h: usize, rgba: &[u8]) {
        let max = rs.device.limits().max_texture_dimension_2d as usize;
        if w == 0 || h == 0 || w > max || h > max || rgba.len() != w * h * 4 {
            return;
        }
        let view = Self::upload(rs, w as u32, h as u32, rgba);
        if back {
            self.back = view;
        } else {
            self.front = view;
        }
        self.art_bind = Self::art_bind_group(rs, &self.art_layout, &self.sampler, &[&self.front, &self.back, &self.finish[0], &self.finish[1]]);
    }

    fn ensure_target(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture(device, "box msaa", size, COLOR_FORMAT, SAMPLES, attach);
        let depth = texture(device, "box depth", size, DEPTH_FORMAT, SAMPLES, attach);
        let resolve = texture(device, "box resolve", size, COLOR_FORMAT, 1, attach | wgpu::TextureUsages::TEXTURE_BINDING);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let target = Target { size, msaa_view: view(&msaa), depth_view: view(&depth), resolve_view: view(&resolve) };
        let mut renderer = rs.renderer.write();
        match self.egui_id {
            Some(id) => renderer.update_egui_texture_from_wgpu_texture(device, &target.resolve_view, wgpu::FilterMode::Linear, id),
            None => self.egui_id = Some(renderer.register_native_texture(device, &target.resolve_view, wgpu::FilterMode::Linear)),
        }
        self.target = Some(target);
    }

    /// 箱を描き、egui で表示できる画像の ID を返す。
    pub fn render(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2], vertices: &[Vertex], view_proj: Mat4, look: &Look, holes: [[f32; 4]; 2]) -> Option<egui::TextureId> {
        let max = rs.device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(2, max), size[1].clamp(2, max)];
        self.ensure_target(rs, size);
        if vertices.len() > self.vertex_capacity {
            self.vertex_capacity = vertices.len().next_power_of_two();
            self.vertex_buf = vertex_buffer(&rs.device, self.vertex_capacity);
        }
        rs.queue.write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(vertices));
        rs.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&self.globals(view_proj, look, holes)));

        let target = self.target.as_ref()?;
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("box") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("box"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa_view,
                    resolve_target: Some(&target.resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(BACKGROUND), store: wgpu::StoreOp::Discard },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_bind_group(1, &self.art_bind, &[]);
            pass.set_bind_group(2, &self.env_bind, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..std::mem::size_of_val(vertices) as u64));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        rs.queue.submit(Some(encoder.finish()));
        self.egui_id
    }

    /// 箱を、画面とは別の画像に描いて、RGBA（アルファは乗算なし）で読み戻す。画像や動画に書き出すとき用。
    /// `background` のアルファを 0 にすると、背景は透明になる。
    #[allow(clippy::too_many_arguments)]
    pub fn capture(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2], vertices: &[Vertex], view_proj: Mat4, look: &Look, holes: [[f32; 4]; 2], background: [f32; 4]) -> Result<Vec<u8>, String> {
        let max = rs.device.limits().max_texture_dimension_2d;
        if size[0] < 2 || size[1] < 2 || size[0] > max || size[1] > max {
            return Err(t!("This PC cannot draw a {} x {} px image (up to {max} px)", size[0], size[1]));
        }
        let device = &rs.device;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture(device, "capture msaa", size, COLOR_FORMAT, SAMPLES, attach);
        let depth = texture(device, "capture depth", size, DEPTH_FORMAT, SAMPLES, attach);
        let resolve = texture(device, "capture resolve", size, COLOR_FORMAT, 1, attach | wgpu::TextureUsages::COPY_SRC);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        if vertices.len() > self.vertex_capacity {
            self.vertex_capacity = vertices.len().next_power_of_two();
            self.vertex_buf = vertex_buffer(device, self.vertex_capacity);
        }
        rs.queue.write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(vertices));
        rs.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&self.globals(view_proj, look, holes)));
        // 1 行の長さは 256 バイトの倍数にそろえる（wgpu の決まり）。
        let row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture readback"), size: (row * size[1]) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture") });
        {
            let (msaa_view, depth_view, resolve_view) = (view(&msaa), view(&depth), view(&resolve));
            let clear = wgpu::Color { r: background[0] as f64, g: background[1] as f64, b: background[2] as f64, a: background[3] as f64 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    resolve_target: Some(&resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Discard },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_bind_group(1, &self.art_bind, &[]);
            pass.set_bind_group(2, &self.env_bind, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..std::mem::size_of_val(vertices) as u64));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) } },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        rs.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (send, receive) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| t!("Could not wait for drawing to finish: {e}"))?;
        receive.recv().map_err(|_| t!("Could not read the picture back").to_owned())?.map_err(|e| t!("Could not read the picture back: {e}"))?;
        let mapped = slice.get_mapped_range().map_err(|e| t!("Could not read the picture back: {e:?}"))?;
        let mut rgba = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for y in 0..size[1] as usize {
            let start = y * row as usize;
            rgba.extend_from_slice(&mapped[start..start + size[0] as usize * 4]);
        }
        drop(mapped);
        readback.unmap();
        // 色は乗算済みで描かれる（背景が透明のとき）ので、乗算なしに直す。
        if background[3] < 1.0 {
            for p in rgba.chunks_exact_mut(4) {
                let a = p[3];
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                    }
                }
            }
        }
        Ok(rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// シェーダーは、起動したときに GPU の部品が読む。書き間違いは起動して初めて分かるので、ここで読んで確かめる。
    #[test]
    fn シェーダーを読めて_型が合っている() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn 逆行列() {
        let m: Mat4 = [[2.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.5, 0.0], [0.0, 0.0, 4.0, 0.0], [1.0, 2.0, 3.0, 1.0]];
        let p = pakenoba_core::math::mat4_mul(&m, &invert(&m));
        for (i, col) in p.iter().enumerate() {
            for (j, v) in col.iter().enumerate() {
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-5, "{p:?}");
            }
        }
    }

    /// 描くたびに送る値の並びが、シェーダーの並び（16 バイトずつ）と合う。
    #[test]
    fn 送る値の大きさ() {
        assert_eq!(std::mem::size_of::<Globals>() % 16, 0);
    }
}
