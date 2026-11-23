//! のぞき込み（出力の工程）。Web カメラで見ている人の顔を見つけ、画面を窓にして、その向こうに箱を置く。
//!
//! - 顔を右へ動かすと箱の右の側面が見え（左の側面は隠れていき）、近づくと大きく見える（頭の位置に合わせた、ずらした透視投影）。
//! - 顔の検出は、SeetaFace の Rust 版（rustface）。カメラの映像は、この中で灰色の小さな絵にして使うだけで、
//!   保存も送信もしない。
//! - カメラの読み込みと顔の検出は、別のスレッドで行う。画面のほうは、届いた目の位置をなめらかにして使う。
//!
//! 座標は、画面（モニター）の中央を原点にした mm。x は右、y は上、z は画面から見ている人のほう。

use pakenoba_i18n::t;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use eframe::egui::{self, Align2, Color32, FontId, Rect, pos2, vec2};
use eframe::egui_wgpu;

use pakenoba_core::math::{Mat4, V3, look_at, mat4_mul, normalize};
use pakenoba_core::template::bounding_sphere;

use super::App;
use crate::gpu::BoxRenderer;

/// 顔の検出のモデル（SeetaFace の正面の顔。BSD-2-Clause）。
const MODEL: &[u8] = include_bytes!("../../assets/seeta_fd_frontal_v1.0.bin");
/// 顔の検出にかける絵の幅（px）。小さいほど速い。
const DETECT_WIDTH: usize = 320;
/// 設定の欄に出す、カメラの絵の幅（px）。
const PREVIEW_WIDTH: usize = 160;
/// 顔の幅（mm）。検出の枠の幅を、この大きさの顔として、カメラからの距離を見積もる。
const FACE_MM: f32 = 150.0;
/// 顔が見つからないとき、見ている人がいるとみなす場所（画面の正面、60 cm）。
pub const REST_EYE: V3 = [0.0, 0.0, 600.0];

/// 画面の幅の見本（mm）。
pub const SCREENS: [(&str, f32); 6] = [("13-inch laptop", 290.0), ("15.6-inch laptop", 345.0), ("24-inch monitor", 530.0), ("27-inch monitor", 600.0), ("32-inch monitor", 710.0), ("Large TV (55-inch)", 1210.0)];

/// のぞき込みの設定。
#[derive(Clone, Debug, PartialEq)]
pub struct PeekSettings {
    /// 使うカメラ（一覧の番号）。
    pub camera: usize,
    /// 画面の見えている幅（mm）。これで、画面の上の 1 点が実際の何 mm かを決める。
    pub screen_mm: f32,
    /// 効き具合。1 で、実際の頭の動きのとおり。大きいほど、少し動いただけで大きく回り込む。
    pub strength: f32,
    /// カメラの絵が左右逆のとき。
    pub mirror: bool,
    /// カメラの横の画角（度）。
    pub camera_fov: f32,
    /// ぶれ補正（0〜1。大きいほど強く抑え、そのぶん少し遅れる）。
    pub stability: f32,
}

impl Default for PeekSettings {
    fn default() -> Self {
        PeekSettings { camera: 0, screen_mm: 530.0, strength: 1.0, mirror: false, camera_fov: 60.0, stability: 0.3 }
    }
}

/// カメラの絵の中の顔。どれも絵の幅と高さに対する割合（0〜1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
}

/// 顔の場所から、目の位置（mm）を見積もる。カメラは画面の上の縁の中央にあるとする。
/// `aspect` はカメラの絵の幅÷高さ、`screen_h` は画面の高さ（mm）。
pub fn eye_from_face(face: Face, aspect: f32, screen_h: f32, settings: &PeekSettings) -> V3 {
    let hfov = settings.camera_fov.to_radians();
    let vfov = 2.0 * ((hfov * 0.5).tan() / aspect).atan();
    let width_angle = (face.w * hfov).max(1e-3);
    let distance = (FACE_MM / (2.0 * (width_angle * 0.5).tan())).clamp(200.0, 3000.0);
    // カメラは見ている人のほうを向いているので、人が自分の右へ動くと、絵では左へ動く。
    let sideways = if settings.mirror { face.cx - 0.5 } else { 0.5 - face.cx };
    let x = distance * (sideways * hfov).tan();
    let y = screen_h * 0.5 + 10.0 + distance * ((0.5 - face.cy) * vfov).tan();
    let k = settings.strength;
    [x * k, y * k, distance]
}

/// 1 つの値の、ぶれを抑えるフィルター（One Euro フィルター）。止まっているときは強く抑え、速く動くときは遅れを減らす。
#[derive(Clone, Copy, Debug, Default)]
struct OneEuro {
    value: Option<f32>,
    speed: f32,
}

impl OneEuro {
    fn alpha(cutoff: f32, dt: f32) -> f32 {
        let tau = 1.0 / (std::f32::consts::TAU * cutoff);
        1.0 / (1.0 + tau / dt)
    }

    /// `min_cutoff` は止まっているときの効き（Hz。小さいほど強く抑える）、`beta` は速さで効きをゆるめる度合い。
    fn filter(&mut self, x: f32, dt: f32, min_cutoff: f32, beta: f32) -> f32 {
        let dt = dt.clamp(1e-3, 0.5);
        let Some(prev) = self.value else {
            self.value = Some(x);
            return x;
        };
        let speed = (x - prev) / dt;
        self.speed += (speed - self.speed) * OneEuro::alpha(1.0, dt);
        let cutoff = min_cutoff + beta * self.speed.abs();
        let out = prev + (x - prev) * OneEuro::alpha(cutoff, dt);
        self.value = Some(out);
        out
    }
}

/// 目の位置の、ぶれ補正。遅れを作らないことを先にする（遅れると、見ている人が酔う）。
/// - 顔が見つかるたび、測った目の位置を One Euro フィルターに通す。補正が 0 なら、そのまま使う。
///   奥行きは顔の幅から見積もるので、ぶれが大きい。横と縦より強く抑える。
/// - 測った速さから、コマを撮ってからいままでの時間（カメラと検出の遅れ）のぶん先を読んで、遅れを打ち消す。
/// - 画面の更新ごとにも、前に測ってからの時間のぶん先を読む（測る間隔の段々を消す）。
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    pub eye: V3,
    /// 最後に測った（ならした）位置と、速さ（mm/秒）と、それからの時間（秒）。
    base: V3,
    speed: V3,
    since: f32,
    filters: [OneEuro; 3],
    /// 顔を見失ってからの時間（秒）。
    lost: f32,
}

impl Default for Smoother {
    fn default() -> Self {
        Smoother { eye: REST_EYE, base: REST_EYE, speed: [0.0; 3], since: 0.0, filters: Default::default(), lost: f32::INFINITY }
    }
}

/// 先読みの上限（秒）。これより長く測れていなければ、先へは進めない。
const LOOK_AHEAD: f32 = 0.12;

impl Smoother {
    /// 顔が見つかったとき。`dt` は前に測ってからの時間、`age` はそのコマを撮ってからの時間（秒）。
    /// `stability` は 0〜1（大きいほど強く抑える。0 なら何もしない）。
    pub fn measure(&mut self, eye: V3, dt: f32, age: f32, stability: f32) {
        let s = stability.clamp(0.0, 1.0);
        let fresh = self.lost > 1.5;
        if fresh {
            // しばらく見失っていたあとは、測った場所から始め直す（正面から飛んでこない）。
            self.filters = Default::default();
            self.speed = [0.0; 3];
        }
        self.lost = 0.0;
        let before = self.base;
        for (k, (base, filter)) in self.base.iter_mut().zip(&mut self.filters).enumerate() {
            *base = if s <= 0.0 {
                eye[k]
            } else {
                // 止まっているときの効き（Hz）。補正が強いほど低く（強く抑える）。奥行きは 3 分の 1。
                let min_cutoff = 10.0 * (1.0 - s) + 0.6 * s;
                let cut = if k == 2 { min_cutoff / 3.0 } else { min_cutoff };
                filter.filter(eye[k], dt, cut, 0.03)
            };
        }
        if !fresh && dt > 1e-3 {
            for ((speed, base), old) in self.speed.iter_mut().zip(self.base).zip(before) {
                *speed += ((base - old) / dt - *speed) * 0.5;
            }
        }
        self.since = age.clamp(0.0, LOOK_AHEAD);
        if fresh {
            self.eye = self.base;
        }
    }

    /// 顔が見つからなかったとき。
    pub fn miss(&mut self, dt: f32) {
        self.lost += dt;
        // 少しのあいだは、その場で待つ（先読みもしない）。それより長く見失ったら、正面へゆっくり戻る。
        self.speed = [0.0; 3];
        if self.lost >= 1.5 {
            self.base = REST_EYE;
        }
    }

    /// 画面の更新ごと。`dt` は前の更新からの時間（秒）。
    pub fn update(&mut self, dt: f32, stability: f32) -> V3 {
        let dt = dt.clamp(1e-3, 0.1);
        self.since = (self.since + dt).min(LOOK_AHEAD);
        let ahead: V3 = std::array::from_fn(|k| self.base[k] + self.speed[k] * self.since);
        if self.lost >= 1.5 {
            // 正面へ戻るときだけは、ゆっくり。
            let a = 1.0 - (-dt * 2.5).exp();
            for (e, t) in self.eye.iter_mut().zip(ahead) {
                *e += (t - *e) * a;
            }
        } else if stability <= 0.0 {
            self.eye = ahead;
        } else {
            // 段々をわずかにならす（1 コマ 60 分の 1 秒のうちに、ほとんど追いつく）。
            let a = 1.0 - (-dt * 40.0).exp();
            for (e, t) in self.eye.iter_mut().zip(ahead) {
                *e += (t - *e) * a;
            }
        }
        self.eye
    }
}

/// 画面の上の窓（mm。原点は画面の中央、y は上）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
}

/// 窓の向こうに箱を置いて、目の位置から見たときの、投影と視点と箱の置き方をかけた行列。
/// 箱は、いまの立体の表示の向き（`yaw`、`pitch`）で、窓の高さの 7 割ほどに収まる大きさにし、
/// 中心を画面の少し奥に置く。返すのは、行列と、箱の座標での光の向き。
pub fn window_view(window: Window, eye: V3, center: V3, radius: f32, yaw: f32, pitch: f32, zoom: f32) -> (Mat4, V3) {
    let (w, h) = (window.right - window.left, window.top - window.bottom);
    let size = 0.36 * w.min(h) * zoom;
    let scale = size / radius.max(1e-3);
    let middle = [(window.left + window.right) * 0.5, (window.bottom + window.top) * 0.5, -0.45 * size];
    // 箱の向き: 立体の表示と同じ向きから見た姿を、画面の正面に向ける。
    let dir = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
    let mut turn = look_at(dir, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    turn[3] = [0.0, 0.0, 0.0, 1.0];
    let model = mat4_mul(&translate(middle), &mat4_mul(&scale_m(scale), &mat4_mul(&turn, &translate([-center[0], -center[1], -center[2]]))));
    // 目が画面に近すぎたり、裏へ回ったりしないようにする。
    let eye = [eye[0], eye[1], eye[2].max(size * 1.2 + 50.0)];
    let view = translate([-eye[0], -eye[1], -eye[2]]);
    let near = (eye[2] * 0.05).max(1.0);
    let far = eye[2] + size * 4.0;
    let k = near / eye[2];
    let (l, r, b, t) = ((window.left - eye[0]) * k, (window.right - eye[0]) * k, (window.bottom - eye[1]) * k, (window.top - eye[1]) * k);
    let proj = frustum(l, r, b, t, near, far);
    // 光は、立体の表示と同じく、箱から見て視点の少し右上から（箱といっしょに動き、頭の動きでは変わらない）。
    let right = normalize([turn[0][0], turn[1][0], turn[2][0]]);
    let light = normalize([dir[0] + right[0] * 0.45, dir[1] + 0.55, dir[2] + right[2] * 0.45]);
    (mat4_mul(&proj, &mat4_mul(&view, &model)), light)
}

/// 右手系の、ずらした透視投影。奥行きは 0〜1（wgpu の規約）。
fn frustum(l: f32, r: f32, b: f32, t: f32, near: f32, far: f32) -> Mat4 {
    [
        [2.0 * near / (r - l), 0.0, 0.0, 0.0],
        [0.0, 2.0 * near / (t - b), 0.0, 0.0],
        [(r + l) / (r - l), (t + b) / (t - b), far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

fn translate(v: V3) -> Mat4 {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [v[0], v[1], v[2], 1.0]]
}

fn scale_m(s: f32) -> Mat4 {
    [[s, 0.0, 0.0, 0.0], [0.0, s, 0.0, 0.0], [0.0, 0.0, s, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

/// 灰色の絵の中から、いちばん大きな顔を探す。
pub struct FaceFinder {
    detector: Box<dyn rustface::Detector>,
}

impl FaceFinder {
    pub fn new() -> Result<FaceFinder, String> {
        let model = rustface::read_model(MODEL).map_err(|e| t!("Could not read the face detection model: {e}"))?;
        let mut detector = rustface::create_detector_with_model(model);
        detector.set_min_face_size(36);
        detector.set_score_thresh(2.0);
        detector.set_pyramid_scale_factor(0.75);
        detector.set_slide_window_step(4, 4);
        Ok(FaceFinder { detector })
    }

    /// `gray` は幅 `w`、高さ `h` の灰色の絵。
    pub fn find(&mut self, gray: &[u8], w: usize, h: usize) -> Option<Face> {
        if w < 24 || h < 24 || gray.len() != w * h {
            return None;
        }
        let image = rustface::ImageData::new(gray, w as u32, h as u32);
        let faces = self.detector.detect(&image);
        let best = faces.iter().max_by_key(|f| f.bbox().width())?;
        let b = best.bbox();
        Some(Face { cx: (b.x() as f32 + b.width() as f32 * 0.5) / w as f32, cy: (b.y() as f32 + b.height() as f32 * 0.5) / h as f32, w: b.width() as f32 / w as f32 })
    }
}

/// 灰色の絵を、幅 `to` に縮める（面積の平均）。
pub fn shrink(gray: &[u8], w: usize, h: usize, to: usize) -> (Vec<u8>, usize, usize) {
    if w <= to {
        return (gray.to_vec(), w, h);
    }
    let (nw, nh) = (to, (h * to / w).max(1));
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = 0u32;
            for row in y0..y1 {
                sum += gray[row * w + x0..row * w + x1].iter().map(|v| *v as u32).sum::<u32>();
            }
            out[y * nw + x] = (sum / ((x1 - x0) * (y1 - y0)) as u32) as u8;
        }
    }
    (out, nw, nh)
}

/// カメラのスレッドから届くもの。
pub enum Report {
    /// 顔（見つからなければ `None`）と、カメラの絵の縦横比と、そのコマを撮ってから送るまでの時間（秒）。
    Face(Option<Face>, f32, f32),
    /// 設定の欄に出す、小さな灰色の絵。
    Preview(Vec<u8>, usize, usize),
    Error(String),
}

/// 動いているカメラ。落とすと止まる。
pub struct Tracker {
    pub reports: mpsc::Receiver<Report>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// つながっているカメラの名前。
pub fn cameras() -> Vec<String> {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        nokhwa::query(nokhwa::utils::ApiBackend::Auto).map(|list| list.iter().map(|c| c.human_name()).collect()).unwrap_or_default()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

impl Tracker {
    /// カメラ `index` を開いて、顔を探しはじめる。
    pub fn start(index: usize) -> Tracker {
        let (send, reports) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new().name("peek-camera".into()).spawn(move || {
            if let Err(message) = run(index, &flag, &send) {
                let _ = send.send(Report::Error(message));
            }
        });
        Tracker { reports, stop, thread: thread.ok() }
    }
}

/// カメラから届いた、いちばん新しいコマ（明るさだけ）。古いコマは捨てる（溜めると、見ている人の動きから遅れる）。
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
type Gray = (Vec<u8>, usize, usize, std::time::Instant);

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
struct Latest {
    /// いちばん新しいコマ（明るさ、幅、高さ、撮った時刻）と、カメラが止まった理由（止まったときだけ）。
    frame: std::sync::Mutex<(Option<Gray>, Option<String>)>,
    ready: std::sync::Condvar,
}

/// カメラを読むスレッドと、顔を探すこのスレッドを分ける。カメラは止まらずに読み続け、
/// こちらは探し終わるたびに、その時点でいちばん新しいコマだけを使う。
/// 前に顔を見つけていれば、そのまわりだけを小さくして探す（速い）。見失ったら、絵の全体から探す。
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn run(index: usize, stop: &AtomicBool, send: &mpsc::SyncSender<Report>) -> Result<(), String> {
    let mut finder = FaceFinder::new()?;
    let latest = Arc::new(Latest { frame: std::sync::Mutex::new((None, None)), ready: std::sync::Condvar::new() });
    let quit = Arc::new(AtomicBool::new(false));
    let reader = {
        let (latest, quit) = (Arc::clone(&latest), Arc::clone(&quit));
        std::thread::Builder::new().name("peek-capture".into()).spawn(move || {
            let failed = capture(index, &quit, &latest).err();
            let mut slot = latest.frame.lock().unwrap_or_else(|e| e.into_inner());
            slot.1 = Some(failed.unwrap_or_default());
            latest.ready.notify_all();
        })
    }
    .map_err(|e| e.to_string())?;
    let mut last: Option<Face> = None;
    let mut sent_preview = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        // いちばん新しいコマを待って、取り出す。
        let taken = {
            let mut slot = latest.frame.lock().unwrap_or_else(|e| e.into_inner());
            while slot.0.is_none() && slot.1.is_none() && !stop.load(Ordering::Relaxed) {
                slot = latest.ready.wait_timeout(slot, std::time::Duration::from_millis(100)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
            }
            if let Some(error) = slot.1.take() {
                break if error.is_empty() { Ok(()) } else { Err(error) };
            }
            slot.0.take()
        };
        let Some((gray, w, h, shot)) = taken else { continue };
        let face = last.and_then(|f| find_near(&mut finder, &gray, w, h, f)).or_else(|| {
            let (small, sw, sh) = shrink(&gray, w, h, DETECT_WIDTH);
            finder.find(&small, sw, sh)
        });
        last = face;
        if send.try_send(Report::Face(face, w as f32 / h.max(1) as f32, shot.elapsed().as_secs_f32())).is_err() && stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        // 設定の欄の小さな絵は、1 秒に 10 回まで。
        if sent_preview.elapsed().as_millis() >= 100 {
            sent_preview = std::time::Instant::now();
            let (preview, pw, ph) = shrink(&gray, w, h, PREVIEW_WIDTH);
            let _ = send.try_send(Report::Preview(preview, pw, ph));
        }
    };
    quit.store(true, Ordering::Relaxed);
    let _ = reader.join();
    result
}

/// 前に見つけた顔のまわり（顔の幅の 2.6 倍の四角）だけを、顔が 46 px ほどになるように縮めて探す。
fn find_near(finder: &mut FaceFinder, gray: &[u8], w: usize, h: usize, last: Face) -> Option<Face> {
    let face_px = last.w * w as f32;
    let side = (face_px * 2.6).clamp(48.0, w.min(h) as f32) as usize;
    let cx = (last.cx * w as f32) as isize;
    let cy = (last.cy * h as f32) as isize;
    let x0 = (cx - side as isize / 2).clamp(0, (w - side) as isize) as usize;
    let y0 = (cy - side as isize / 2).clamp(0, (h - side) as isize) as usize;
    let mut crop = Vec::with_capacity(side * side);
    for y in y0..y0 + side {
        crop.extend_from_slice(&gray[y * w + x0..y * w + x0 + side]);
    }
    let to = ((side as f32 * 46.0 / face_px.max(1.0)) as usize).clamp(48, side.max(48));
    let (small, sw, sh) = shrink(&crop, side, side, to);
    let f = finder.find(&small, sw, sh)?;
    Some(Face { cx: (x0 as f32 + f.cx * side as f32) / w as f32, cy: (y0 as f32 + f.cy * side as f32) / h as f32, w: f.w * side as f32 / w as f32 })
}

/// カメラを読み続け、明るさだけにして、いちばん新しいコマの置き場に入れる。
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn capture(index: usize, quit: &AtomicBool, latest: &Latest) -> Result<(), String> {
    use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
    const FORMATS: [FrameFormat; 4] = [FrameFormat::YUYV, FrameFormat::MJPEG, FrameFormat::NV12, FrameFormat::GRAY];
    // 640 x 480 に近い大きさで、なるべく速いコマ数で開く。だめなら、カメラに任せる。
    let mut camera = FORMATS
        .iter()
        .find_map(|f| nokhwa::Camera::new(CameraIndex::Index(index as u32), RequestedFormat::with_formats(RequestedFormatType::Closest(CameraFormat::new_from(640, 480, *f, 60)), &FORMATS)).ok())
        .or_else(|| nokhwa::Camera::new(CameraIndex::Index(index as u32), RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestFrameRate, &FORMATS)).ok())
        .ok_or_else(|| t!("Could not open the camera."))?;
    camera.open_stream().map_err(|e| t!("Could not start the camera: {e}"))?;
    let mut failures = 0;
    while !quit.load(Ordering::Relaxed) {
        let frame = match camera.frame() {
            Ok(frame) => frame,
            Err(e) => {
                failures += 1;
                if failures > 30 {
                    let _ = camera.stop_stream();
                    return Err(t!("The camera stopped sending pictures: {e}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            }
        };
        failures = 0;
        let shot = std::time::Instant::now();
        let res = frame.resolution();
        let Some((gray, w, h)) = luma(frame.buffer(), res.width() as usize, res.height() as usize, frame.source_frame_format()) else { continue };
        let mut slot = latest.frame.lock().unwrap_or_else(|e| e.into_inner());
        slot.0 = Some((gray, w, h, shot));
        latest.ready.notify_all();
    }
    let _ = camera.stop_stream();
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn run(_index: usize, _stop: &AtomicBool, _send: &mpsc::SyncSender<Report>) -> Result<(), String> {
    Err(t!("Cameras are not supported on this system."))
}

/// カメラの絵から、明るさだけを取り出す。
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn luma(bytes: &[u8], w: usize, h: usize, format: nokhwa::utils::FrameFormat) -> Option<(Vec<u8>, usize, usize)> {
    use nokhwa::utils::FrameFormat;
    match format {
        FrameFormat::YUYV => (bytes.len() >= w * h * 2).then(|| (bytes.chunks_exact(2).take(w * h).map(|p| p[0]).collect(), w, h)),
        FrameFormat::NV12 | FrameFormat::GRAY => (bytes.len() >= w * h).then(|| (bytes[..w * h].to_vec(), w, h)),
        FrameFormat::RAWRGB | FrameFormat::RAWBGR => (bytes.len() >= w * h * 3).then(|| (bytes.chunks_exact(3).take(w * h).map(|p| ((p[0] as u32 + 2 * p[1] as u32 + p[2] as u32) / 4) as u8).collect(), w, h)),
        FrameFormat::MJPEG => {
            use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
            let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
            let gray = decoder.decode().ok()?;
            let (w, h) = decoder.dimensions()?;
            (gray.len() == w * h).then_some((gray, w, h))
        }
    }
}

/// 画面の側の、のぞき込みの状態。
#[derive(Default)]
pub struct PeekState {
    pub settings: PeekSettings,
    pub tracker: Option<Tracker>,
    pub smoother: Smoother,
    pub face: Option<Face>,
    /// カメラの絵の縦横比。
    pub aspect: f32,
    pub preview: Option<egui::TextureHandle>,
    pub error: Option<String>,
    /// 全画面で見せている。
    pub full: bool,
    /// 起動の引数で決めた目の位置（カメラを使わずに確かめるとき）。
    pub fixed: Option<V3>,
    /// つながっているカメラの名前（はじめて開いたときに調べる）。
    pub cameras: Option<Vec<String>>,
    /// 届いた顔を、この回の画面の更新で使う。
    seen: Option<(Option<Face>, f32)>,
    /// 前に顔の知らせが届いた時刻（秒）。
    last_report: Option<f64>,
}

impl PeekState {
    pub fn new(fixed: Option<V3>) -> PeekState {
        PeekState { fixed, ..Default::default() }
    }
}

impl PeekState {
    /// カメラから届いたものを受け取る。
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(tracker) = &self.tracker else { return };
        let mut stopped = false;
        while let Ok(report) = tracker.reports.try_recv() {
            match report {
                Report::Face(face, aspect, age) => {
                    self.face = face;
                    self.aspect = aspect;
                    self.seen = Some((face, age));
                }
                Report::Preview(gray, w, h) => {
                    let image = egui::ColorImage::from_gray([w, h], &gray);
                    match &mut self.preview {
                        Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                        None => self.preview = Some(ctx.load_texture("peek-camera", image, egui::TextureOptions::LINEAR)),
                    }
                }
                Report::Error(message) => {
                    self.error = Some(message);
                    stopped = true;
                }
            }
        }
        if stopped {
            self.tracker = None;
        }
    }

    pub fn running(&self) -> bool {
        self.tracker.is_some()
    }

    pub fn start(&mut self) {
        self.error = None;
        self.face = None;
        self.smoother = Smoother::default();
        self.tracker = Some(Tracker::start(self.settings.camera));
    }

    pub fn stop(&mut self) {
        self.tracker = None;
        self.face = None;
        self.preview = None;
    }

    /// この回の画面の更新での目の位置（mm）。`now` は時刻（秒）、`dt` は前の更新からの時間、`screen_h` は画面の高さ（mm）。
    pub fn eye(&mut self, now: f64, dt: f32, screen_h: f32) -> V3 {
        if let Some(eye) = self.fixed {
            return eye;
        }
        let aspect = if self.aspect > 0.0 { self.aspect } else { 4.0 / 3.0 };
        let stability = self.settings.stability;
        if !self.running() {
            self.smoother.miss(f32::INFINITY);
        } else if let Some((face, age)) = self.seen.take() {
            let since = self.last_report.map_or(0.1, |t| (now - t) as f32);
            self.last_report = Some(now);
            match face {
                Some(f) => self.smoother.measure(eye_from_face(f, aspect, screen_h, &self.settings), since, age, stability),
                None => self.smoother.miss(since),
            }
        }
        self.smoother.update(dt, stability)
    }
}


/// 出力の一覧での、のぞき込みの番号。
pub const PEEK_OUTPUT: usize = 5;

impl App {
    /// のぞき込みの絵を `rect` に描く。
    pub(super) fn peek_view(&mut self, ui: &mut egui::Ui, rect: Rect, rs: Option<&egui_wgpu::RenderState>) {
        let painter = ui.painter().with_clip_rect(rect);
        let Some(rs) = rs else {
            painter.text(rect.center(), Align2::CENTER_CENTER, t!("The box cannot be shown because wgpu is not available."), FontId::proportional(13.0), Color32::LIGHT_RED);
            return;
        };
        // マウスでも回せる（ドラッグで向き、ホイールで大きさ。立体の表示と同じ）。頭の動きは、その上に重なる。
        let response = ui.interact(rect, ui.scope_id().with("peek-view"), egui::Sense::click_and_drag());
        if response.dragged() {
            let delta = response.drag_delta();
            self.view_to = None;
            self.yaw -= delta.x * 0.01;
            self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.5, 1.5);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.3, 6.0);
            }
        }
        if self.peek.full && response.double_clicked() {
            self.set_peek_full(ui.ctx(), false);
        }
        self.peek.poll(ui.ctx());
        // 画面の上の、この表示の場所（mm）。分からなければ、この表示が画面いっぱいだとみなす。
        let screen_mm = self.peek.settings.screen_mm;
        let (window, screen_h) = ui.input(|i| {
            let monitor = i.viewport().monitor_size.filter(|m| m.x > 0.0 && m.y > 0.0);
            let inner = i.viewport().inner_rect;
            match (monitor, inner) {
                (Some(m), Some(inner)) => {
                    let k = screen_mm / m.x;
                    let at = |p: egui::Pos2| ((p.x + inner.min.x - m.x * 0.5) * k, (m.y * 0.5 - (p.y + inner.min.y)) * k);
                    let (l, t) = at(rect.min);
                    let (r, b) = at(rect.max);
                    (Window { left: l, right: r, bottom: b, top: t }, m.y * k)
                }
                _ => {
                    let h = screen_mm * rect.height() / rect.width().max(1.0);
                    (Window { left: -screen_mm * 0.5, right: screen_mm * 0.5, bottom: -h * 0.5, top: h * 0.5 }, h)
                }
            }
        });
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.1);
        let now = ui.input(|i| i.time);
        let eye = self.peek.eye(now, dt, screen_h);
        let mesh = self.template.build_mesh_at(self.fold, [1.0 - self.open[0], 1.0 - self.open[1]]);
        let (center, radius) = bounding_sphere(&mesh);
        let (view_proj, light) = window_view(window, eye, center, radius, self.yaw, self.pitch, self.zoom);
        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round().max(2.0) as u32, (rect.height() * ppp).round().max(2.0) as u32];
        let holes = self.holes();
        let renderer = self.renderer.get_or_insert_with(|| BoxRenderer::new(rs));
        if let Some(id) = renderer.render(rs, size, &mesh, view_proj, light, holes) {
            painter.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        // 小さな知らせ（顔が見えているか）。
        let note = if self.peek.fixed.is_some() {
            None
        } else if !self.peek.running() {
            Some(t!("The camera is off. The box is shown from the front."))
        } else if self.peek.face.is_none() {
            Some(t!("Looking for a face…"))
        } else {
            None
        };
        if let Some(note) = note {
            painter.text(rect.left_bottom() + vec2(12.0, -12.0), Align2::LEFT_BOTTOM, note, FontId::proportional(12.0), ui.visuals().weak_text_color());
        }
        if self.peek.full {
            painter.text(rect.right_bottom() + vec2(-12.0, -12.0), Align2::RIGHT_BOTTOM, t!("Esc: back"), FontId::proportional(12.0), ui.visuals().weak_text_color());
        }
        if self.peek.running() || self.peek.fixed.is_none() {
            ui.ctx().request_repaint();
        }
    }

    /// 全画面ののぞき込み。ほかの欄は出さない。Esc かダブルクリックで戻る。
    pub(super) fn peek_full_screen(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rs = frame.wgpu_render_state().cloned();
        self.render_state.clone_from(&rs);
        self.refresh_art(ui.ctx(), rs.as_ref(), super::Shown::SOLID_ONLY);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui.visuals().window_stroke.color)).show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            self.peek_view(ui, rect, rs.as_ref());
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                self.set_peek_full(ui.ctx(), false);
            }
        });
    }

    pub(super) fn set_peek_full(&mut self, ctx: &egui::Context, full: bool) {
        self.peek.full = full;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(full));
    }

    /// のぞき込みを見せていないあいだは、カメラを止める（使っていないカメラを点けたままにしない）。
    pub(super) fn settle_peek(&mut self) {
        let showing = self.workspace == super::Workspace::Output && self.output_kind == PEEK_OUTPUT && !self.library.open;
        if !showing {
            if self.peek.running() {
                self.peek.stop();
            }
            if self.peek.full {
                self.peek.full = false;
            }
        }
    }

    /// 右の欄: のぞき込みの設定。
    pub(super) fn peek_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(t!("Shows the box as if it were behind the screen. A webcam watches where your face is: move your head to look around the box, and come closer to see it larger."));
        ui.small(t!("The camera picture is only used inside this app to find the face. It is not saved or sent anywhere."));
        ui.add_space(6.0);
        let cameras = self.peek.cameras.get_or_insert_with(cameras).clone();
        let s = &mut self.peek.settings;
        let before = s.clone();
        egui::Grid::new("peek").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(t!("Camera"));
            let name = cameras.get(s.camera).cloned().unwrap_or_else(|| if cameras.is_empty() { t!("No camera found").to_owned() } else { t!("Camera {}", s.camera + 1) });
            egui::ComboBox::from_id_salt("peek-camera").width(190.0).selected_text(name).show_ui(ui, |ui| {
                for (i, name) in cameras.iter().enumerate() {
                    ui.selectable_value(&mut s.camera, i, name);
                }
            });
            ui.end_row();
            ui.label(t!("Screen width"));
            ui.horizontal(|ui| {
                ui.add_sized([80.0, ui.spacing().interact_size.y], egui::DragValue::new(&mut s.screen_mm).range(150.0..=3000.0).speed(2.0).max_decimals(0).suffix(" mm"));
                ui.menu_button(t!("Presets"), |ui| {
                    for (label, mm) in SCREENS {
                        if ui.button(format!("{} ({mm:.0} mm)", pakenoba_i18n::tr(label))).clicked() {
                            s.screen_mm = mm;
                            ui.close();
                        }
                    }
                });
            });
            ui.end_row();
            ui.label(t!("Strength"));
            ui.add(egui::Slider::new(&mut s.strength, 0.3..=3.0).max_decimals(1));
            ui.end_row();
            ui.label(t!("Stabilization"));
            ui.add(egui::Slider::new(&mut s.stability, 0.0..=1.0).max_decimals(2)).on_hover_text(t!("Higher values hold the view steadier when your head is still, with a little more delay"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut s.mirror, t!("Flip left and right"));
            ui.end_row();
        });
        ui.small(t!("Set the visible width of your screen so the depth looks right. If the box moves the wrong way, turn on \"Flip left and right\"."));
        let changed_camera = before.camera != self.peek.settings.camera;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if self.peek.running() {
                if ui.button(t!("Turn the camera off")).clicked() {
                    self.peek.stop();
                }
            } else if ui.add_enabled(!cameras.is_empty(), egui::Button::new(t!("Turn the camera on"))).on_disabled_hover_text(t!("No camera found")).clicked() {
                self.peek.start();
            }
            if ui.button(t!("Show full screen")).clicked() {
                if !self.peek.running() && !cameras.is_empty() {
                    self.peek.start();
                }
                self.set_peek_full(ui.ctx(), true);
            }
        });
        if changed_camera && self.peek.running() {
            self.peek.start();
        }
        if let Some(error) = &self.peek.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        // カメラの絵と、見つけた顔の枠。
        if let Some(texture) = &self.peek.preview {
            let size = texture.size_vec2();
            let width = ui.available_width().min(280.0);
            let (rect, _) = ui.allocate_exact_size(vec2(width, width * size.y / size.x.max(1.0)), egui::Sense::hover());
            ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            if let Some(face) = self.peek.face {
                let c = rect.min + vec2(face.cx * rect.width(), face.cy * rect.height());
                let half = face.w * rect.width() * 0.5;
                ui.painter().rect_stroke(Rect::from_center_size(c, vec2(half * 2.0, half * 2.0)), 2.0, (1.5, ui.visuals().selection.bg_fill), egui::StrokeKind::Middle);
            }
            ui.small(match self.peek.face {
                Some(_) => t!("A face was found."),
                None => t!("Looking for a face…"),
            });
        }
        ui.small(t!("Drag on the box to turn it and scroll to resize it, together with your head movement. Esc or a double-click leaves full screen."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(m: &Mat4, p: V3) -> [f32; 3] {
        let v = [p[0], p[1], p[2], 1.0];
        let out: Vec<f32> = (0..4).map(|r| (0..4).map(|c| m[c][r] * v[c]).sum()).collect();
        [out[0] / out[3], out[1] / out[3], out[2] / out[3]]
    }

    const SCREEN: Window = Window { left: -265.0, right: 265.0, bottom: -150.0, top: 150.0 };

    #[test]
    fn 画面の上の点は_目がどこにあっても同じ場所に映る() {
        // 窓の上にある点は、目を動かしても画面の上の同じ場所に出る（窓の見え方の決まり）。
        for eye in [[0.0, 0.0, 600.0], [200.0, -80.0, 450.0], [-300.0, 120.0, 900.0]] {
            let (m, _) = window_view(SCREEN, eye, [0.0; 3], 100.0, 0.0, 0.0, 1.0);
            // 箱の座標から、画面の上の点 (100, 50, 0) に来る点を逆算するかわりに、
            // 箱を置かない投影だけで確かめる。
            let k = 0.05f32;
            let near = (eye[2] * k).max(1.0);
            let s = near / eye[2];
            let p = frustum((SCREEN.left - eye[0]) * s, (SCREEN.right - eye[0]) * s, (SCREEN.bottom - eye[1]) * s, (SCREEN.top - eye[1]) * s, near, eye[2] + 1000.0);
            let q = project(&mat4_mul(&p, &translate([-eye[0], -eye[1], -eye[2]])), [100.0, 50.0, 0.0]);
            assert!((q[0] - 100.0 / 265.0).abs() < 1e-4 && (q[1] - 50.0 / 150.0).abs() < 1e-4, "{eye:?} {q:?}");
            assert!(m.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn 右へ動くと_左の側面は隠れていく() {
        // 左の側面の奥の点と手前の点の、画面での横の距離（左の側面が見えている幅）を比べる。
        // 窓の向こうの物を右からのぞくと、右の側面が見え、左の側面は縮んで隠れていく。
        let gap = |eye: V3| {
            let (m, _) = window_view(SCREEN, eye, [0.0; 3], 100.0, 0.0, 0.0, 1.0);
            let left_back = project(&m, [-50.0, 0.0, -50.0]);
            let left_front = project(&m, [-50.0, 0.0, 50.0]);
            left_front[0] - left_back[0]
        };
        assert!(gap([250.0, 0.0, 600.0]) < gap([-250.0, 0.0, 600.0]));
    }

    #[test]
    fn 近づくと大きく見える() {
        let width = |z: f32| {
            let (m, _) = window_view(SCREEN, [0.0, 0.0, z], [0.0; 3], 100.0, 0.0, 0.0, 1.0);
            project(&m, [50.0, 0.0, 50.0])[0] - project(&m, [-50.0, 0.0, 50.0])[0]
        };
        assert!(width(350.0) > width(900.0));
    }

    #[test]
    fn 顔の場所から目の位置を見積もる() {
        let s = PeekSettings::default();
        // 絵の中央で、幅の 2 割の顔は、正面のおよそ 70 cm。
        let eye = eye_from_face(Face { cx: 0.5, cy: 0.5, w: 0.2 }, 4.0 / 3.0, 300.0, &s);
        assert!(eye[0].abs() < 1e-3 && (600.0..800.0).contains(&eye[2]), "{eye:?}");
        // 絵の左にいる人は、見ている人の右（x がプラス）。左右逆のカメラでは逆。
        assert!(eye_from_face(Face { cx: 0.3, cy: 0.5, w: 0.2 }, 4.0 / 3.0, 300.0, &s)[0] > 0.0);
        assert!(eye_from_face(Face { cx: 0.3, cy: 0.5, w: 0.2 }, 4.0 / 3.0, 300.0, &PeekSettings { mirror: true, ..s.clone() })[0] < 0.0);
        // 顔が小さいほど遠い。
        assert!(eye_from_face(Face { cx: 0.5, cy: 0.5, w: 0.1 }, 4.0 / 3.0, 300.0, &s)[2] > eye[2]);
    }

    #[test]
    fn 見失ったら_しばらく待って_正面へ戻る() {
        let mut s = Smoother::default();
        for _ in 0..30 {
            s.measure([200.0, 0.0, 500.0], 0.05, 0.03, 0.3);
            for _ in 0..3 {
                s.update(1.0 / 60.0, 0.3);
            }
        }
        assert!((s.eye[0] - 200.0).abs() < 2.0, "{:?}", s.eye);
        s.miss(0.5);
        for _ in 0..30 {
            s.update(1.0 / 60.0, 0.3);
        }
        assert!((s.eye[0] - 200.0).abs() < 2.0, "すぐには戻らない");
        s.miss(1.5);
        for _ in 0..600 {
            s.update(1.0 / 60.0, 0.3);
        }
        assert!(s.eye[0].abs() < 10.0, "{:?}", s.eye);
    }

    #[test]
    fn 補正が0なら_測った位置をそのまま使う() {
        let mut s = Smoother::default();
        s.measure([10.0, 0.0, 600.0], 0.05, 0.0, 0.0);
        s.measure([50.0, 0.0, 600.0], 0.05, 0.0, 0.0);
        let e = s.update(0.0, 0.0);
        assert!(e[0] >= 50.0, "遅れない（先読みで少し先へ出てよい）: {e:?}");
    }

    #[test]
    fn 動いている頭には_先読みで遅れない() {
        // 毎秒 300 mm で動く頭を、20 回／秒、50 ms 遅れのコマで測る。表示は、いまの本当の位置から 15 mm 以内。
        let mut s = Smoother::default();
        let mut t = 0.0f32;
        let mut worst = 0.0f32;
        for n in 0..60 {
            let shot = t - 0.05;
            s.measure([300.0 * shot, 0.0, 600.0], 0.05, 0.05, 0.3);
            for _ in 0..3 {
                t += 1.0 / 60.0;
                let e = s.update(1.0 / 60.0, 0.3);
                if n > 20 {
                    worst = worst.max((e[0] - 300.0 * t).abs());
                }
            }
        }
        assert!(worst < 15.0, "{worst}");
    }

    #[test]
    fn ぶれ補正で_止まった頭のふるえが小さくなる() {
        // 20 回／秒の測定に、±8 mm のふるえを足す。補正が強いほど、表示のふるえは小さい。
        let shake = |stability: f32| {
            let mut s = Smoother::default();
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for n in 0..200 {
                let noise = if n % 2 == 0 { 8.0 } else { -8.0 };
                s.measure([100.0 + noise, 0.0, 600.0 + noise * 3.0], 0.05, 0.03, stability);
                for _ in 0..3 {
                    let e = s.update(1.0 / 60.0, stability);
                    if n > 100 {
                        lo = lo.min(e[0]);
                        hi = hi.max(e[0]);
                    }
                }
            }
            hi - lo
        };
        assert!(shake(0.9) < shake(0.1) * 0.5, "{} {}", shake(0.9), shake(0.1));
    }

    #[test]
    fn 縮めると_明るさの平均になる() {
        let gray: Vec<u8> = (0..16).map(|i| if i % 2 == 0 { 0 } else { 200 }).collect();
        let (out, w, h) = shrink(&gray, 4, 4, 2);
        assert_eq!((w, h), (2, 2));
        assert!(out.iter().all(|v| *v == 100));
    }

    #[test]
    fn モデルが読めて_顔のない絵では何も見つけない() {
        let mut finder = FaceFinder::new().unwrap();
        assert_eq!(finder.find(&vec![128u8; 320 * 240], 320, 240), None);
    }
}
