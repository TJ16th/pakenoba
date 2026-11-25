//! 出力の工程の、型紙の PSD 以外の書き出し。
//!
//! - 16:9 の画像（スライド用）: いまの視点の箱を、PNG で。背景は、明るい灰色、白、透明から選ぶ。
//! - 箱開けの動画: 閉じた箱が開く動きを、GIF で。少しずつ回り込む動きも付けられる。
//!   1 回の画面の更新で 1 コマずつ描き、別のスレッドで GIF にまとめる（書き出しの間も、画面は止まらない）。
//! - 実寸の型紙: 展開図を実寸で、A4 か A3 に分けた PDF に（print.rs）。
//!
//! どれも、選んでいる柄の枠、カーソルの下の枠、ガイドの線を消した絵で描く。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use eframe::egui;
use eframe::egui_wgpu;

use pakenoba_core::math::{Mat4, look_at, mat4_mul};
use pakenoba_core::template::bounding_sphere;

use super::{App, Shown};
use crate::art::{compose, mirror, mirror_gray, sheet_lines};
use crate::gpu::BoxRenderer;
use crate::paint::{Canvas, Marks, paint};
use crate::print::{Paper, PrintSettings, ReadyColor, ReadySide, SheetPrint, print_pdf, print_ready_pdf, tile};

/// 出力の種類（左の一覧の順）。
pub(super) const OUTPUTS: [(&str, bool); 9] = [("Dieline (PSD)", true), ("16:9 image (for slides)", true), ("Box-opening video", true), ("Store shelf picture", false), ("Full-size dieline (printed on A4 or A3 sheets)", true), ("Peek-in view (webcam)", true), ("Print-ready PDF (full size, with finish plates)", true), ("Images for generative AI (ControlNet, Firefly)", true), ("3D model (GLB, OBJ)", true)];
/// 入稿用の PDF の出力の番号。
pub(super) const READY_OUTPUT: usize = 6;
/// 生成AI用の画像の出力の番号。
pub(super) const VIEW_OUTPUT: usize = 7;
/// 3D の書き出しの番号。
pub(super) const MODEL_OUTPUT: usize = 8;

/// 画像の大きさ（16:9）。
const SHOT_SIZES: [(&str, [u32; 2]); 2] = [("1920 x 1080", [1920, 1080]), ("3840 x 2160 (4K)", [3840, 2160])];
/// 背景の色。最後は透明。
const BACKGROUNDS: [(&str, [f32; 4]); 3] = [("Light gray", [0.93, 0.93, 0.92, 1.0]), ("White", [1.0, 1.0, 1.0, 1.0]), ("Transparent", [0.0, 0.0, 0.0, 0.0])];
/// 動画の大きさと、1 秒のコマ数。
const MOVIE_SIZES: [(&str, [u32; 2]); 2] = [("960 x 540", [960, 540]), ("1280 x 720", [1280, 720])];
const MOVIE_FPS: u32 = 20;

/// 出力の設定。
#[derive(Clone)]
pub(super) struct OutputSettings {
    pub shot_size: usize,
    pub background: usize,
    pub movie_size: usize,
    pub movie_seconds: f32,
    /// 開けながら、少し回り込む。
    pub movie_turn: bool,
    pub print: PrintSettings,
    /// 型紙の PSD と入稿用の PDF の細かさ（dpi。100 は画面と同じ）。
    pub dpi: u32,
    /// 塗り足しの幅（mm。0 なら付けない）。型紙の PSD と入稿用の PDF。
    pub bleed: f32,
    /// 生成AI用の画像の大きさ（`viewgen::VIEW_SIZES` の番号）。
    pub view_size: usize,
    /// 生成した絵を面へ貼るとき、面ごとの明るさをそろえる。
    pub even_faces: bool,
    /// 入稿用の PDF の絵の色（RGB のままか、CMYK にするか）。
    pub color: ReadyColorChoice,
}

/// 入稿用の PDF の絵の色の選び方。
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum ReadyColorChoice {
    /// RGB のまま（sRGB）。
    #[default]
    Rgb,
    /// CMYK。内蔵のコート紙のプロファイル（FOGRA39 の測定値に合わせたもの。CC0）。
    Fogra39,
    /// CMYK。使う人が選んだ ICC プロファイルのファイル（Japan Color 2001 Coated など）。
    Icc(PathBuf),
}

impl ReadyColorChoice {
    /// 画面に出す名前。
    pub fn label(&self) -> String {
        match self {
            ReadyColorChoice::Rgb => t!("RGB (as on screen)").to_owned(),
            ReadyColorChoice::Fogra39 => t!("CMYK: coated paper (built in, FOGRA39)").to_owned(),
            ReadyColorChoice::Icc(path) => t!("CMYK: {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        }
    }

    /// CMYK のプロファイルを用意する（RGB なら `None`）。ファイルは、ここで読んで確かめる。
    pub fn profile(&self) -> Result<Option<std::borrow::Cow<'static, craft_cms::Profile>>, String> {
        match self {
            ReadyColorChoice::Rgb => Ok(None),
            ReadyColorChoice::Fogra39 => Ok(Some(std::borrow::Cow::Borrowed(craft_cms::builtin::Builtin::CoatedFogra39.profile()))),
            ReadyColorChoice::Icc(path) => {
                let bytes = std::fs::read(path).map_err(|e| t!("Could not read the ICC profile {}: {e}", path.display()))?;
                let profile = craft_cms::Profile::parse(&bytes).map_err(|e| t!("{} is not an ICC profile PAKENOBA can read: {e:?}", path.display()))?;
                if profile.color_space.channels() != 4 || !profile.can_be_destination() {
                    return Err(t!("{} is not a CMYK output profile", path.display()));
                }
                Ok(Some(std::borrow::Cow::Owned(profile)))
            }
        }
    }
}

impl Default for OutputSettings {
    fn default() -> Self {
        OutputSettings { shot_size: 0, background: 0, movie_size: 0, movie_seconds: 3.0, movie_turn: true, print: PrintSettings::default(), dpi: 350, bleed: 3.0, view_size: 0, even_faces: true, color: ReadyColorChoice::Rgb }
    }
}

/// 画面で再生している、箱開け動画のプレビュー。
pub(super) struct MoviePreview {
    start: std::time::Instant,
    /// はじめの視点（いまの視点）。
    yaw: f32,
    /// 隠している、選んだ柄の枠とガイド（終わったら戻す）。
    kept: Option<(Option<usize>, Vec<usize>, bool)>,
    pub(super) texture: Option<egui::TextureHandle>,
}

/// プレビューの絵の大きさ（動画と同じ 16:9）。
const PREVIEW_SIZE: [u32; 2] = [640, 360];

/// 書き出している途中の動画。
pub(super) struct MovieJob {
    frame: u32,
    total: u32,
    yaw: f32,
    /// 書き出しのあいだ隠している、選んだ柄の枠とガイド（終わったら戻す）。
    kept: Option<(Option<usize>, Vec<usize>, bool)>,
    /// 描いたが、まだ渡せていないコマ（渡し口が詰まっているとき。画面を止めないよう、待たずに次の画面で渡す）。
    pending: Option<(u32, Vec<u8>)>,
    send: Option<mpsc::SyncSender<(u32, Vec<u8>)>>,
    writer: Option<JoinHandle<Result<(), String>>>,
    path: PathBuf,
}

/// 動画のコマ `frame` の、開け具合（0〜1）と、回り込む角度（ラジアン）。
/// はじめの 1 割は閉じたまま、そのあと 6 割で開け、残りは開けたまま。
pub(super) fn movie_pose(frame: u32, total: u32, turn: bool) -> (f32, f32) {
    let t = frame as f32 / (total.max(2) - 1) as f32;
    let open = ((t - 0.1) / 0.6).clamp(0.0, 1.0);
    let eased = open * open * (3.0 - 2.0 * open);
    let swing = if turn { (t - 0.5) * 30f32.to_radians() } else { 0.0 };
    (eased, swing)
}

/// 箱の視点（立体の表示と同じ計算）。返すのは、投影と視点をかけたもの。
/// `view_aspect` は立体の表示の縦横比。画像は、立体の表示の中央を `aspect` の縦横比で切り抜いた範囲にする
/// （出力の工程で、立体の表示の上に描いた枠の中と同じになる）。
fn camera(mesh: &[pakenoba_core::template::Vertex], aspect: f32, view_aspect: f32, yaw: f32, pitch: f32, zoom: f32, ortho: bool) -> Mat4 {
    let (center, radius) = bounding_sphere(mesh);
    let fov = 30f32.to_radians();
    let dist = radius / (fov * 0.5).sin() * 1.1 / zoom / view_aspect.min(1.0);
    // 枠の高さが、立体の表示の高さのどれだけか。そのぶん、縦の画角を狭める。
    let share = if view_aspect > aspect { 1.0 } else { view_aspect / aspect };
    let fov_out = 2.0 * ((fov * 0.5).tan() * share).atan();
    let (dir, up) = super::views::orbit(yaw, pitch);
    let eye = [center[0] + dir[0] * dist, center[1] + dir[1] * dist, center[2] + dir[2] * dist];
    let view = look_at(eye, center, up);
    // 平行投影の表示なら、画像も平行投影にする。
    let proj = super::views::projection(ortho, fov_out, aspect, dist, radius);
    mat4_mul(&proj, &view)
}

/// 名前の拡張子をそろえる。
fn with_extension(path: PathBuf, ext: &str) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
        return path;
    }
    let mut name = path.file_name().map(|n| n.to_owned()).unwrap_or_default();
    name.push(format!(".{ext}"));
    path.with_file_name(name)
}

/// RGBA を PNG にする。
pub(super) fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

impl App {
    /// 保存先を選んでもらう。`what` はダイアログの題、`ext` は拡張子、`suffix` ははじめの名前に足す言葉。
    pub(super) fn ask_output(&mut self, what: &str, filter: &str, ext: &str, suffix: &str) -> Option<PathBuf> {
        if let Some(answer) = self.answer_export.take() {
            return Some(with_extension(answer, ext));
        }
        self.track("export", serde_json::json!({ "format": ext }));
        let project = self.file.path.as_deref().and_then(Path::file_stem).map_or_else(|| t!("PAKENOBA").to_owned(), |s| s.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new().set_title(what).add_filter(filter, &[ext]).set_file_name(format!("{project}_{suffix}.{ext}"));
        if let Some(dir) = self.file.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(|p| with_extension(p, ext))
    }

    /// 書き出し用に、枠とガイドのない絵を、箱に貼り直す。戻すときは `restore_marks`。
    fn clean_art(&mut self, ctx: &egui::Context, rs: &egui_wgpu::RenderState) -> (Option<usize>, Vec<usize>, bool) {
        let kept = (self.selected.take(), std::mem::take(&mut self.extra), self.show_guides);
        self.show_guides = false;
        self.hover_decal = None;
        (self.art_dirty, self.back_dirty) = (true, true);
        self.refresh_art(ctx, Some(rs), Shown::SOLID_ONLY);
        kept
    }

    fn restore_marks(&mut self, kept: (Option<usize>, Vec<usize>, bool)) {
        (self.selected, self.extra, self.show_guides) = kept;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// いまの視点の箱を、`size` の画像に描く。`frame` が真なら、開けきった箱が収まる視点にする（動画で、コマごとに大きさが変わらないように）。
    fn render_box(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2], open: [f32; 2], yaw: f32, background: [f32; 4], frame: bool) -> Result<Vec<u8>, String> {
        let mesh = self.template.build_mesh_at(self.fold, [1.0 - open[0], 1.0 - open[1]]);
        let fit = if frame { self.template.build_mesh_at(self.fold, [0.0, 1.0 - open[1]]) } else { mesh.clone() };
        let aspect = size[0] as f32 / size[1] as f32;
        // 動画は、開けきった箱が画面いっぱいに収まる視点（向きだけ、いまの視点から取る）。
        let (view_aspect, zoom) = if frame { (aspect, 1.25) } else { (self.solid_aspect, self.zoom) };
        let view_proj = camera(&fit, aspect, view_aspect, yaw, self.pitch, zoom, self.ortho);
        let holes = self.holes();
        let look = self.look(yaw);
        self.renderer.get_or_insert_with(|| BoxRenderer::new(rs)).capture(rs, size, &mesh, view_proj, &look, holes, background)
    }

    /// 抜き穴を、絵柄の座標（0〜1）に直したもの。
    pub(super) fn holes(&self) -> [[f32; 4]; 2] {
        let t = &self.template;
        let (sheet_w, sheet_h) = (t.max[0] - t.min[0], t.max[1] - t.min[1]);
        let mut holes = [[0.0f32; 4]; 2];
        for (slot, h) in holes.iter_mut().zip(&t.holes) {
            *slot = [(h[0] - t.min[0]) / sheet_w, (t.max[1] - h[1]) / sheet_h, h[2] / sheet_w, h[3] / sheet_h];
        }
        holes
    }

    /// 16:9 の画像を書き出す。
    pub(super) fn export_image(&mut self, ctx: &egui::Context) {
        let Some(rs) = self.render_state.clone() else { return self.say(t!("This PC cannot draw the box, so the image cannot be exported.")) };
        let Some(path) = self.ask_output(t!("Save the 16:9 image"), "PNG", "png", t!("Image")) else { return };
        let size = SHOT_SIZES[self.output.shot_size].1;
        let background = BACKGROUNDS[self.output.background].1;
        let kept = self.clean_art(ctx, &rs);
        let result = self.render_box(&rs, size, self.open, self.yaw, background, false).and_then(|rgba| png_bytes(size[0], size[1], &rgba));
        self.restore_marks(kept);
        match result.and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string())) {
            Ok(()) => self.say(t!("Saved {} ({} x {} px)", path.display(), size[0], size[1])),
            Err(e) => self.say(t!("Could not save the image: {e}")),
        }
    }

    /// 箱開けの動画を書き出しはじめる。コマは `movie_frame` で 1 つずつ描く。
    pub(super) fn export_movie(&mut self) {
        if self.movie.is_some() {
            return;
        }
        if self.render_state.is_none() {
            return self.say(t!("This PC cannot draw the box, so the video cannot be exported."));
        }
        if !self.template.can_open_at(0) {
            return self.say(t!("This box type has no opening motion."));
        }
        let Some(path) = self.ask_output(t!("Save the box-opening video"), t!("GIF animation"), "gif", t!("box-opening")) else { return };
        let size = MOVIE_SIZES[self.output.movie_size].1;
        let total = ((self.output.movie_seconds * MOVIE_FPS as f32).round() as u32).max(10);
        // コマを受け取って GIF にまとめる。色を 256 色に絞るのが重いので、何人かで手分けして絞り、書く係が順に並べて書く。
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).saturating_sub(1).clamp(1, 6);
        let (send, receive) = mpsc::sync_channel::<(u32, Vec<u8>)>(workers * 2);
        let file_path = path.clone();
        let writer = std::thread::Builder::new()
            .name("pakenoba-movie".to_owned())
            .spawn(move || -> Result<(), String> {
                let file = std::fs::File::create(&file_path).map_err(|e| e.to_string())?;
                let mut encoder = gif::Encoder::new(std::io::BufWriter::new(file), size[0] as u16, size[1] as u16, &[]).map_err(|e| e.to_string())?;
                encoder.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                let receive = std::sync::Arc::new(std::sync::Mutex::new(receive));
                let (done_send, done) = mpsc::channel::<(u32, gif::Frame<'static>)>();
                let mut pool = Vec::new();
                for _ in 0..workers {
                    let (receive, done_send) = (receive.clone(), done_send.clone());
                    pool.push(std::thread::spawn(move || {
                        loop {
                            let next = receive.lock().unwrap_or_else(|e| e.into_inner()).recv();
                            let Ok((index, mut rgba)) = next else { return };
                            let mut frame = gif::Frame::from_rgba_speed(size[0] as u16, size[1] as u16, &mut rgba, 10);
                            frame.delay = (100 / MOVIE_FPS) as u16;
                            if done_send.send((index, frame)).is_err() {
                                return;
                            }
                        }
                    }));
                }
                drop(done_send);
                // 手分けした結果は、終わった順に届く。番号の順に並べて書く。
                let mut waiting = std::collections::BTreeMap::new();
                let mut next = 0u32;
                for (index, frame) in done {
                    waiting.insert(index, frame);
                    while let Some(frame) = waiting.remove(&next) {
                        encoder.write_frame(&frame).map_err(|e| e.to_string())?;
                        next += 1;
                    }
                }
                for worker in pool {
                    let _ = worker.join();
                }
                Ok(())
            })
            .map_err(|e| e.to_string());
        match writer {
            Ok(writer) => {
                self.movie = Some(MovieJob { frame: 0, total, yaw: self.yaw, kept: None, pending: None, send: Some(send), writer: Some(writer), path });
                self.say(t!("Exporting the video…"));
            }
            Err(e) => self.say(t!("Could not export the video: {e}")),
        }
    }

    /// 箱開け動画のプレビューを始める（もう一度押すと止める）。
    pub(super) fn toggle_movie_preview(&mut self) {
        if self.movie_preview.is_some() {
            return self.stop_movie_preview();
        }
        if self.render_state.is_none() || !self.template.can_open_at(0) {
            return;
        }
        self.movie_preview = Some(MoviePreview { start: std::time::Instant::now(), yaw: self.yaw, kept: None, texture: None });
    }

    fn stop_movie_preview(&mut self) {
        if let Some(kept) = self.movie_preview.take().and_then(|p| p.kept) {
            self.restore_marks(kept);
        }
    }

    /// プレビューの 1 コマを描く（画面の更新ごと）。書き出すときと同じ視点と背景で、小さく描く。
    pub(super) fn movie_preview_frame(&mut self, ctx: &egui::Context) {
        let Some(preview) = &self.movie_preview else { return };
        if self.workspace != super::Workspace::Output || self.movie.is_some() {
            return self.stop_movie_preview();
        }
        let Some(rs) = self.render_state.clone() else { return };
        let total = ((self.output.movie_seconds * MOVIE_FPS as f32).round() as u32).max(10);
        // 最後のコマのあと 0.8 秒止めてから、はじめに戻ってくり返す。
        let cycle = total as f32 / MOVIE_FPS as f32 + 0.8;
        let t = preview.start.elapsed().as_secs_f32() % cycle;
        let frame = ((t * MOVIE_FPS as f32) as u32).min(total - 1);
        let yaw0 = preview.yaw;
        if preview.kept.is_none() {
            let kept = self.clean_art(ctx, &rs);
            if let Some(preview) = self.movie_preview.as_mut() {
                preview.kept = Some(kept);
            }
        }
        let (open, swing) = movie_pose(frame, total, self.output.movie_turn);
        let background = BACKGROUNDS[self.output.background.min(1)].1;
        if let Ok(rgba) = self.render_box(&rs, PREVIEW_SIZE, [open, 0.0], yaw0 + swing, background, true) {
            let image = egui::ColorImage::from_rgba_unmultiplied([PREVIEW_SIZE[0] as usize, PREVIEW_SIZE[1] as usize], &rgba);
            if let Some(preview) = self.movie_preview.as_mut() {
                match &mut preview.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => preview.texture = Some(ctx.load_texture("movie-preview", image, egui::TextureOptions::LINEAR)),
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(1000 / MOVIE_FPS as u64));
    }

    /// 立体の表示の上に、プレビューを重ねる（書き出す動画と同じ 16:9 の枠に）。押すと止める。
    pub(super) fn movie_preview_overlay(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let Some(texture) = self.movie_preview.as_ref().and_then(|p| p.texture.clone()) else { return };
        let target = PREVIEW_SIZE[0] as f32 / PREVIEW_SIZE[1] as f32;
        let aspect = rect.width() / rect.height();
        let frame = if aspect > target { egui::Rect::from_center_size(rect.center(), egui::vec2(rect.height() * target, rect.height())) } else { egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), rect.width() / target)) };
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(200));
        painter.image(texture.id(), frame, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        let label = painter.layout_no_wrap(t!("Preview (click to stop)").to_owned(), egui::FontId::proportional(13.0), egui::Color32::WHITE);
        let at = frame.left_top() + egui::vec2(10.0, 8.0);
        painter.rect_filled(egui::Rect::from_min_size(at - egui::vec2(6.0, 3.0), label.size() + egui::vec2(12.0, 6.0)), 4.0, egui::Color32::from_black_alpha(160));
        painter.galley(at, label, egui::Color32::WHITE);
        let response = ui.interact(rect, ui.scope_id().with("movie-preview"), egui::Sense::click());
        if response.clicked() {
            self.stop_movie_preview();
        }
    }

    /// いまの動きの設定を、名前を付けてプリセットに残す（同じ名前があれば上書き）。
    fn save_motion(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let preset = super::settings::MotionPreset { name: name.to_owned(), seconds: self.output.movie_seconds, turn: self.output.movie_turn, yaw: self.yaw, pitch: self.pitch, size: self.output.movie_size, background: self.output.background };
        match self.settings.motions.iter_mut().find(|m| m.name == name) {
            Some(old) => *old = preset,
            None => self.settings.motions.push(preset),
        }
        if let Err(e) = self.settings.save() {
            self.say(e);
        } else {
            self.say(t!("Saved the motion preset \"{name}\". It can be used in any project."));
        }
    }

    /// プリセットの動きにする（視点も、その向きへ回す）。
    fn use_motion(&mut self, index: usize) {
        let Some(m) = self.settings.motions.get(index).cloned() else { return };
        self.output.movie_seconds = m.seconds;
        self.output.movie_turn = m.turn;
        self.output.movie_size = m.size.min(MOVIE_SIZES.len() - 1);
        self.output.background = m.background.min(1);
        self.look_from(m.yaw, m.pitch);
        if let Some(preview) = self.movie_preview.as_mut() {
            preview.yaw = m.yaw;
            preview.start = std::time::Instant::now();
        }
    }

    /// 動画のコマを 1 つ描いて渡す。描き終わったら、書き終わるのを待って知らせる。画面の更新ごとに呼ぶ。
    pub(super) fn movie_frame(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.movie else { return };
        ctx.request_repaint();
        // 前の画面で渡せなかったコマがあれば、待たずに渡してみる（詰まっていれば、次の画面でまた）。
        if let Some((index, rgba)) = job.pending.take() {
            match job.send.as_ref().map(|send| send.try_send((index, rgba))) {
                Some(Err(mpsc::TrySendError::Full(back))) => {
                    job.pending = Some(back);
                    return;
                }
                Some(Err(mpsc::TrySendError::Disconnected(_))) | None => job.frame = job.total,
                Some(Ok(())) => {}
            }
        }
        if job.frame < job.total {
            let (frame, total, yaw0) = (job.frame, job.total, job.yaw);
            let Some(rs) = self.render_state.clone() else { return };
            // 柄の絵は、はじめに 1 回だけ、選んだ枠とガイドを消して作り直す（コマごとに作り直さない）。
            if self.movie.as_ref().is_some_and(|job| job.kept.is_none()) {
                let kept = self.clean_art(ctx, &rs);
                if let Some(job) = self.movie.as_mut() {
                    job.kept = Some(kept);
                }
            }
            let (open, swing) = movie_pose(frame, total, self.output.movie_turn);
            let size = MOVIE_SIZES[self.output.movie_size].1;
            let background = BACKGROUNDS[self.output.background.min(1)].1;
            let rendered = self.render_box(&rs, size, [open, 0.0], yaw0 + swing, background, true);
            let job = self.movie.as_mut().expect("movie job present");
            match rendered {
                Ok(rgba) => {
                    match job.send.as_ref().map(|send| send.try_send((frame, rgba))) {
                        Some(Ok(())) => {}
                        Some(Err(mpsc::TrySendError::Full(back))) => job.pending = Some(back),
                        _ => job.frame = job.total,
                    }
                    job.frame = (job.frame + 1).min(job.total);
                    let progress = t!("Exporting the video… frame {} of {}", job.frame, job.total);
                    self.say(progress);
                }
                Err(e) => {
                    let kept = self.movie.take().and_then(|job| job.kept);
                    if let Some(kept) = kept {
                        self.restore_marks(kept);
                    }
                    return self.say(t!("Could not export the video: {e}"));
                }
            }
            return;
        }
        // 描き終わった。渡し口を閉じて、書き終わるのを待つ（待つあいだも、画面は止めない）。
        if job.pending.is_some() {
            return;
        }
        job.send = None;
        if job.writer.as_ref().is_some_and(|w| !w.is_finished()) {
            self.say(t!("Finishing the video…"));
            return;
        }
        let job = self.movie.take().expect("movie job present");
        if let Some(kept) = job.kept {
            self.restore_marks(kept);
        }
        let result = job.writer.map_or(Ok(()), |w| w.join().unwrap_or_else(|_| Err(t!("Exporting stopped partway").to_owned())));
        match result {
            Ok(()) => self.say(t!("Saved {} ({} frames)", job.path.display(), job.total)),
            Err(e) => self.say(t!("Could not save the video: {e}")),
        }
    }

    /// 実寸の型紙の PDF を書き出す。
    pub(super) fn export_print(&mut self) {
        let Some(path) = self.ask_output(t!("Save the full-size dieline"), "PDF", "pdf", t!("full-size dieline")) else { return };
        match self.print_file(&path) {
            Ok(pages) => self.say(t!("Saved {} ({} {pages} sheet(s)). Print at \"Actual size\" (100%).", path.display(), self.output.print.paper.name())),
            Err(e) => self.say(t!("Could not save the full-size dieline: {e}")),
        }
    }

    /// 実寸の型紙の PDF を書き、紙の枚数を返す。
    pub(super) fn print_file(&self, path: &std::path::Path) -> Result<usize, String> {
        let (w, h) = (self.sheet.w, self.sheet.h);
        // 柄と紙の色だけの絵（ガイドの線は、PDF に線で描く）。
        let art = compose(&self.sheet, self.custom[0].as_deref(), false, |rgba| paint(&self.surface, &self.template, &self.decals, Marks::default(), false, Canvas::output(w, h, rgba)));
        let (cuts, folds) = sheet_lines(&self.template);
        let title = self.project_name();
        let sheet = SheetPrint { min: self.template.min, max: self.template.max, w, h, rgba: &art, cuts: &cuts, folds: &folds, title: &title };
        print_pdf(&sheet, &self.output.print).and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))?;
        let size = [self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]];
        Ok(self.used_tiles(size, &tile(size, &self.output.print)).len())
    }

    /// 入稿用の PDF を保存する（ダイアログで場所を選ぶ）。
    pub(super) fn export_ready(&mut self) {
        let Some(path) = self.ask_output(t!("Save the print-ready PDF"), "PDF", "pdf", t!("print-ready")) else { return };
        let color = match self.output.color {
            ReadyColorChoice::Rgb => "rgb",
            ReadyColorChoice::Fogra39 => "cmyk_builtin",
            ReadyColorChoice::Icc(_) => "cmyk_icc",
        };
        self.track("print_ready_color", serde_json::json!({ "color": color }));
        self.start_export(super::export::ExportKind::Ready, path);
    }

    /// 入稿用の PDF を書き、知らせる文を返す（書き出しのスレッドから呼ぶ）。
    pub(super) fn ready_message(&self, path: &Path) -> Result<String, String> {
        match self.ready_file(path) {
            Ok(pages) => {
                let note = self.coarse_note();
                Ok(format!("{}{note}", t!("Saved {} ({pages} page(s), with the finishing instructions on the last page).", path.display())))
            }
            Err(e) => Err(t!("Could not save the print-ready PDF: {e}")),
        }
    }

    /// 入稿用に刷る絵（紙の色は入れない。読み込んだ絵柄があれば、それごと）。展開図の表の向き。
    pub(super) fn ready_art(&self, out: &super::export::OutSheet, back: bool) -> Vec<u8> {
        let frame = out.sheet.frame;
        let zone = out.zone.as_ref();
        // 紙の色は入れず、読み込んだ絵柄（あれば）の上に、柄を描く。
        let mut art = out.custom()[usize::from(back)].map_or_else(|| vec![0u8; frame.w * frame.h * 4], <[u8]>::to_vec);
        paint(&self.surface, &self.template, &self.decals, Marks::default(), back, Canvas::bled(frame, zone, &mut art));
        if let Some(zone) = zone {
            crate::art::extend_edges(&mut art, frame, zone, None);
        }
        art
    }

    /// 入稿用の PDF を書き、ページの数を返す。表と、裏に柄があれば裏（裏から見た向き、左右を裏返す）。最後に指示のページ。
    pub(super) fn ready_file(&self, path: &Path) -> Result<usize, String> {
        let out = self.out_sheet();
        let frame = out.sheet.frame;
        let (w, h) = (frame.w, frame.h);
        let (min, max) = (self.template.min, self.template.max);
        let (cuts, folds) = sheet_lines(&self.template);
        let flip = |lines: &[(pakenoba_core::template::P2, pakenoba_core::template::P2)]| lines.iter().map(|&(a, b)| ([min[0] + max[0] - a[0], a[1]], [min[0] + max[0] - b[0], b[1]])).collect::<Vec<_>>();
        let front = (self.ready_art(&out, false), self.finish_plates(false, frame));
        let back = self.has_back_decals().then(|| {
            let plates = self.finish_plates(true, frame).into_iter().map(|p| crate::art::Plate { ink: mirror_gray(&p.ink, w), ..p }).collect::<Vec<_>>();
            (mirror(&self.ready_art(&out, true), w, h), plates, flip(&cuts), flip(&folds))
        });
        let mut sides = vec![ReadySide { label: "Front", rgba: &front.0, plates: &front.1, cuts: &cuts, folds: &folds }];
        if let Some((art, plates, cuts, folds)) = &back {
            sides.push(ReadySide { label: "Back (seen from the back)", rgba: art, plates, cuts, folds });
        }
        let pages = sides.len() + 1;
        let title = self.project_name();
        let profile = self.output.color.profile()?;
        let color = match &profile {
            Some(profile) => ReadyColor::Cmyk { profile },
            None => ReadyColor::Rgb,
        };
        print_ready_pdf(min, max, frame, &sides, &title, &self.finish_notes_ascii(), &color).and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))?;
        Ok(pages)
    }

    /// 生成AI用の画像の設定と、書き出し、生成した絵を面へ貼る操作。
    fn view_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(t!("Saves the box as you see it now as images to hand to generative AI: a shaded reference, line art, depth, normals, a mask and one colour per face, plus view.json (the camera). Turn the box in the middle to choose the angle."));
        ui.horizontal(|ui| {
            ui.label(t!("Size"));
            let label = |i: usize| format!("{} x {}", super::viewgen::VIEW_SIZES[i][0], super::viewgen::VIEW_SIZES[i][1]);
            egui::ComboBox::from_id_salt("view-size").selected_text(label(self.output.view_size)).show_ui(ui, |ui| {
                for i in 0..super::viewgen::VIEW_SIZES.len() {
                    ui.selectable_value(&mut self.output.view_size, i, label(i));
                }
            });
        });
        if ui.button(t!("Save the images for generative AI…")).clicked() {
            self.export_view();
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(t!("Then generate with ComfyUI (ControlNet), Firefly (composition reference) or another tool, at the same width-to-height ratio, and put the result back on the faces you can see. The camera is read from view.json in the same folder as the image (or the images saved last)."));
        ui.checkbox(&mut self.output.even_faces, t!("Even out the brightness of the faces"));
        if ui.button(t!("Put a generated image on the faces…")).clicked() {
            self.import_view_image();
        }
        ui.small(t!("It goes onto the front of the sheet as dieline artwork (Ctrl+Z to undo). Faces that cannot be seen in the image stay as they are."));
    }

    /// 生成AI用の画像を、選んだフォルダの中に、新しいフォルダを作って書く。
    pub(super) fn export_view(&mut self) {
        let project = self.file.path.as_deref().and_then(Path::file_stem).map_or_else(|| t!("PAKENOBA").to_owned(), |s| s.to_string_lossy().into_owned());
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let dir = match self.answer_export.take() {
            Some(answer) => answer,
            None => {
                let mut dialog = rfd::FileDialog::new().set_title(t!("Choose a folder for the images for generative AI"));
                if let Some(dir) = self.file.path.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(dir);
                }
                let Some(folder) = dialog.pick_folder() else { return };
                folder.join(format!("{project}_view_{stamp}"))
            }
        };
        let spec = self.view_spec(super::viewgen::VIEW_SIZES[self.output.view_size]);
        match self.write_view(&dir, &spec) {
            Ok(files) => {
                self.last_view = Some((dir.clone(), spec));
                self.say(t!("Saved {} files in {}.", files.len(), dir.display()));
            }
            Err(e) => self.say(t!("Could not save the images for generative AI: {e}")),
        }
    }

    /// 生成した絵を選んでもらい、面へ貼る。カメラは、絵の隣の view.json か、最後に書き出したもの。
    pub(super) fn import_view_image(&mut self) {
        let image = match self.answer_replace.take() {
            Some(answer) => answer,
            None => {
                let mut dialog = rfd::FileDialog::new().set_title(t!("Choose the generated image")).add_filter(t!("Images"), &["png", "jpg", "jpeg", "psd"]);
                if let Some((dir, _)) = &self.last_view {
                    dialog = dialog.set_directory(dir);
                }
                let Some(path) = dialog.pick_file() else { return };
                path
            }
        };
        let spec = match super::viewgen::find_view_json(&image) {
            Some(json) => match std::fs::read_to_string(&json).map_err(|e| e.to_string()).and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())).and_then(|v| super::viewgen::ViewSpec::from_json(&v)) {
                Ok(spec) => spec,
                Err(e) => return self.say(t!("Could not read {}: {e}", json.display())),
            },
            None => match &self.last_view {
                Some((_, spec)) => spec.clone(),
                None => return self.say(t!("No view.json was found beside the image. Save the images for generative AI first, and keep view.json in the same folder as the generated image.")),
            },
        };
        let even = self.output.even_faces;
        match self.apply_view_image(&image, &spec, even) {
            Ok(faces) => self.say(t!("Put the image on {faces} face(s). Press Ctrl+Z to undo.")),
            Err(e) => self.say(e),
        }
    }

    /// 入稿用の PDF の絵の色。RGB のまま、内蔵のコート紙の CMYK、または ICC のファイル。
    fn ready_color_choice(&mut self, ui: &mut egui::Ui) {
        let mut pick_file = false;
        ui.horizontal(|ui| {
            ui.label(t!("Color mode"));
            egui::ComboBox::from_id_salt("ready-color").width(260.0).selected_text(self.output.color.label()).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.output.color, ReadyColorChoice::Rgb, ReadyColorChoice::Rgb.label());
                ui.selectable_value(&mut self.output.color, ReadyColorChoice::Fogra39, ReadyColorChoice::Fogra39.label());
                if let Some(path) = self.settings.cmyk_icc.clone() {
                    let choice = ReadyColorChoice::Icc(path);
                    let label = choice.label();
                    ui.selectable_value(&mut self.output.color, choice, label);
                }
                if ui.button(t!("Choose an ICC profile…")).clicked() {
                    pick_file = true;
                    ui.close();
                }
            });
        });
        if pick_file && let Some(path) = rfd::FileDialog::new().set_title(t!("Choose a CMYK ICC profile")).add_filter("ICC", &["icc", "icm"]).pick_file() {
            let choice = ReadyColorChoice::Icc(path.clone());
            match choice.profile() {
                Ok(_) => {
                    self.output.color = choice;
                    self.settings.cmyk_icc = Some(path);
                    if let Err(e) = self.settings.save() {
                        self.say(e);
                    }
                }
                Err(e) => self.say(e),
            }
        }
        if self.output.color != ReadyColorChoice::Rgb {
            ui.small(t!("The artwork is converted from sRGB with relative colorimetric intent and black point compensation, and the profile is embedded as the output intent. Ask your print shop which profile to use (in Japan, often Japan Color 2001 Coated). Black text becomes four-color black."));
        }
    }

    /// 書き出しの細かさを選ぶ（型紙の PSD と入稿用の PDF で共通）。画素の数と、細かさの足りない画像も出す。
    fn resolution_choice(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t!("Bleed"));
            ui.add(egui::DragValue::new(&mut self.output.bleed).range(0.0..=5.0).speed(0.1).max_decimals(1).suffix(" mm"));
        });
        ui.small(t!("Artwork that crosses a cut line continues past it in the plane of its panel. Where nothing reaches, the edge colour is stretched. Finish plates get no bleed."));
        let label = |dpi: u32| if dpi <= 100 { t!("Same as the screen (about 100 dpi)").to_owned() } else { format!("{dpi} dpi") };
        ui.horizontal(|ui| {
            ui.label(t!("Resolution"));
            egui::ComboBox::from_id_salt("export-dpi").selected_text(label(self.output.dpi)).show_ui(ui, |ui| {
                for dpi in super::export::EXPORT_DPI {
                    ui.selectable_value(&mut self.output.dpi, dpi, label(dpi));
                }
            });
        });
        let frame = self.export_frame();
        ui.small(t!("{} x {} pixels. Text is drawn again at this resolution.", frame.w, frame.h));
        let coarse = self.coarse_images();
        if !coarse.is_empty() {
            let list: Vec<String> = coarse.iter().map(|(name, dpi)| format!("{name} ({dpi} dpi)")).collect();
            ui.colored_label(ui.visuals().warn_fg_color, t!("These images are coarse at actual size: {}", list.join(", ")));
        }
    }

    /// 右（出力）: 選んだ出力の設定。
    pub(super) fn output_settings(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("output-settings").resizable(false).exact_size(320.0).frame(super::theme::side_frame()).show(ui, |ui| {
            super::panels::boxed(ui, pakenoba_i18n::tr(OUTPUTS[self.output_kind].0), true, |ui| {
            let ctx = ui.ctx().clone();
            match self.output_kind {
                0 => {
                    ui.label(t!("Saves the cut lines, fold lines, glue flaps and placed artwork as a PSD with separate layers."));
                    self.resolution_choice(ui);
                    if self.export_busy(ui) {
                    } else {
                        if ui.button(t!("Save the dieline as PSD…")).clicked() {
                            self.export_psd();
                        }
                        if ui.button(t!("Save the assembled view as PSD…")).clicked() {
                            self.export_atlas();
                        }
                    }
                    ui.small(t!("Each PSD has a selection channel for each face: Ctrl+click a channel in Photoshop to select that face (for Generative Fill, for example). The assembled view lays the outside faces out as they face on the box (front in the middle, top above), so nothing comes out upside down; drop it back on the window to put each face back on the dieline."));
                    ui.small(t!("After painting on it in Photoshop or another app, drop the PSD on the window (or choose File > Import dieline artwork (PSD)…). It comes back as the dieline artwork in the Layers panel."));
                    if self.has_back_decals() {
                        ui.small(t!("There is artwork on the back of the sheet too, so the back dieline is saved in the same place with \"_back\" added to the name. The back dieline is seen from the back (flipped left to right)."));
                    }
                }
                1 => {
                    ui.label(t!("Saves the box as you see it now as a 16:9 image (PNG). Set the angle and size by dragging and scrolling on the box in the middle."));
                    egui::Grid::new("shot").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label(t!("Size"));
                        egui::ComboBox::from_id_salt("shot-size").selected_text(pakenoba_i18n::tr(SHOT_SIZES[self.output.shot_size].0)).show_ui(ui, |ui| {
                            for (i, (label, _)) in SHOT_SIZES.iter().enumerate() {
                                ui.selectable_value(&mut self.output.shot_size, i, pakenoba_i18n::tr(label));
                            }
                        });
                        ui.end_row();
                        ui.label(t!("Background"));
                        egui::ComboBox::from_id_salt("shot-bg").selected_text(pakenoba_i18n::tr(BACKGROUNDS[self.output.background].0)).show_ui(ui, |ui| {
                            for (i, (label, _)) in BACKGROUNDS.iter().enumerate() {
                                ui.selectable_value(&mut self.output.background, i, pakenoba_i18n::tr(label));
                            }
                        });
                        ui.end_row();
                    });
                    if ui.button(t!("Save image…")).clicked() {
                        self.export_image(&ctx);
                    }
                    ui.small(t!("The frame in the middle shows what goes into the image."));
                }
                2 => {
                    ui.label(t!("Saves the closed box opening as a GIF animation you can put on a slide. It starts from the current angle. The size is set so the fully open box fits."));
                    egui::Grid::new("movie").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label(t!("Size"));
                        egui::ComboBox::from_id_salt("movie-size").selected_text(pakenoba_i18n::tr(MOVIE_SIZES[self.output.movie_size].0)).show_ui(ui, |ui| {
                            for (i, (label, _)) in MOVIE_SIZES.iter().enumerate() {
                                ui.selectable_value(&mut self.output.movie_size, i, pakenoba_i18n::tr(label));
                            }
                        });
                        ui.end_row();
                        ui.label(t!("Length"));
                        ui.add(egui::DragValue::new(&mut self.output.movie_seconds).range(1.5..=10.0).speed(0.1).max_decimals(1).suffix(t!(" s")));
                        ui.end_row();
                        ui.label(t!("Background"));
                        egui::ComboBox::from_id_salt("movie-bg").selected_text(pakenoba_i18n::tr(BACKGROUNDS[self.output.background.min(1)].0)).show_ui(ui, |ui| {
                            for (i, (label, _)) in BACKGROUNDS.iter().take(2).enumerate() {
                                ui.selectable_value(&mut self.output.background, i, pakenoba_i18n::tr(label));
                            }
                        });
                        ui.end_row();
                    });
                    ui.checkbox(&mut self.output.movie_turn, t!("Turn slightly while opening"));
                    // 動きのプリセット（どの案件でも使える）。
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t!("Motion preset"));
                        let mut chosen = None;
                        egui::ComboBox::from_id_salt("motion-preset").selected_text(t!("Choose…")).show_ui(ui, |ui| {
                            if self.settings.motions.is_empty() {
                                ui.weak(t!("No presets yet"));
                            }
                            for (i, m) in self.settings.motions.iter().enumerate() {
                                if ui.selectable_label(false, &m.name).clicked() {
                                    chosen = Some(i);
                                }
                            }
                        });
                        if let Some(i) = chosen {
                            self.use_motion(i);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.motion_name).hint_text(t!("Preset name")).desired_width(120.0));
                        if ui.add_enabled(!self.motion_name.trim().is_empty(), egui::Button::new(t!("Save motion"))).on_hover_text(t!("Saves the length, turning, size, background and the current viewing angle")).clicked() {
                            let name = std::mem::take(&mut self.motion_name);
                            self.save_motion(&name);
                        }
                        let named = self.settings.motions.iter().position(|m| m.name == self.motion_name.trim());
                        if ui.add_enabled(named.is_some(), egui::Button::new(t!("Delete"))).on_hover_text(t!("Deletes the preset with this name")).clicked()
                            && let Some(i) = named
                        {
                            self.settings.motions.remove(i);
                            if let Err(e) = self.settings.save() {
                                self.say(e);
                            }
                        }
                    });
                    ui.add_space(4.0);
                    let busy = self.movie.is_some();
                    ui.horizontal(|ui| {
                        let playing = self.movie_preview.is_some();
                        if ui.add_enabled(!busy && self.template.can_open_at(0), egui::Button::new(if playing { t!("Stop preview") } else { t!("Preview") })).on_hover_text(t!("Plays the video on screen, the same as it will be saved")).clicked() {
                            self.toggle_movie_preview();
                        }
                        if ui.add_enabled(!busy, egui::Button::new(t!("Save video…"))).clicked() {
                            self.stop_movie_preview();
                            self.export_movie();
                        }
                    });
                    if !self.template.can_open_at(0) {
                        ui.small(t!("This box type has no opening motion."));
                    }
                    ui.small(t!("Saving as MP4 is not available yet."));
                }
                4 => {
                    ui.label(t!("Makes a full-size PDF of the dieline split across sheets of paper. Join them along the overlaps, stick them on card stock and assemble."));
                    egui::Grid::new("print").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label(t!("Paper"));
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.output.print.paper, Paper::A4, "A4");
                            ui.selectable_value(&mut self.output.print.paper, Paper::A3, "A3");
                        });
                        ui.end_row();
                        ui.label(t!("Overlap"));
                        ui.add(egui::DragValue::new(&mut self.output.print.overlap).range(0.0..=30.0).speed(0.5).max_decimals(0).suffix(" mm"));
                        ui.end_row();
                        ui.label(t!("Paper margin"));
                        ui.add(egui::DragValue::new(&mut self.output.print.margin).range(3.0..=25.0).speed(0.5).max_decimals(0).suffix(" mm"));
                        ui.end_row();
                    });
                    let size = [self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]];
                    let tiling = tile(size, &self.output.print);
                    let used = self.used_tiles(size, &tiling);
                    ui.label(t!("{} {} sheets of {} ({} across x {} down; sheets the dieline does not touch are skipped)", self.output.print.paper.name(), if tiling.landscape() { t!("landscape") } else { t!("portrait") }, used.len(), tiling.columns, tiling.rows));
                    self.print_preview(ui, size, &tiling, &used);
                    if ui.button(t!("Save the full-size dieline as PDF…")).clicked() {
                        self.export_print();
                    }
                    ui.small(t!("When printing, do not scale; print at \"Actual size\" (100%). Measure the 100 mm scale on the first sheet with a ruler to check."));
                }
                super::peek::PEEK_OUTPUT => self.peek_settings(ui),
                VIEW_OUTPUT => self.view_settings(ui),
                MODEL_OUTPUT => {
                    ui.label(t!("Saves the box as a 3D model, folded and opened as it is now on screen, with the artwork on the outside and inside."));
                    ui.horizontal(|ui| {
                        if ui.button(t!("Save as GLB…")).on_hover_text(t!("One file with the shape and artwork. Opens in Blender, browser viewers and many 3D apps")).clicked() {
                            self.export_glb();
                        }
                        if ui.button(t!("Save as OBJ…")).on_hover_text(t!("For older 3D apps. Saves the .obj, a .mtl and two PNG images; keep them together")).clicked() {
                            self.export_obj();
                        }
                    });
                    ui.small(t!("Lengths are in meters. Foil, embossing and varnish are baked into the artwork as colors; their shine is not included."));
                    ui.small(t!("To look at it on Windows, use Blender, or open it in a browser viewer such as Babylon.js Sandbox."));
                }
                READY_OUTPUT => {
                    ui.label(t!("Makes a PDF to hand to a print shop: one full-size page per side, with the cut and fold lines and each finish as its own spot color plate."));
                    let plates = self.plate_names();
                    if plates.is_empty() {
                        ui.small(t!("There are no finishes (foil, embossing, spot UV, white ink) yet. Add them from the star button in the Layers panel."));
                    } else {
                        ui.label(t!("Plates: {}", plates.join(", ")));
                    }
                    self.resolution_choice(ui);
                    self.ready_color_choice(ui);
                    if self.export_busy(ui) {
                    } else if ui.button(t!("Save the print-ready PDF…")).clicked() {
                        self.export_ready();
                    }
                    ui.small(t!("Cut lines are the spot color \"Die Cut\" and fold lines \"Crease\". Every plate is set to overprint. The last page lists the finishes, heights and edges."));
                }
                _ => {}
            }
            ui.add_space(6.0);
            ui.small(t!("This output is for checking the design. It is not production data and does not guarantee die-cutting, trimming or print positions."));
            });
        });
    }

    /// 紙の分け方の見本。展開図の絵の上に、紙の区切り（刷れる範囲）と番号を描く。
    /// 刷る紙（展開図の紙がかかるもの）。
    fn used_tiles(&self, size: [f32; 2], tiling: &crate::print::Tiling) -> Vec<(usize, usize)> {
        let (w, art) = (self.sheet.w, &self.sheet.art);
        crate::print::used_tiles(tiling, size, self.output.print.margin, w, self.sheet.h, |x, y| art[(y * w + x) * 4 + 3])
    }

    fn print_preview(&mut self, ui: &mut egui::Ui, size: [f32; 2], tiling: &crate::print::Tiling, used: &[(usize, usize)]) {
        // 展開図の絵（出力の工程では、展開図の表示を出さないので、ここで用意する）。
        if self.front_tex.is_none() || self.front_tex_dirty {
            self.front_tex = Some(super::views::texture(ui.ctx(), "sheet-front", self.sheet.w, self.sheet.h, &self.front_rgba));
            self.front_tex_dirty = false;
        }
        let margin = self.output.print.margin;
        let usable = [tiling.page[0] - 2.0 * margin, tiling.page[1] - 2.0 * margin];
        let extent = [((tiling.columns - 1) as f32 * tiling.step[0] + usable[0]).max(size[0]), ((tiling.rows - 1) as f32 * tiling.step[1] + usable[1]).max(size[1])];
        let width = ui.available_width();
        let scale = (width / extent[0]).min(240.0 / extent[1]);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, extent[1] * scale + 4.0), egui::Sense::hover());
        let origin = rect.left_top() + egui::vec2((width - extent[0] * scale) * 0.5, 2.0);
        let painter = ui.painter_at(rect);
        // 展開図は、1 枚目の左上から置く（PDF と同じ）。
        let sheet = egui::Rect::from_min_size(origin, egui::vec2(size[0] * scale, size[1] * scale));
        if let Some(texture) = &self.front_tex {
            painter.image(texture.id(), sheet, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        for (number, &(row, column)) in used.iter().enumerate() {
            let min = origin + egui::vec2(column as f32 * tiling.step[0], row as f32 * tiling.step[1]) * scale;
            let page = egui::Rect::from_min_size(min, egui::vec2(usable[0], usable[1]) * scale);
            painter.rect_stroke(page, 0.0, egui::Stroke::new(1.0, super::TEAL), egui::StrokeKind::Inside);
            painter.text(page.left_top() + egui::vec2(3.0, 2.0), egui::Align2::LEFT_TOP, format!("{}", number + 1), egui::FontId::proportional(11.0), super::TEAL);
        }
    }

    /// 出力の工程で、16:9 の画像を選んでいるとき、立体の表示の上に、画像に入る範囲の枠を描く。
    pub(super) fn shot_frame(&self, ui: &egui::Ui, rect: egui::Rect) {
        let target = match self.output_kind {
            1 => 16.0 / 9.0,
            VIEW_OUTPUT => {
                let [w, h] = super::viewgen::VIEW_SIZES[self.output.view_size];
                w as f32 / h as f32
            }
            _ => return,
        };
        let aspect = rect.width() / rect.height();
        let frame = if aspect > target { egui::Rect::from_center_size(rect.center(), egui::vec2(rect.height() * target, rect.height())) } else { egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), rect.width() / target)) };
        let painter = ui.painter().with_clip_rect(rect);
        let shade = egui::Color32::from_black_alpha(110);
        for part in [egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, frame.min.y)), egui::Rect::from_min_max(egui::pos2(rect.min.x, frame.max.y), rect.max), egui::Rect::from_min_max(egui::pos2(rect.min.x, frame.min.y), egui::pos2(frame.min.x, frame.max.y)), egui::Rect::from_min_max(egui::pos2(frame.max.x, frame.min.y), egui::pos2(rect.max.x, frame.max.y))] {
            painter.rect_filled(part, 0.0, shade);
        }
        painter.rect_stroke(frame, 0.0, egui::Stroke::new(1.0, egui::Color32::from_white_alpha(140)), egui::StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pakenoba_core::template::P2;

    #[test]
    fn 塗り足しは_切り線の外を_柄か端の色で埋める() {
        let mut app = App::new(&super::super::Initial::default());
        let (_, text) = super::super::samples::SAMPLES[3];
        let opened = pakenoba_core::project::Project::from_text(text).unwrap();
        assert!(app.apply_project(opened.project).is_empty());
        app.output.dpi = 100;
        app.output.bleed = 3.0;
        let out = app.out_sheet();
        let zone = out.zone.as_ref().unwrap();
        assert!(!zone.map.is_empty());
        let art = app.ready_art(&out, false);
        let frame = out.sheet.frame;
        // 余白のぶん、画像が大きい。
        assert_eq!(frame.w, crate::art::Frame::of(&app.template, crate::art::PX_PER_MM, 3.0).w);
        // 塗り足しの画素は、どれも紙の外。黒の見本は全面に柄があるので、ほとんどが塗られる。
        assert!(zone.map.keys().all(|&p| out.sheet.art[p as usize * 4 + 3] == 0));
        let alpha_at = |p: P2| {
            let [x, y] = frame.to_px(p);
            art[(y as usize * frame.w + x as usize) * 4 + 3]
        };
        let (min, max) = (app.template.min, app.template.max);
        // 右端の側面（黒）の切り線の外 1.5 mm は、黒が続く。
        assert_eq!(alpha_at([max[0] + 1.5, (min[1] + max[1]) * 0.5 + 20.0]), 255);
        // 柄のない底（白）の横の塗り足しには、隣の側面の柄を回り込ませない。
        let top_of = |p: &&pakenoba_core::template::Panel| p.outline.iter().map(|q| q[1]).fold(f32::MIN, f32::max);
        let bottom = app.template.panels.iter().filter(|p| p.role == pakenoba_core::template::Role::Face).min_by(|a, b| top_of(a).total_cmp(&top_of(b))).unwrap();
        let (lo, hi) = bottom.outline.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), q| ([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])]));
        assert_eq!(alpha_at([lo[0] - 1.5, (lo[1] + hi[1]) * 0.5]), 0);
        assert_eq!(alpha_at([hi[0] + 1.5, (lo[1] + hi[1]) * 0.5]), 0);
        // 塗り足しの外（紙から 3 mm より遠い所）は塗らない。
        let far = (0..frame.w * frame.h).filter(|&p| out.sheet.art[p * 4 + 3] == 0 && !zone.map.contains_key(&(p as u32)) && art[p * 4 + 3] > 0).count();
        assert!(far < zone.map.len() / 50, "{far}");
    }

    #[test]
    fn 隣り合う面の図形のあいだに_明るい線が出ない() {
        let mut app = App::new(&super::super::Initial::default());
        let (_, text) = super::super::samples::SAMPLES[3];
        let opened = pakenoba_core::project::Project::from_text(text).unwrap();
        assert!(app.apply_project(opened.project).is_empty());
        for dpi in [100, 350] {
            app.output.dpi = dpi;
            app.output.bleed = 0.0;
            let out = app.out_sheet();
            let art = app.ready_art(&out, false);
            let frame = out.sheet.frame;
            // 黒の見本の側面の帯（4 面が横に並ぶ）。中ほどの行は、紙のある所がすべて黒で、透けた画素がない。
            let (_, folds) = sheet_lines(&app.template);
            let mut checked = 0;
            let alpha = |p: P2| {
                let [px, py] = frame.to_px(p);
                art[(py as usize * frame.w + px as usize) * 4 + 3]
            };
            for (a, b) in folds.iter().filter(|(a, b)| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() > 30.0) {
                let y = (a[1] + b[1]) * 0.5;
                // 両側とも黒い面の折り線だけを見る（片側が白い底のものは除く）。
                if alpha([a[0] - 0.6, y]) < 255 || alpha([a[0] + 0.6, y]) < 255 {
                    continue;
                }
                for dx in [-0.3f32, -0.15, -0.05, 0.0, 0.05, 0.15, 0.3] {
                    assert_eq!(alpha([a[0] + dx, y]), 255, "{dpi} dpi, 折り線 x = {} の近く ({dx} mm) が透けている", a[0]);
                    checked += 1;
                }
            }
            assert!(checked >= 14, "{checked}");
        }
    }

    #[test]
    fn 入稿用の_pdf_には_見本の加工がすべて特色で入る() {
        let mut app = App::new(&super::super::Initial::default());
        let (_, text) = super::super::samples::SAMPLES[3];
        let opened = pakenoba_core::project::Project::from_text(text).unwrap();
        assert!(app.apply_project(opened.project).is_empty());
        let dir = std::env::temp_dir().join(format!("pakenoba-ready-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ready.pdf");
        app.output.dpi = 100;
        let pages = app.ready_file(&path).unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        for name in app.plate_names() {
            let name = name.replace(' ', "#20");
            assert!(text.contains(&format!("/Separation /{name}")), "{name}");
        }
        assert!(pages >= 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 動画は_閉じたまま始まり_開けたまま終わる() {
        let total = 60;
        assert_eq!(movie_pose(0, total, false), (0.0, 0.0));
        assert_eq!(movie_pose(5, total, false).0, 0.0, "はじめの 1 割は閉じたまま");
        assert_eq!(movie_pose(total - 1, total, false).0, 1.0);
        assert_eq!(movie_pose(45, total, false).0, 1.0, "後ろは開けたまま");
        let opens: Vec<f32> = (0..total).map(|f| movie_pose(f, total, true).0).collect();
        assert!(opens.windows(2).all(|w| w[1] >= w[0]), "閉じる向きに戻らない");
        let (_, first) = movie_pose(0, total, true);
        let (_, last) = movie_pose(total - 1, total, true);
        assert!((last - first - 30f32.to_radians()).abs() < 1e-4, "30 度回り込む");
    }

    #[test]
    fn 名前に拡張子を足す() {
        assert_eq!(with_extension(PathBuf::from("a/箱"), "png"), PathBuf::from("a/箱.png"));
        assert_eq!(with_extension(PathBuf::from("a/箱.PNG"), "png"), PathBuf::from("a/箱.PNG"));
    }
}
