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
}

impl Default for PeekSettings {
    fn default() -> Self {
        PeekSettings { camera: 0, screen_mm: 530.0, strength: 1.0, mirror: false, camera_fov: 60.0 }
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

/// 目の位置を、なめらかにする。大きく動いたときはすぐ追い、小さなふるえは抑える。
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    pub eye: V3,
    /// 顔を見失ってからの時間（秒）。
    lost: f32,
}

impl Default for Smoother {
    fn default() -> Self {
        Smoother { eye: REST_EYE, lost: f32::INFINITY }
    }
}

impl Smoother {
    /// 新しい目の位置（見つからなければ `None`）と、前からの時間（秒）。
    pub fn update(&mut self, seen: Option<V3>, dt: f32) -> V3 {
        let target = match seen {
            Some(eye) => {
                self.lost = 0.0;
                eye
            }
            None => {
                self.lost += dt;
                // 少しのあいだは、その場で待つ。それより長く見失ったら、正面へゆっくり戻る。
                if self.lost < 1.5 { self.eye } else { REST_EYE }
            }
        };
        let gap = (0..3).map(|i| (target[i] - self.eye[i]).powi(2)).sum::<f32>().sqrt();
        let rate = if seen.is_some() { (0.25 + gap / 60.0).min(0.85) } else { 0.04 };
        // 画面の更新の間隔によらず、同じ速さで寄せる（60 コマ／秒を基準にした割合）。
        let a = 1.0 - (1.0 - rate).powf((dt * 60.0).clamp(0.1, 6.0));
        for (e, t) in self.eye.iter_mut().zip(target) {
            *e += (t - *e) * a;
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
    /// 顔（見つからなければ `None`）と、カメラの絵の縦横比。
    Face(Option<Face>, f32),
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

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn run(index: usize, stop: &AtomicBool, send: &mpsc::SyncSender<Report>) -> Result<(), String> {
    use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
    const FORMATS: [FrameFormat; 4] = [FrameFormat::YUYV, FrameFormat::MJPEG, FrameFormat::NV12, FrameFormat::GRAY];
    let mut finder = FaceFinder::new()?;
    // 640 x 480 に近い大きさで開く。だめなら、カメラに任せる。
    let mut camera = FORMATS
        .iter()
        .find_map(|f| nokhwa::Camera::new(CameraIndex::Index(index as u32), RequestedFormat::with_formats(RequestedFormatType::Closest(CameraFormat::new_from(640, 480, *f, 30)), &FORMATS)).ok())
        .or_else(|| nokhwa::Camera::new(CameraIndex::Index(index as u32), RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestFrameRate, &FORMATS)).ok())
        .ok_or_else(|| t!("Could not open the camera."))?;
    camera.open_stream().map_err(|e| t!("Could not start the camera: {e}"))?;
    let mut failures = 0;
    while !stop.load(Ordering::Relaxed) {
        let frame = match camera.frame() {
            Ok(frame) => frame,
            Err(e) => {
                failures += 1;
                if failures > 30 {
                    return Err(t!("The camera stopped sending pictures: {e}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            }
        };
        failures = 0;
        let res = frame.resolution();
        let Some((gray, w, h)) = luma(frame.buffer(), res.width() as usize, res.height() as usize, frame.source_frame_format()) else { continue };
        let (small, sw, sh) = shrink(&gray, w, h, DETECT_WIDTH);
        let face = finder.find(&small, sw, sh);
        let (preview, pw, ph) = shrink(&small, sw, sh, PREVIEW_WIDTH);
        if send.try_send(Report::Face(face, w as f32 / h.max(1) as f32)).is_err() && stop.load(Ordering::Relaxed) {
            break;
        }
        let _ = send.try_send(Report::Preview(preview, pw, ph));
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
    seen: Option<Option<Face>>,
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
                Report::Face(face, aspect) => {
                    self.face = face;
                    self.aspect = aspect;
                    self.seen = Some(face);
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

    /// この回の画面の更新での目の位置（mm）。`screen_h` は画面の高さ（mm）。
    pub fn eye(&mut self, dt: f32, screen_h: f32) -> V3 {
        if let Some(eye) = self.fixed {
            return eye;
        }
        let aspect = if self.aspect > 0.0 { self.aspect } else { 4.0 / 3.0 };
        // 新しい顔が届いていなければ、前の顔のまま寄せ続ける（見失ったときだけ数える）。
        let seen = match self.seen.take() {
            Some(face) => face.map(|f| eye_from_face(f, aspect, screen_h, &self.settings)),
            None => self.face.map(|f| eye_from_face(f, aspect, screen_h, &self.settings)),
        };
        self.smoother.update(if self.running() { seen } else { None }, dt)
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
        let eye = self.peek.eye(dt, screen_h);
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
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
            self.peek_view(ui, rect, rs.as_ref());
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape || response.double_clicked() {
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
        ui.small(t!("Drag and scroll on the box in the Structure or Design step to set the angle and size. Esc or a double-click leaves full screen."));
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
        for _ in 0..120 {
            s.update(Some([200.0, 0.0, 500.0]), 1.0 / 60.0);
        }
        assert!((s.eye[0] - 200.0).abs() < 1.0);
        for _ in 0..30 {
            s.update(None, 1.0 / 60.0);
        }
        assert!((s.eye[0] - 200.0).abs() < 1.0, "すぐには戻らない");
        for _ in 0..600 {
            s.update(None, 1.0 / 60.0);
        }
        assert!(s.eye[0].abs() < 10.0, "{:?}", s.eye);
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
