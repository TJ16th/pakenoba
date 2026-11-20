//! ウィンドウ（egui / eframe）。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;

use crate::psd_io::{load_psd, make_sample_psd};
use crate::raster::{Texture, render};
use crate::scene::{BoxDims, Camera};
use crate::video::{VideoSpec, VideoStats, export_rotation};

const SAMPLE_PSD_NAME: &str = "pakenova-s01-sample.psd";
const VIDEO_NAME: &str = "pakenova-s01-rotation.mp4";

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("新パケノバ 試作 S01", options, Box::new(|_cc| Ok(Box::new(App::new()))))
}

enum ExportMsg {
    Progress(f32),
    Finished(Result<VideoStats, String>),
}

enum ExportState {
    Idle,
    Running { rx: Receiver<ExportMsg>, progress: f32 },
    Done(String),
    Failed(String),
}

#[derive(PartialEq, Clone, Copy)]
struct RenderKey {
    dims: BoxDims,
    cam: Camera,
    size: [usize; 2],
    tex_version: u64,
}

struct App {
    dims: BoxDims,
    cam: Camera,
    texture: Option<Arc<Texture>>,
    tex_version: u64,
    psd_info: String,
    psd_path: String,
    line_text: String,
    area_text: String,
    view: Option<egui::TextureHandle>,
    view_key: Option<RenderKey>,
    render_ms: f64,
    export: ExportState,
    sample_note: String,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            dims: BoxDims::default(),
            cam: Camera::default(),
            texture: None,
            tex_version: 0,
            psd_info: String::new(),
            psd_path: String::new(),
            line_text: String::new(),
            area_text: String::new(),
            view: None,
            view_key: None,
            render_ms: 0.0,
            export: ExportState::Idle,
            sample_note: String::new(),
        };
        match make_sample_psd(app.dims) {
            Ok(bytes) => app.set_psd(&bytes, "内蔵のサンプル"),
            Err(e) => app.psd_info = e,
        }
        app
    }

    fn set_psd(&mut self, bytes: &[u8], source: &str) {
        match load_psd(bytes) {
            Ok(loaded) => {
                let merged = match loaded.has_real_merged {
                    Some(false) => "\n注意: この PSD には統合画像が入っていません（白い仮画像を表示しています）。",
                    _ => "",
                };
                self.psd_info = format!(
                    "{source}\n大きさ: {} x {} px\nレイヤー: {}{merged}",
                    loaded.texture.w,
                    loaded.texture.h,
                    if loaded.layer_names.is_empty() { "なし".to_owned() } else { loaded.layer_names.join("、") }
                );
                self.texture = Some(Arc::new(loaded.texture));
                self.tex_version += 1;
            }
            Err(e) => self.psd_info = format!("{source}\n{e}"),
        }
    }

    fn open_psd_path(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => self.set_psd(&bytes, &path.display().to_string()),
            Err(e) => self.psd_info = format!("{} を読めませんでした: {e}", path.display()),
        }
    }

    fn start_export(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (dims, texture, ctx) = (self.dims, self.texture.clone(), ctx.clone());
        let path = PathBuf::from(VIDEO_NAME);
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = export_rotation(dims, texture.as_deref(), &VideoSpec::default(), &path, move |p| {
                let _ = progress_tx.send(ExportMsg::Progress(p));
                progress_ctx.request_repaint();
            });
            let _ = tx.send(ExportMsg::Finished(result));
            ctx.request_repaint();
        });
        self.export = ExportState::Running { rx, progress: 0.0 };
    }

    fn poll_export(&mut self) {
        let mut next = None;
        if let ExportState::Running { rx, progress } = &mut self.export {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    ExportMsg::Progress(p) => *progress = p,
                    ExportMsg::Finished(Ok(stats)) => {
                        let full = std::env::current_dir().map(|d| d.join(VIDEO_NAME)).unwrap_or_else(|_| PathBuf::from(VIDEO_NAME));
                        next = Some(ExportState::Done(format!(
                            "{}\n{} フレーム、{:.1} MB\n描画 {:.1} 秒、圧縮 {:.1} 秒",
                            full.display(),
                            stats.frames,
                            stats.bytes as f64 / 1_048_576.0,
                            stats.render_ms / 1000.0,
                            stats.encode_ms / 1000.0
                        )));
                    }
                    ExportMsg::Finished(Err(e)) => next = Some(ExportState::Failed(e)),
                }
            }
        }
        if let Some(state) = next {
            self.export = state;
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("新パケノバ 試作 S01");
        ui.label("土台の確認用です。本番の画面ではありません。");
        ui.add_space(10.0);

        ui.strong("1. 日本語入力");
        ui.label("変換中の文字、候補ウィンドウの位置、確定、削除を確かめてください。");
        ui.add(egui::TextEdit::singleline(&mut self.line_text).hint_text("1行の入力欄").desired_width(f32::INFINITY));
        ui.add(egui::TextEdit::multiline(&mut self.area_text).hint_text("複数行の入力欄").desired_rows(3).desired_width(f32::INFINITY));
        let typed = if self.line_text.is_empty() { &self.area_text } else { &self.line_text };
        if !typed.is_empty() {
            ui.label(egui::RichText::new(typed).size(22.0));
            ui.weak(format!("{} 文字", typed.chars().count()));
        }
        ui.add_space(10.0);

        ui.strong("2. 箱の寸法（mm）");
        ui.add(egui::Slider::new(&mut self.dims.w, 30.0..=300.0).text("幅"));
        ui.add(egui::Slider::new(&mut self.dims.h, 30.0..=300.0).text("高さ"));
        ui.add(egui::Slider::new(&mut self.dims.d, 20.0..=200.0).text("奥行き"));
        ui.weak("箱の上をドラッグで回転、ホイールで拡大縮小。");
        ui.add_space(10.0);

        ui.strong("3. PSD");
        ui.label("PSD ファイルをウィンドウにドロップするか、パスを入力してください。");
        ui.horizontal(|ui| {
            let button_width = 70.0;
            ui.add(egui::TextEdit::singleline(&mut self.psd_path).hint_text("PSD のパス").desired_width(ui.available_width() - button_width));
            if ui.button("読み込む").clicked() {
                let path = PathBuf::from(self.psd_path.trim().trim_matches('"'));
                self.open_psd_path(&path);
            }
        });
        ui.label(&self.psd_info);
        if ui.button("サンプル PSD を保存").clicked() {
            self.sample_note = match make_sample_psd(self.dims).and_then(|b| std::fs::write(SAMPLE_PSD_NAME, b).map_err(|e| e.to_string())) {
                Ok(()) => {
                    let full = std::env::current_dir().map(|d| d.join(SAMPLE_PSD_NAME)).unwrap_or_else(|_| PathBuf::from(SAMPLE_PSD_NAME));
                    format!("保存しました: {}", full.display())
                }
                Err(e) => format!("保存できませんでした: {e}"),
            };
        }
        if !self.sample_note.is_empty() {
            ui.weak(&self.sample_note);
        }
        ui.add_space(10.0);

        ui.strong("4. 動画の書き出し");
        let running = matches!(self.export, ExportState::Running { .. });
        if ui.add_enabled(!running, egui::Button::new("回転動画を書き出す（4 秒）")).clicked() {
            self.start_export(ui.ctx());
        }
        match &self.export {
            ExportState::Idle => {}
            ExportState::Running { progress, .. } => {
                ui.add(egui::ProgressBar::new(*progress).show_percentage());
            }
            ExportState::Done(text) => {
                ui.label(format!("書き出しました\n{text}"));
            }
            ExportState::Failed(e) => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("失敗しました: {e}"));
            }
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        if response.dragged() {
            let delta = response.drag_delta();
            self.cam.yaw -= delta.x * 0.01;
            self.cam.pitch = (self.cam.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.cam.zoom = (self.cam.zoom * (scroll * 0.002).exp()).clamp(0.3, 4.0);
            }
        }

        let ppp = ui.ctx().pixels_per_point();
        let size = [((rect.width() * ppp) as usize).clamp(2, 1920), ((rect.height() * ppp) as usize).clamp(2, 1200)];
        let key = RenderKey { dims: self.dims, cam: self.cam, size, tex_version: self.tex_version };
        if self.view_key != Some(key) {
            let start = std::time::Instant::now();
            let frame = render(self.dims, self.cam, self.texture.as_deref(), size[0], size[1], 2);
            self.render_ms = start.elapsed().as_secs_f64() * 1000.0;
            let image = egui::ColorImage::from_rgba_unmultiplied([frame.w, frame.h], &frame.rgba);
            match &mut self.view {
                Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
                None => self.view = Some(ui.ctx().load_texture("box-view", image, egui::TextureOptions::LINEAR)),
            }
            self.view_key = Some(key);
        }
        if let Some(handle) = &self.view {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(handle.id(), rect, uv, egui::Color32::WHITE);
        }
        ui.painter().text(
            rect.left_bottom() + egui::vec2(10.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{} x {} px、描画 {:.1} ms（CPU の仮描画）", size[0], size[1], self.render_ms),
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(170),
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_export();

        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(path) = dropped.first() {
            self.psd_path = path.display().to_string();
            self.open_psd_path(path);
        }

        egui::Panel::left("controls").default_size(360.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
    }
}
