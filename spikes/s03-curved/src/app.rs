//! ウィンドウ（egui / eframe）。

use std::path::{Path, PathBuf};

use eframe::egui;
use eframe::egui_wgpu;

use crate::art::{GuideHandling, SheetArt, compose, load_psd, make_sheet_art, save_psd};
use crate::gpu::BoxRenderer;
use crate::math::{look_at, mat4_mul, normalize, perspective};
use crate::template::{Template, bounding_sphere};

const PSD_NAME: &str = "pakenova-s03-dieline.psd";

/// 起動時の状態。画面を撮って確かめるときに、コマンドラインから指定できる。
#[derive(Clone)]
pub struct Initial {
    pub fold: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub thickness: f32,
    pub guides: bool,
    /// 起動時に読み込む PSD。
    pub psd: Option<PathBuf>,
    /// 箱の型。0 は差し込み箱、1 は手提げ型、3 以上は正多角柱の角の数。
    pub sides: usize,
    /// 手提げ型のふくらみ（度）。
    pub bulge: f32,
}

impl Default for Initial {
    fn default() -> Self {
        Initial { fold: 1.0, yaw: 0.6, pitch: 0.35, thickness: 0.5, guides: true, psd: None, sides: 1, bulge: 40.0 }
    }
}

pub fn run(initial: Initial) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("新パケノバ 試作 S03", options, Box::new(move |_cc| Ok(Box::new(App::new(&initial)))))
}

#[derive(Clone, Copy, PartialEq)]
struct Dims {
    /// 0 は差し込み箱、1 は手提げ型、3 以上は正多角柱の角の数。
    sides: usize,
    /// 差し込み箱と手提げ型では幅、角柱では一辺の長さ。
    w: f32,
    d: f32,
    /// 差し込み箱と角柱では高さ、手提げ型では正面と背面の長さ。
    h: f32,
    t: f32,
    /// 手提げ型のふくらみ（度）。
    bulge: f32,
}

impl Dims {
    fn template(&self) -> Template {
        match self.sides {
            1 => Template::carry_bag(self.w, self.d, self.h, self.t, self.bulge.to_radians()),
            n if n >= 3 => Template::prism(n, self.w, self.h, self.t),
            _ => Template::tuck_box(self.w, self.d, self.h, self.t),
        }
    }
}

const SHAPES: [(usize, &str); 6] = [(1, "手提げ型"), (0, "差し込み箱"), (3, "三角柱"), (5, "五角柱"), (6, "六角柱"), (8, "八角柱")];

struct App {
    dims: Dims,
    built_dims: Option<Dims>,
    template: Template,
    sheet: SheetArt,
    /// 読み込んだ PSD の中身。寸法を変えたら展開図の大きさに合わせ直す。
    custom_bytes: Option<Vec<u8>>,
    custom: Option<Vec<u8>>,
    show_guides: bool,
    art_dirty: bool,
    fold: f32,
    /// アニメーションの行き先（0 か 1）。
    animate_to: Option<f32>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    renderer: Option<BoxRenderer>,
    psd_path: String,
    psd_info: String,
    note: String,
    triangles: usize,
    backend: String,
}

impl App {
    fn new(initial: &Initial) -> Self {
        let sides = if initial.sides == 2 { 0 } else { initial.sides };
        let w = match sides {
            1 => 150.0,
            n if n >= 3 => 70.0,
            _ => 120.0,
        };
        let dims = Dims { sides, w, d: 60.0, h: 160.0, t: initial.thickness, bulge: initial.bulge };
        let template = dims.template();
        let sheet = make_sheet_art(&template);
        let mut app = App {
            dims,
            built_dims: Some(dims),
            template,
            sheet,
            custom_bytes: None,
            custom: None,
            show_guides: initial.guides,
            art_dirty: true,
            fold: initial.fold.clamp(0.0, 1.0),
            animate_to: None,
            yaw: initial.yaw,
            pitch: initial.pitch,
            zoom: 1.0,
            renderer: None,
            psd_path: String::new(),
            psd_info: "確認用の絵柄".to_owned(),
            note: String::new(),
            triangles: 0,
            backend: String::new(),
        };
        if let Some(path) = &initial.psd {
            app.psd_path = path.display().to_string();
            app.open_psd(path);
        }
        app
    }

    /// 寸法が変わっていたら、型紙と絵柄を作り直す。
    fn rebuild_if_needed(&mut self) {
        if self.built_dims == Some(self.dims) {
            return;
        }
        self.template = self.dims.template();
        self.sheet = make_sheet_art(&self.template);
        self.built_dims = Some(self.dims);
        if let Some(bytes) = self.custom_bytes.clone() {
            self.apply_psd(&bytes, "読み込んだ PSD");
        }
        self.art_dirty = true;
    }

    fn apply_psd(&mut self, bytes: &[u8], source: &str) {
        match load_psd(bytes, self.sheet.w, self.sheet.h) {
            Ok(loaded) => {
                let merged = if loaded.has_real_merged == Some(false) { "\n注意: この PSD には統合画像が入っていません。" } else { "" };
                let ratio_src = loaded.source_size[0] as f32 / loaded.source_size[1] as f32;
                let ratio_sheet = self.sheet.w as f32 / self.sheet.h as f32;
                let stretch = if (ratio_src / ratio_sheet - 1.0).abs() > 0.01 { "\n注意: 縦横比が今の展開図と違うので、引き伸ばして貼っています。" } else { "" };
                let guides = match loaded.guides {
                    GuideHandling::NotPresent => "",
                    GuideHandling::Removed => "\nガイドのレイヤーは除いて読み込みました。",
                    GuideHandling::Baked => "\n注意: ガイドのレイヤーを除けませんでした（グループ、マスク、通常以外の合成モードがあります）。ガイドを非表示にしてから書き出してください。",
                };
                self.psd_info = format!(
                    "{source}\n大きさ: {} x {} px\nレイヤー: {}{merged}{stretch}{guides}",
                    loaded.source_size[0],
                    loaded.source_size[1],
                    if loaded.layer_names.is_empty() { "なし".to_owned() } else { loaded.layer_names.join("、") }
                );
                self.custom = Some(loaded.rgba);
                self.custom_bytes = Some(bytes.to_vec());
                self.art_dirty = true;
            }
            Err(e) => self.psd_info = format!("{source}\n{e}"),
        }
    }

    fn open_psd(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => self.apply_psd(&bytes, &path.display().to_string()),
            Err(e) => self.psd_info = format!("{} を読めませんでした: {e}", path.display()),
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("新パケノバ 試作 S03");
        ui.label("曲面を含む型（手提げ型）の確認用です。");
        ui.add_space(10.0);

        ui.strong("1. 開き具合");
        if ui.add(egui::Slider::new(&mut self.fold, 0.0..=1.0).text("展開図 → 箱")).changed() {
            self.animate_to = None;
        }
        ui.horizontal(|ui| {
            if ui.button("組み立てる").clicked() {
                self.animate_to = Some(1.0);
            }
            if ui.button("開く").clicked() {
                self.animate_to = Some(0.0);
            }
        });
        ui.add_space(10.0);

        ui.strong("2. 型と寸法（内寸、mm）");
        ui.horizontal_wrapped(|ui| {
            for (sides, label) in SHAPES {
                ui.radio_value(&mut self.dims.sides, sides, label);
            }
        });
        let prism = self.dims.sides >= 3;
        ui.add(egui::Slider::new(&mut self.dims.w, 40.0..=300.0).text(if prism { "一辺" } else { "幅" }));
        if !prism {
            ui.add(egui::Slider::new(&mut self.dims.d, 20.0..=200.0).text("奥行き"));
        }
        let bag = self.dims.sides == 1;
        ui.add(egui::Slider::new(&mut self.dims.h, 40.0..=300.0).text(if bag { "前後の長さ" } else { "高さ" }));
        if bag {
            ui.add(egui::Slider::new(&mut self.dims.bulge, 0.0..=75.0).text("ふくらみ（度）"));
        }
        ui.add(egui::Slider::new(&mut self.dims.t, 0.3..=6.0).step_by(0.1).text("紙の厚み"));
        let d = self.dims;
        if !prism && !bag {
            ui.weak(format!("外寸: {:.1} x {:.1} x {:.1} mm", d.w + 2.0 * d.t, d.d + 2.0 * d.t, d.h + 4.0 * d.t));
        }
        ui.weak(format!("展開図: {:.0} x {:.0} mm", self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]));
        ui.add_space(10.0);

        ui.strong("3. 絵柄");
        if ui.checkbox(&mut self.show_guides, "切り線と折り線を表示").changed() {
            self.art_dirty = true;
        }
        if ui.button("型紙を PSD で保存").clicked() {
            self.note = match save_psd(&self.sheet).and_then(|b| std::fs::write(PSD_NAME, b).map_err(|e| e.to_string())) {
                Ok(()) => {
                    let full = std::env::current_dir().map(|dir| dir.join(PSD_NAME)).unwrap_or_else(|_| PathBuf::from(PSD_NAME));
                    format!("保存しました: {}", full.display())
                }
                Err(e) => format!("保存できませんでした: {e}"),
            };
        }
        if !self.note.is_empty() {
            ui.weak(&self.note);
        }
        ui.label("保存した PSD に絵を描いて、ウィンドウにドロップすると箱に貼られます。");
        let mut load_clicked = false;
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            load_clicked = ui.button("読み込む").clicked();
            ui.add(egui::TextEdit::singleline(&mut self.psd_path).hint_text("PSD のパス").desired_width(f32::INFINITY));
        });
        if load_clicked {
            let path = PathBuf::from(self.psd_path.trim().trim_matches('"'));
            self.open_psd(&path);
        }
        ui.label(&self.psd_info);
        if self.custom.is_some() && ui.button("確認用の絵柄に戻す").clicked() {
            self.custom = None;
            self.custom_bytes = None;
            self.psd_info = "確認用の絵柄".to_owned();
            self.art_dirty = true;
        }
        ui.add_space(10.0);

        ui.weak(format!("パネル {} 枚、三角形 {} 個", self.template.panels.len(), self.triangles));
        let mut names: Vec<&str> = self.template.panels.iter().map(|p| p.name).collect();
        names.dedup();
        ui.weak(names.join("、"));
        ui.weak(format!("描画: {}", self.backend));
        ui.weak("箱の上をドラッグで回転、ホイールで拡大縮小。");
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        if response.dragged() {
            let delta = response.drag_delta();
            self.yaw -= delta.x * 0.01;
            self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.5, 1.5);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.3, 6.0);
            }
        }

        let Some(rs): Option<egui_wgpu::RenderState> = frame.wgpu_render_state().cloned() else {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "wgpu を使えないため、箱を表示できません。", egui::FontId::proportional(16.0), egui::Color32::LIGHT_RED);
            return;
        };
        if self.backend.is_empty() {
            let info = rs.adapter.get_info();
            self.backend = format!("{:?} / {}", info.backend, info.name);
        }
        let renderer = self.renderer.get_or_insert_with(|| BoxRenderer::new(&rs));
        if self.art_dirty {
            let rgba = compose(&self.sheet, self.custom.as_deref(), self.show_guides);
            renderer.set_art(&rs, self.sheet.w, self.sheet.h, &rgba);
            self.art_dirty = false;
        }

        let mesh = self.template.build_mesh(self.fold);
        self.triangles = mesh.len() / 3;
        let (center, radius) = bounding_sphere(&mesh);

        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round().max(2.0) as u32, (rect.height() * ppp).round().max(2.0) as u32];
        let aspect = size[0] as f32 / size[1] as f32;
        let fov = 30f32.to_radians();
        // 縦長の画面でも箱がはみ出さないように、狭いほうに合わせて離れる。
        let dist = radius / (fov * 0.5).sin() * 1.1 / self.zoom / aspect.min(1.0);
        let dir = [self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos()];
        let eye = [center[0] + dir[0] * dist, center[1] + dir[1] * dist, center[2] + dir[2] * dist];
        let view = look_at(eye, center, [0.0, 1.0, 0.0]);
        let proj = perspective(fov, aspect, (dist - radius * 1.5).max(dist * 0.02), dist + radius * 2.0);
        // 光は視点の少し右上から当てる（どの角度から見ても手前の面が明るくなる）。
        let right = normalize([view[0][0], view[1][0], view[2][0]]);
        let light = normalize([dir[0] + right[0] * 0.45, dir[1] + 0.55, dir[2] + right[2] * 0.45]);

        // 抜き穴を、絵柄の座標（0〜1）に直して渡す。
        let (sheet_w, sheet_h) = (self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]);
        let mut holes = [[0.0f32; 4]; 2];
        for (slot, h) in holes.iter_mut().zip(&self.template.holes) {
            *slot = [(h[0] - self.template.min[0]) / sheet_w, (self.template.max[1] - h[1]) / sheet_h, h[2] / sheet_w, h[3] / sheet_h];
        }
        if let Some(id) = renderer.render(&rs, size, &mesh, mat4_mul(&proj, &view), light, holes) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, egui::Color32::WHITE);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // 開閉のアニメーション。全体で約 3 秒。
        if let Some(target) = self.animate_to {
            let step = ui.input(|i| i.stable_dt).min(0.05) / 3.0;
            if (target - self.fold).abs() <= step {
                self.fold = target;
                self.animate_to = None;
            } else {
                self.fold += step * (target - self.fold).signum();
                ui.ctx().request_repaint();
            }
        }

        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(path) = dropped.first() {
            self.psd_path = path.display().to_string();
            self.open_psd(path);
        }

        // 幅は固定する。中身がはみ出してもパネルが広がらないようにするため。
        egui::Panel::left("controls").resizable(false).exact_size(360.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.controls(ui));
        });
        self.rebuild_if_needed();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui, frame));
    }
}
