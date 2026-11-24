//! 照らすための景色（周りの 360 度の絵。環境マップ）。
//!
//! 絵は正距円筒（横 2：縦 1）。真ん中が「見ている人の側」（+Z）、上が空（+Y）。
//! スタジオの景色は、ここで計算して作る（取ってこなくてよい）。屋外の景色は、Poly Haven（CC0）の HDR を
//! はじめて使うときに取ってきて、使う人のデータのフォルダにしまう。
//!
//! 描く前に、粗さごとにぼかした段階（ミップ）と、拡散の光（球面調和の 9 個の係数）を作っておく。

use pakenoba_i18n::t;
use std::f32::consts::PI;
use std::path::PathBuf;

/// 正距円筒の絵（線形の RGB）。
#[derive(Clone)]
pub struct Equirect {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
}

/// 絵の 1 画素の真ん中が指す向き。
fn direction(x: usize, y: usize, w: usize, h: usize) -> [f32; 3] {
    let phi = ((x as f32 + 0.5) / w as f32 - 0.5) * 2.0 * PI;
    let theta = (y as f32 + 0.5) / h as f32 * PI;
    [theta.sin() * phi.sin(), theta.cos(), theta.sin() * phi.cos()]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl Equirect {
    /// 縦横を半分にする（2x2 の平均）。
    fn half(&self) -> Equirect {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut px = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = self.px[(y * 2 + dy).min(self.h - 1) * self.w + (x * 2 + dx).min(self.w - 1)];
                    for c in 0..3 {
                        sum[c] += p[c] * 0.25;
                    }
                }
                px[y * w + x] = sum;
            }
        }
        Equirect { w, h, px }
    }

    /// 横が `w` 以下になるまで半分にしていく。
    fn shrink_to(&self, w: usize) -> Equirect {
        let mut out = self.clone();
        while out.w > w {
            out = out.half();
        }
        out
    }

    fn scale(&mut self, k: f32) {
        for p in &mut self.px {
            for c in p.iter_mut() {
                *c *= k;
            }
        }
    }
}

// ---- スタジオの景色を作る ----

/// 四角い照明（ソフトボックス、細長いストリップ）。
struct Panel {
    /// 向き（度）。左右は 0 が見ている人の側、正が右。上下は 0 が水平、正が上。
    azimuth: f32,
    elevation: f32,
    /// 半分の幅と高さ（度）。
    half: [f32; 2],
    /// 明るさ（線形の RGB）。
    color: [f32; 3],
}

fn toward(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [a.sin() * e.cos(), e.sin(), a.cos() * e.cos()]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Panel {
    fn light(&self, d: [f32; 3]) -> f32 {
        let c = toward(self.azimuth, self.elevation);
        let k = dot(d, c);
        if k <= 0.0 {
            return 0.0;
        }
        let up = if c[1].abs() > 0.99 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
        let t = normalize(cross(up, c));
        let b = cross(c, t);
        let (x, y) = ((dot(d, t) / k).abs(), (dot(d, b) / k).abs());
        let (hx, hy) = (self.half[0].to_radians().tan(), self.half[1].to_radians().tan());
        // 縁は少しだけぼかす（布の照明の縁の感じ）。
        (1.0 - smoothstep(0.92, 1.0, x / hx)) * (1.0 - smoothstep(0.92, 1.0, y / hy))
    }
}

/// 背景（壁と床）の明るさ。上下の向きだけで決める。
fn backdrop(d: [f32; 3], top: f32, horizon: f32, floor: f32) -> f32 {
    if d[1] >= 0.0 { horizon + (top - horizon) * d[1] } else { horizon + (floor - horizon) * smoothstep(0.0, 0.15, -d[1]) }
}

fn studio(backdrop_levels: [f32; 3], panels: &[Panel]) -> Equirect {
    let (w, h) = (1024, 512);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let d = direction(x, y, w, h);
            let base = backdrop(d, backdrop_levels[0], backdrop_levels[1], backdrop_levels[2]);
            let mut c = [base; 3];
            for p in panels {
                let k = p.light(d);
                if k > 0.0 {
                    for (c, light) in c.iter_mut().zip(p.color) {
                        *c += light * k;
                    }
                }
            }
            px.push(c);
        }
    }
    Equirect { w, h, px }
}

const fn white(k: f32) -> [f32; 3] {
    [k, k, k]
}

/// やわらかいスタジオ: 真上と奥の上の大きなソフトボックス、左右の後ろの細長い照明、手前からの弱い光。
/// フタの面には奥の照明が、側面には細長い照明が、つやのある紙なら映り込む。
fn studio_soft() -> Equirect {
    studio(
        [0.22, 0.16, 0.10],
        &[
            Panel { azimuth: 0.0, elevation: 75.0, half: [30.0, 22.0], color: white(5.0) },
            Panel { azimuth: 180.0, elevation: 32.0, half: [34.0, 16.0], color: white(5.0) },
            Panel { azimuth: -110.0, elevation: 14.0, half: [5.0, 30.0], color: white(6.0) },
            Panel { azimuth: 110.0, elevation: 14.0, half: [5.0, 30.0], color: white(6.0) },
            Panel { azimuth: 0.0, elevation: 24.0, half: [40.0, 14.0], color: white(4.0) },
        ],
    )
}

/// 一灯のスタジオ: 暗い部屋で、左上から強い光と、後ろから縁を照らす光。陰影がくっきり出る。
fn studio_key() -> Equirect {
    studio(
        [0.025, 0.02, 0.03],
        &[
            Panel { azimuth: -42.0, elevation: 32.0, half: [18.0, 24.0], color: [13.0, 12.6, 12.0] },
            Panel { azimuth: 150.0, elevation: 18.0, half: [4.0, 30.0], color: white(9.0) },
            Panel { azimuth: 62.0, elevation: 4.0, half: [16.0, 20.0], color: white(0.5) },
        ],
    )
}

/// 白いスタジオ（白ホリ）: まわりがすべて明るい。色の確認に向く。
fn studio_white() -> Equirect {
    studio([1.0, 0.85, 0.7], &[Panel { azimuth: 0.0, elevation: 78.0, half: [40.0, 40.0], color: white(2.5) }])
}

// ---- 景色の一覧 ----

/// 景色の作り方。
#[derive(Clone, Copy)]
enum Source {
    Built(fn() -> Equirect),
    /// Poly Haven の HDRI の名前。
    PolyHaven(&'static str),
}

pub struct Preset {
    /// 設定に書く名前。
    pub key: &'static str,
    label: &'static str,
    source: Source,
}

impl Preset {
    pub fn label(&self) -> &'static str {
        pakenoba_i18n::tr(self.label)
    }

    /// 取ってくる景色なら真。
    pub fn downloads(&self) -> bool {
        matches!(self.source, Source::PolyHaven(_))
    }

    /// 取ってきた景色を、もう持っているか。
    pub fn cached(&self) -> bool {
        match self.source {
            Source::Built(_) => true,
            Source::PolyHaven(id) => cache_path(id).is_some_and(|p| p.is_file()),
        }
    }
}

pub const PRESETS: &[Preset] = &[
    Preset { key: "studio_soft", label: "Studio (soft)", source: Source::Built(studio_soft) },
    Preset { key: "studio_key", label: "Studio (one key light)", source: Source::Built(studio_key) },
    Preset { key: "studio_white", label: "Studio (white)", source: Source::Built(studio_white) },
    Preset { key: "outdoor_clear", label: "Outdoors, clear sky", source: Source::PolyHaven("kloofendal_43d_clear") },
    Preset { key: "outdoor_partly_cloudy", label: "Outdoors, partly cloudy", source: Source::PolyHaven("kloofendal_48d_partly_cloudy") },
    Preset { key: "outdoor_overcast", label: "Outdoors, overcast", source: Source::PolyHaven("quarry_cloudy") },
];

pub fn preset(key: &str) -> &'static Preset {
    PRESETS.iter().find(|p| p.key == key).unwrap_or(&PRESETS[0])
}

fn cache_path(id: &str) -> Option<PathBuf> {
    Some(crate::app::settings::data_dir()?.join("environments").join(format!("{id}_1k.hdr")))
}

/// 取ってくる HDR の大きさの上限。1K の HDR は 1〜3 MB。
const DOWNLOAD_LIMIT: u64 = 32 << 20;

fn download(id: &str) -> Result<Vec<u8>, String> {
    let url = format!("https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/1k/{id}_1k.hdr");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(120))).build().into();
    let mut response = agent.get(&url).call().map_err(|e| t!("Could not download the environment: {e}"))?;
    response.body_mut().with_config().limit(DOWNLOAD_LIMIT).read_to_vec().map_err(|e| t!("Could not download the environment: {e}"))
}

/// 景色の絵を用意する（スタジオなら作り、屋外なら、しまったものを読むか、取ってくる）。重いので、別のスレッドで呼ぶ。
pub fn load(preset: &Preset) -> Result<Prepared, String> {
    let image = match preset.source {
        Source::Built(make) => make(),
        Source::PolyHaven(id) => {
            let path = cache_path(id).ok_or_else(|| t!("There is no folder to keep the environment in").to_owned())?;
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => {
                    let bytes = download(id)?;
                    // しまえなくても、今回は使う。
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                        let part = path.with_extension("part");
                        if std::fs::write(&part, &bytes).is_ok() {
                            let _ = std::fs::rename(&part, &path);
                        }
                    }
                    bytes
                }
            };
            match decode_hdr(&bytes) {
                Ok(image) => image,
                Err(e) => {
                    // 壊れたものは捨てて、次に取り直す。
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
            }
        }
    };
    Ok(prepare(&image))
}

// ---- Radiance HDR（.hdr）を読む ----

fn rgbe(p: [u8; 4]) -> [f32; 3] {
    if p[3] == 0 {
        return [0.0; 3];
    }
    let f = 2f32.powi(p[3] as i32 - 136);
    [p[0] as f32 * f, p[1] as f32 * f, p[2] as f32 * f]
}

/// Radiance HDR を読む（新しい形のランレングス圧縮と、圧縮なし）。
pub fn decode_hdr(bytes: &[u8]) -> Result<Equirect, String> {
    let bad = || t!("The environment file is not a valid HDR image").to_owned();
    if !bytes.starts_with(b"#?") {
        return Err(bad());
    }
    // 見出しは、空の行で終わる。その次の行が大きさ。
    let mut at = 0;
    let line = |at: &mut usize| -> Option<&[u8]> {
        let start = *at;
        let end = start + bytes.get(start..)?.iter().position(|&b| b == b'\n')?;
        *at = end + 1;
        Some(&bytes[start..end])
    };
    loop {
        let l = line(&mut at).ok_or_else(bad)?;
        if l.is_empty() {
            break;
        }
        if l.starts_with(b"FORMAT=") && l != b"FORMAT=32-bit_rle_rgbe" {
            return Err(bad());
        }
    }
    let size = std::str::from_utf8(line(&mut at).ok_or_else(bad)?).map_err(|_| bad())?;
    let parts: Vec<&str> = size.split_whitespace().collect();
    let (h, w) = match parts.as_slice() {
        ["-Y", h, "+X", w] => (h.parse::<usize>().map_err(|_| bad())?, w.parse::<usize>().map_err(|_| bad())?),
        _ => return Err(bad()),
    };
    if w == 0 || h == 0 || w > 16384 || h > 8192 {
        return Err(bad());
    }
    let mut px = Vec::with_capacity(w * h);
    let mut row = vec![[0u8; 4]; w];
    for _ in 0..h {
        let head = bytes.get(at..at + 4).ok_or_else(bad)?;
        if (8..32768).contains(&w) && head[0] == 2 && head[1] == 2 && head[2] & 0x80 == 0 {
            if ((head[2] as usize) << 8 | head[3] as usize) != w {
                return Err(bad());
            }
            at += 4;
            for c in 0..4 {
                let mut x = 0;
                while x < w {
                    let n = *bytes.get(at).ok_or_else(bad)? as usize;
                    at += 1;
                    if n > 128 {
                        let n = n - 128;
                        let v = *bytes.get(at).ok_or_else(bad)?;
                        at += 1;
                        if x + n > w {
                            return Err(bad());
                        }
                        for p in &mut row[x..x + n] {
                            p[c] = v;
                        }
                        x += n;
                    } else {
                        if n == 0 || x + n > w {
                            return Err(bad());
                        }
                        let run = bytes.get(at..at + n).ok_or_else(bad)?;
                        for (p, &v) in row[x..x + n].iter_mut().zip(run) {
                            p[c] = v;
                        }
                        at += n;
                        x += n;
                    }
                }
            }
        } else {
            let flat = bytes.get(at..at + w * 4).ok_or_else(bad)?;
            for (p, q) in row.iter_mut().zip(flat.chunks_exact(4)) {
                *p = [q[0], q[1], q[2], q[3]];
            }
            at += w * 4;
        }
        px.extend(row.iter().map(|&p| rgbe(p)));
    }
    // 太陽のような強すぎる光は、GPU の半精度の数に収まるようにする。
    for p in &mut px {
        for c in p.iter_mut() {
            *c = c.min(60000.0);
        }
    }
    Ok(Equirect { w, h, px })
}

// ---- 描く前の下ごしらえ ----

/// 粗さの段階の数。段階 i の粗さは i / (MIPS - 1)。
pub const MIPS: usize = 6;
/// いちばん細かい段階の横の画素数。
const TOP: usize = 512;

/// GPU へ送るもの。
pub struct Prepared {
    /// 粗さごとにぼかした絵。0 が鏡のような映り込み、最後がいちばん粗い。横は 512, 256, …
    pub mips: Vec<Equirect>,
    /// 拡散の光（放射照度）の、球面調和の 9 個の係数（RGB）。基底の定数を掛けてある。
    pub sh: [[f32; 3]; 9],
}

/// 景色から、描くためのものを作る。明るさは、いちばん明るい向きの白い紙がちょうど白くなるようにそろえる。
pub fn prepare(image: &Equirect) -> Prepared {
    let mut top = image.shrink_to(TOP);
    let sh = irradiance(&top.shrink_to(64));
    // 見ている人の側の少し上を向いた面（箱の正面）の白い紙が、ほぼ白（0.9）になるようにする。
    // ただし、いちばん明るい向きが白飛びしすぎない（1.5 まで）ようにする。
    let peak = peak_irradiance(&sh);
    let front = luminance(shade(&sh, toward(0.0, 25.0)));
    let k = if front > 1e-6 && peak > 1e-6 { (0.9 / front).min(1.5 / peak) } else if peak > 1e-6 { 0.95 / peak } else { 1.0 };
    top.scale(k);
    let sh = sh.map(|c| c.map(|v| v * k));
    let mut mips = vec![top.clone()];
    let mut level = top;
    for i in 1..MIPS {
        level = level.half();
        let roughness = i as f32 / (MIPS - 1) as f32;
        mips.push(blur(&level, roughness));
    }
    Prepared { mips, sh }
}

/// 粗さ `roughness` の映り込みにぼかす。GGX の山を、同じくらいの幅の cos の n 乗で近似して、近くの画素を足す。
fn blur(src: &Equirect, roughness: f32) -> Equirect {
    let alpha = (roughness * roughness).max(1e-3);
    let n = (2.0 / (alpha * alpha) - 2.0).max(0.0);
    // 重みが 1/1000 より小さくなる角度より外は、足さない。
    let reach = if n < 1.0 { PI * 0.5 } else { (0.001f32.powf(1.0 / n)).acos().min(PI * 0.5) };
    let (w, h) = (src.w, src.h);
    let dtheta = PI / h as f32;
    let dphi = 2.0 * PI / w as f32;
    let dirs: Vec<[f32; 3]> = (0..w * h).map(|i| direction(i % w, i / w, w, h)).collect();
    let solid: Vec<f32> = (0..h).map(|y| ((y as f32 + 0.5) * dtheta).sin() * dtheta * dphi).collect();
    let mut px = vec![[0.0f32; 3]; w * h];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (chunk, out) in px.chunks_mut(rows_per * w).enumerate() {
            let (dirs, solid, src) = (&dirs, &solid, src);
            s.spawn(move || {
                for (i, slot) in out.iter_mut().enumerate() {
                    let (x, y) = (i % w, chunk * rows_per + i / w);
                    let r = dirs[y * w + x];
                    let theta_r = (y as f32 + 0.5) * dtheta;
                    let y0 = ((theta_r - reach) / dtheta).floor().max(0.0) as usize;
                    let y1 = (((theta_r + reach) / dtheta).ceil() as usize).min(h - 1);
                    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
                    for sy in y0..=y1 {
                        let theta_s = (sy as f32 + 0.5) * dtheta;
                        // この行で、角度が reach 以内になる左右の幅。
                        let denom = theta_r.sin() * theta_s.sin();
                        let span = if denom < 1e-4 {
                            w / 2
                        } else {
                            let c = (reach.cos() - theta_r.cos() * theta_s.cos()) / denom;
                            if c <= -1.0 {
                                w / 2
                            } else if c >= 1.0 {
                                0
                            } else {
                                ((c.acos() / dphi).ceil() as usize + 1).min(w / 2)
                            }
                        };
                        for k in 0..=span * 2 {
                            let sx = (x + w + k).wrapping_sub(span) % w;
                            if span * 2 >= w && k >= w {
                                break;
                            }
                            let d = dot(r, dirs[sy * w + sx]);
                            if d <= 0.0 {
                                continue;
                            }
                            let wgt = if n > 0.0 { (n * d.ln()).exp() } else { 1.0 } * solid[sy];
                            let p = src.px[sy * w + sx];
                            for c in 0..3 {
                                sum[c] += p[c] * wgt;
                            }
                            weight += wgt;
                        }
                    }
                    *slot = if weight > 0.0 { sum.map(|v| v / weight) } else { src.px[y * w + x] };
                }
            });
        }
    });
    Equirect { w, h, px }
}

/// 球面調和の基底（9 個）。並びは シェーダーと同じ。
fn basis(d: [f32; 3]) -> [f32; 9] {
    let [x, y, z] = d;
    [0.282095, 0.488603 * y, 0.488603 * z, 0.488603 * x, 1.092548 * x * y, 1.092548 * y * z, 0.315392 * (3.0 * z * z - 1.0), 1.092548 * x * z, 0.546274 * (x * x - y * y)]
}

/// 放射照度 E(n) の係数。シェーダーでは Σ sh[i] * basis(n)[i] で E が出る。
fn irradiance(src: &Equirect) -> [[f32; 3]; 9] {
    let (w, h) = (src.w, src.h);
    let (dtheta, dphi) = (PI / h as f32, 2.0 * PI / w as f32);
    let mut l = [[0.0f32; 3]; 9];
    for y in 0..h {
        let solid = ((y as f32 + 0.5) * dtheta).sin() * dtheta * dphi;
        for x in 0..w {
            let b = basis(direction(x, y, w, h));
            let p = src.px[y * w + x];
            for i in 0..9 {
                for c in 0..3 {
                    l[i][c] += p[c] * b[i] * solid;
                }
            }
        }
    }
    // 光の量から放射照度へ（帯ごとの定数）。
    let a = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];
    let mut out = [[0.0f32; 3]; 9];
    for i in 0..9 {
        for c in 0..3 {
            out[i][c] = l[i][c] * a[i] * basis_scale(i);
        }
    }
    out
}

/// 係数に、基底の定数をもう一度掛けておく（シェーダーでは定数なしの多項式で足すため）。
fn basis_scale(i: usize) -> f32 {
    [0.282095, 0.488603, 0.488603, 0.488603, 1.092548, 1.092548, 0.315392, 1.092548, 0.546274][i]
}

/// シェーダーと同じ式で、向き `n` の E/π（白い紙の明るさ）を出す。
pub fn shade(sh: &[[f32; 3]; 9], n: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = n;
    let p = [1.0, y, z, x, x * y, y * z, 3.0 * z * z - 1.0, x * z, x * x - y * y];
    let mut e = [0.0f32; 3];
    for i in 0..9 {
        for c in 0..3 {
            e[c] += sh[i][c] * p[i];
        }
    }
    e.map(|v| v.max(0.0) / PI)
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn peak_irradiance(sh: &[[f32; 3]; 9]) -> f32 {
    let (w, h) = (32, 16);
    (0..w * h).map(|i| luminance(shade(sh, direction(i % w, i / w, w, h)))).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, v: f32) -> Equirect {
        Equirect { w, h, px: vec![[v; 3]; w * h] }
    }

    #[test]
    fn 一様な景色では_どの向きも同じ明るさ() {
        let sh = irradiance(&flat(64, 32, 1.0));
        for n in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], normalize([1.0, -1.0, 1.0])] {
            let e = shade(&sh, n);
            assert!((e[0] - 1.0).abs() < 0.02, "{n:?} {e:?}");
        }
        // ぼかしても、明るさは変わらない。
        let b = blur(&flat(64, 32, 0.5), 0.6);
        assert!(b.px.iter().all(|p| (p[0] - 0.5).abs() < 1e-3));
    }

    #[test]
    fn 上が明るいと_上を向いた面が明るい() {
        let mut image = flat(64, 32, 0.0);
        for y in 0..8 {
            for x in 0..64 {
                image.px[y * 64 + x] = [1.0; 3];
            }
        }
        let sh = irradiance(&image);
        assert!(shade(&sh, [0.0, 1.0, 0.0])[0] > 0.5);
        assert!(shade(&sh, [0.0, -1.0, 0.0])[0] < 0.05);
        let p = prepare(&image);
        assert_eq!(p.mips.len(), MIPS);
        assert_eq!(p.mips[0].w, 64);
        // 明るさをそろえてある（正面は暗い景色なので、いちばん明るい向きが 1.5 になる）。
        assert!((peak_irradiance(&p.sh) - 1.5).abs() < 0.01);
    }

    #[test]
    fn 粗いほど_光の点が広がる() {
        let mut image = flat(128, 64, 0.0);
        image.px[32 * 128 + 64] = [1000.0; 3];
        let sharp = blur(&image, 0.2);
        let rough = blur(&image, 0.8);
        let at = |e: &Equirect, x: usize| e.px[32 * 128 + x][0];
        assert!(at(&sharp, 64) > at(&rough, 64));
        assert!(at(&rough, 64 + 12) > at(&sharp, 64 + 12));
    }

    /// 圧縮なしの HDR と、ランレングス圧縮の HDR を作って読む。
    #[test]
    fn hdr_を読める() {
        let (w, h) = (16usize, 2usize);
        let header = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {h} +X {w}\n");
        // 1 行目は圧縮なし、2 行目はランレングス。
        let mut bytes = header.into_bytes();
        for x in 0..w {
            bytes.extend_from_slice(&[128, x as u8 * 8, 0, 129]);
        }
        bytes.extend_from_slice(&[2, 2, 0, w as u8]);
        for c in [64u8, 32, 16, 130] {
            bytes.extend_from_slice(&[128 + w as u8, c]);
        }
        let image = decode_hdr(&bytes).unwrap();
        assert_eq!((image.w, image.h), (w, h));
        // 128 * 2^(129-136) = 1.0
        assert!((image.px[0][0] - 1.0).abs() < 1e-6);
        assert!((image.px[3][1] - 24.0 / 128.0).abs() < 1e-6);
        // 64 * 2^(130-136) = 1.0
        assert!((image.px[w][0] - 1.0).abs() < 1e-6 && (image.px[w + 5][2] - 0.25).abs() < 1e-6);
        assert!(decode_hdr(b"PNG").is_err());
        assert!(decode_hdr(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn スタジオの景色を作れる() {
        for p in PRESETS.iter().filter(|p| !p.downloads()) {
            let prepared = load(p).unwrap();
            assert_eq!(prepared.mips[0].w, TOP);
            assert!(prepared.mips.iter().all(|m| m.px.iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0))));
        }
    }
}
