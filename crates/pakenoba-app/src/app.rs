//! ウィンドウ（egui / eframe）。

use std::path::{Path, PathBuf};

use eframe::egui;
use eframe::egui_wgpu;

use pakenoba_core::check::check;
use pakenoba_core::math::{cross, look_at, mat4_mul, normalize, perspective, sub};
use pakenoba_core::surface::{Decal, Surface, pick, ray_through};
use pakenoba_core::template::{P2, Template, bounding_sphere};

use crate::art::{GuideHandling, SheetArt, compose, fit, load_psd, make_sheet_art, save_psd};
use crate::gpu::BoxRenderer;
use crate::paint::{Picture, Placed, paint, spots};

const PSD_NAME: &str = "pakenoba-dieline.psd";

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
    /// 箱の型。0 は差し込み箱、2 はゲーブル型、10 はピロー型、それ以外の 3 以上は正多角柱の角の数。
    pub sides: usize,
    /// 起動時に開くワークスペース（0: 型、1: デザイン、2: 出力）。
    pub workspace: usize,
    /// お客さんが開けた度合い（0 が閉じた状態、1 が開いた状態）。
    pub open: f32,
    /// 手提げ型とピロー型のふくらみ（度）。指定がなければ、型ごとの初期値を使う。
    pub bulge: Option<f32>,
    /// 幅（ピロー型では横の長さ）と高さ（mm）。指定がなければ、型ごとの初期値を使う。
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// 起動時に柄として置く画像（PNG）。
    pub png: Option<PathBuf>,
    /// 起動時に置くロゴと帯の場所。「パネルの番号、横の位置、縦の位置、回転（度）」。
    pub logo: Option<[f32; 4]>,
    pub band: Option<[f32; 4]>,
    /// 起動時に選んでおく柄の番号。
    pub select: Option<usize>,
}

impl Default for Initial {
    fn default() -> Self {
        Initial { fold: 1.0, yaw: 0.6, pitch: 0.35, thickness: 0.5, guides: true, psd: None, sides: 0, workspace: 0, open: 0.0, bulge: None, width: None, height: None, png: None, logo: None, band: None, select: None }
    }
}

pub fn run(initial: Initial) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("パケノバ", options, Box::new(move |_cc| Ok(Box::new(App::new(&initial)))))
}

/// ピロー型を表す番号。角柱の角の数と重ならない値にしてある。
pub const PILLOW: usize = 10;

#[derive(Clone, Copy, PartialEq)]
struct Dims {
    /// 0 は差し込み箱、1 は丸みのある手提げ型、2 はゲーブル型、10 はピロー型、それ以外の 3 以上は正多角柱の角の数。
    sides: usize,
    /// 差し込み箱と手提げ型では幅、角柱では一辺の長さ、ピロー型では横の長さ。
    w: f32,
    d: f32,
    /// 差し込み箱と角柱では高さ、手提げ型では正面と背面の長さ、ピロー型では 1 枚の面の高さ。
    h: f32,
    t: f32,
    /// 手提げ型とピロー型のふくらみ（度）。
    bulge: f32,
    /// ゲーブル型の屋根の高さ。
    roof: f32,
}

impl Dims {
    fn template(&self) -> Template {
        match self.sides {
            2 => Template::gable(self.w, self.d, self.h, self.roof, self.t),
            PILLOW => Template::pillow(self.w, self.h, self.t, self.bulge.to_radians()),
            n if n >= 3 => Template::prism(n, self.w, self.h, self.t),
            _ => Template::tuck_box(self.w, self.d, self.h, self.t),
        }
    }

    /// 型紙と、柄がたどれる面のつながりと、検査の結果。
    /// 合わせ目と検査には、厚みをほぼ 0 にした同じ寸法の型紙を使う。
    fn build(&self) -> (Template, Surface, Vec<(bool, String)>) {
        let template = self.template();
        let thin = Dims { t: 0.02, ..*self }.template();
        let surface = Surface::new(&template, &thin);
        let problems = check(&template, &thin).iter().map(|p| (p.is_blocking(), p.describe(&template))).collect();
        (template, surface, problems)
    }

    /// 柄を置く、最初のパネル（正面。なければ最初のパネル）と、箱を一周する帯の長さ。
    fn front_and_around(&self, tpl: &Template) -> (usize, f32) {
        let front = tpl.panels.iter().position(|p| p.name == "正面").unwrap_or(0);
        let around = match self.sides {
            0 => 2.0 * (self.w + self.d),
            2 => 2.0 * (self.w + self.d.min(self.w)),
            PILLOW => self.w,
            n => n as f32 * self.w,
        };
        (front, around)
    }
}

/// PNG を読んで、RGBA（アルファは乗算なし）にする。
fn load_png(path: &Path) -> Result<Picture, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{} を開けませんでした: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("PNG を読めませんでした: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("画像が大きすぎます")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("PNG を読めませんでした: {e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => return Err(format!("この形式の PNG は扱えません（{other:?}）")),
    };
    if w == 0 || h == 0 || buf.len() < w * h * channels {
        return Err("PNG の大きさが合いません".to_owned());
    }
    let rgba: Vec<u8> = buf[..w * h * channels]
        .chunks_exact(channels)
        .flat_map(|p| match channels {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        })
        .collect();
    Ok(Picture::Image { w, h, rgba: std::sync::Arc::new(rgba) })
}

/// 選べる型。検査を通ったものだけを並べる。
const SHAPES: [(usize, &str); 7] = [(0, "差し込み箱"), (2, "ゲーブル型"), (PILLOW, "ピロー型"), (3, "三角柱"), (5, "五角柱"), (6, "六角柱"), (8, "八角柱")];

/// ワークスペース。左から順に進めば完成する。
#[derive(Clone, Copy, PartialEq)]
enum Workspace {
    /// 型: 箱の形と寸法を決める。
    Shape,
    /// デザイン: 絵柄を置く。
    Design,
    /// 出力: 見せるための画像や、型紙を書き出す。
    Output,
}

struct App {
    dims: Dims,
    built_dims: Option<Dims>,
    template: Template,
    /// 柄がたどれる面のつながり（折り線と合わせ目）。
    surface: Surface,
    /// 型紙の検査の結果。（箱として成り立たない問題か、説明）。
    problems: Vec<(bool, String)>,
    workspace: Workspace,
    /// 立体の上で置いた柄。後ろのものほど上に重なる。
    decals: Vec<Placed>,
    selected: Option<usize>,
    /// つかんでいる柄の番号と、柄の座標でのつかんだ位置。
    grab: Option<(usize, P2)>,
    /// カーソルの下のパネル。
    hover: Option<usize>,
    sheet: SheetArt,
    /// 読み込んだ PSD の絵（合わせる前の画像と、その大きさ）。寸法を変えたら、展開図の大きさに合わせ直す。
    custom_source: Option<(Vec<u8>, [usize; 2])>,
    custom: Option<Vec<u8>>,
    show_guides: bool,
    art_dirty: bool,
    fold: f32,
    /// アニメーションの行き先（0 か 1）。
    animate_to: Option<f32>,
    /// 視点を動かす先（左右と上下の角度）。展開図に戻すときに、印刷面が見える正面へ向ける。
    view_to: Option<(f32, f32)>,
    /// 展開図に戻す前の視点。組み立て直すときに、ここへ戻す。
    view_before_flat: Option<(f32, f32)>,
    /// お客さんが開けた度合い（0 が閉じた状態、1 が開いた状態）と、そのアニメーションの行き先。
    open: f32,
    open_to: Option<f32>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    renderer: Option<BoxRenderer>,
    psd_path: String,
    psd_info: String,
    note: String,
    triangles: usize,
    backend: String,
    decal_note: String,
}

impl App {
    fn new(initial: &Initial) -> Self {
        let sides = initial.sides;
        let (w, d, h) = match sides {
            2 => (160.0, 100.0, 110.0),
            PILLOW => (180.0, 60.0, 110.0),
            n if n >= 3 => (70.0, 60.0, 160.0),
            _ => (120.0, 60.0, 160.0),
        };
        let (w, h) = (initial.width.unwrap_or(w), initial.height.unwrap_or(h));
        let bulge = initial.bulge.unwrap_or(if sides == PILLOW { 70.0 } else { 40.0 });
        let dims = Dims { sides, w, d, h, t: initial.thickness, bulge, roof: 45.0 };
        let (template, surface, problems) = dims.build();
        let sheet = make_sheet_art(&template);
        let (front, around) = dims.front_and_around(&template);
        let logo = 46.0;
        let mut decals = Vec::new();
        for (picture, size, place) in [(Picture::Band, [around, 14.0], initial.band), (Picture::Logo, [logo, logo], initial.logo)] {
            if let Some([panel, x, y, degrees]) = place {
                let panel = Some((panel as usize).min(template.panels.len() - 1)).filter(|&i| !template.panels[i].hidden_when_closed()).unwrap_or(front);
                decals.push(Placed { picture, decal: Decal { panel, at: [x, y], size, angle: degrees.to_radians() } });
            }
        }
        let mut app = App {
            dims,
            built_dims: Some(dims),
            template,
            surface,
            problems,
            workspace: [Workspace::Shape, Workspace::Design, Workspace::Output][initial.workspace.min(2)],
            decals,
            selected: None,
            grab: None,
            hover: None,
            sheet,
            custom_source: None,
            custom: None,
            show_guides: initial.guides,
            art_dirty: true,
            fold: initial.fold.clamp(0.0, 1.0),
            animate_to: None,
            view_to: None,
            view_before_flat: None,
            open: initial.open.clamp(0.0, 1.0),
            open_to: None,
            yaw: initial.yaw,
            pitch: initial.pitch,
            zoom: 1.0,
            renderer: None,
            psd_path: String::new(),
            psd_info: String::new(),
            note: String::new(),
            triangles: 0,
            backend: String::new(),
            decal_note: String::new(),
        };
        if let Some(path) = &initial.psd {
            app.psd_path = path.display().to_string();
            app.open_psd(path);
        }
        if let Some(path) = &initial.png {
            app.add_png(path);
        }
        app.selected = initial.select.filter(|&i| i < app.decals.len());
        app
    }

    /// 寸法が変わっていたら、型紙と絵柄を作り直す。
    fn rebuild_if_needed(&mut self) {
        if self.built_dims == Some(self.dims) {
            return;
        }
        let shape_changed = self.built_dims.map(|d| d.sides) != Some(self.dims.sides);
        (self.template, self.surface, self.problems) = self.dims.build();
        self.sheet = make_sheet_art(&self.template);
        self.built_dims = Some(self.dims);
        if shape_changed {
            // 型を変えたら、パネルの並びが変わるので、置いた柄は外す。
            // 寸法だけを変えたときは、柄はパネルの中での位置で持っているので、そのまま付いてくる。
            self.decals.clear();
            self.selected = None;
            self.grab = None;
        }
        if let Some((source, size)) = &self.custom_source {
            // 読み込んだ絵は、読み直さずに、展開図の新しい大きさへ合わせ直すだけにする。
            self.custom = Some(fit(source, *size, self.sheet.w, self.sheet.h));
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
                    GuideHandling::Removed => "\nガイドのレイヤー（と、立体の上で置いた柄のレイヤー）は除いて読み込みました。",
                    GuideHandling::Baked => "\n注意: ガイドのレイヤーを除けませんでした（グループ、マスク、通常以外の合成モードがあります）。ガイドを非表示にしてから書き出してください。",
                };
                self.psd_info = format!(
                    "{source}\n大きさ: {} x {} px\nレイヤー: {}{merged}{stretch}{guides}",
                    loaded.source_size[0],
                    loaded.source_size[1],
                    if loaded.layer_names.is_empty() { "なし".to_owned() } else { loaded.layer_names.join("、") }
                );
                self.custom = Some(loaded.rgba);
                self.custom_source = Some((loaded.source, loaded.source_size));
                self.art_dirty = true;
            }
            Err(e) => self.psd_info = format!("{source}\n{e}"),
        }
    }

    /// 柄を 1 つ足して、選ぶ。正面（なければ最初のパネル）の中央に置く。
    fn add_decal(&mut self, picture: Picture, size: P2) {
        let panel = self.template.panels.iter().position(|p| p.name == "正面").unwrap_or(0);
        self.decals.push(Placed { picture, decal: Decal { panel, at: [0.5, 0.5], size, angle: 0.0 } });
        self.selected = Some(self.decals.len() - 1);
        self.art_dirty = true;
    }

    fn add_png(&mut self, path: &Path) {
        match load_png(path) {
            Ok(picture) => {
                let Picture::Image { w, h, .. } = &picture else { return };
                let size = [50.0, 50.0 * *h as f32 / *w as f32];
                self.decal_note = format!("{} を柄として置きました。", path.display());
                self.add_decal(picture, size);
            }
            Err(e) => self.decal_note = e,
        }
    }

    /// 柄を 1 つずつ、展開図の大きさの透明な画像に描く（PSD のレイヤー用）。名前は「番号（種類）」。
    fn decal_layers(&self) -> Vec<(String, Vec<u8>)> {
        self.decals
            .iter()
            .enumerate()
            .map(|(index, decal)| {
                let mut layer = vec![0u8; self.sheet.w * self.sheet.h * 4];
                paint(&self.surface, &self.template, std::slice::from_ref(decal), None, self.sheet.w, self.sheet.h, &mut layer);
                (format!("{}（{}）", index + 1, decal.picture.label()), layer)
            })
            .collect()
    }

    fn decal_controls(&mut self, ui: &mut egui::Ui) {
        ui.strong("柄");
        ui.label("PNG をウィンドウにドロップすると、画像が柄として箱に乗ります。箱の上の柄をドラッグすると面の上を動き、折り線や合わせ目をまたいで隣の面へ続きます。");
        ui.horizontal(|ui| {
            ui.weak("見本:");
            if ui.button("ロゴ").clicked() {
                self.add_decal(Picture::Logo, [46.0, 46.0]);
            }
            if ui.button("一周する帯").clicked() {
                let (_, around) = self.dims.front_and_around(&self.template);
                self.add_decal(Picture::Band, [around, 14.0]);
            }
        });
        if !self.decal_note.is_empty() {
            ui.weak(&self.decal_note);
        }
        ui.horizontal_wrapped(|ui| {
            for index in 0..self.decals.len() {
                let label = format!("{} {}", self.decals[index].picture.label(), index + 1);
                if ui.selectable_label(self.selected == Some(index), label).clicked() {
                    self.selected = if self.selected == Some(index) { None } else { Some(index) };
                    self.art_dirty = true;
                }
            }
        });
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else {
            if !self.decals.is_empty() {
                ui.weak("柄をクリックすると選べます。");
            }
            return;
        };
        let decal = &mut self.decals[index];
        let before = (decal.size, decal.angle);
        let mut width = decal.size[0];
        ui.add(egui::Slider::new(&mut width, 8.0..=600.0).logarithmic(true).text("幅（mm）"));
        if matches!(decal.picture, Picture::Band) {
            ui.add(egui::Slider::new(&mut decal.size[1], 4.0..=60.0).text("高さ（mm）"));
            decal.size[0] = width;
        } else {
            // ロゴと画像は、縦横の比を保つ。
            decal.size = [width, decal.size[1] * width / decal.size[0]];
        }
        let mut degrees = decal.angle.to_degrees();
        ui.add(egui::Slider::new(&mut degrees, -180.0..=180.0).text("回転（度）"));
        decal.angle = degrees.to_radians();
        if before != (decal.size, decal.angle) {
            self.art_dirty = true;
        }
        // どのパネルにかかっているか。
        let pieces = self.surface.layout(&self.template, &self.decals[index]);
        let mut names: Vec<&str> = pieces.iter().map(|p| self.template.panels[p.panel].name).collect();
        names.dedup();
        let seams = pieces.iter().map(|p| p.seams).max().unwrap_or(0);
        ui.weak(format!("かかっている面: {}", names.join("、")));
        ui.weak(if seams > 0 { format!("合わせ目を {seams} 回またいでいます。") } else { "合わせ目はまたいでいません。".to_owned() });
        if ui.button("この柄を消す").clicked() {
            self.decals.remove(index);
            self.selected = None;
            self.grab = None;
            self.art_dirty = true;
        }
    }

    fn open_psd(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => self.apply_psd(&bytes, &path.display().to_string()),
            Err(e) => self.psd_info = format!("{} を読めませんでした: {e}", path.display()),
        }
    }

    /// 型: 箱の形と寸法を決める。
    fn shape_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("型");
        ui.horizontal_wrapped(|ui| {
            for (sides, label) in SHAPES {
                ui.radio_value(&mut self.dims.sides, sides, label);
            }
        });
        ui.add_space(8.0);

        ui.strong("寸法（内寸、mm）");
        let gable = self.dims.sides == 2;
        let pillow = self.dims.sides == PILLOW;
        let prism = self.dims.sides >= 3 && !pillow;
        ui.add(egui::Slider::new(&mut self.dims.w, 40.0..=300.0).text(if prism { "一辺" } else if pillow { "横の長さ" } else { "幅" }));
        if !prism && !pillow {
            ui.add(egui::Slider::new(&mut self.dims.d, 20.0..=200.0).text("奥行き"));
        }
        ui.add(egui::Slider::new(&mut self.dims.h, 40.0..=300.0).text(if gable { "胴の高さ" } else if pillow { "面の高さ" } else { "高さ" }));
        if gable {
            ui.add(egui::Slider::new(&mut self.dims.roof, 10.0..=150.0).text("屋根の高さ"));
            if self.dims.d > self.dims.w {
                ui.weak("奥行きは幅までに抑えています（側面の三角形が中央でぶつかるため）。");
            }
        }
        if pillow {
            ui.add(egui::Slider::new(&mut self.dims.bulge, 15.0..=90.0).text("ふくらみ（度）"));
            let psi = self.dims.bulge.clamp(15.0, 90.0).to_radians();
            let deepest = self.dims.h * (1.0 - (psi * 0.5).cos()) / psi;
            ui.weak(format!("いちばん厚いところ: 約 {:.0} mm", deepest * 2.0));
        }
        ui.add(egui::Slider::new(&mut self.dims.t, 0.3..=6.0).step_by(0.1).text("紙の厚み"));
        let d = self.dims;
        if !prism && !gable && !pillow {
            ui.weak(format!("外寸: {:.1} x {:.1} x {:.1} mm", d.w + 2.0 * d.t, d.d + 2.0 * d.t, d.h + 4.0 * d.t));
        }
        ui.weak(format!("展開図: {:.0} x {:.0} mm", self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]));
        ui.add_space(8.0);

        ui.strong("型紙の検査");
        if self.problems.is_empty() {
            ui.label("問題は見つかりませんでした。");
        }
        for (blocking, text) in &self.problems {
            if *blocking {
                ui.colored_label(egui::Color32::from_rgb(240, 120, 100), format!("問題: {text}"));
            } else {
                ui.label(format!("注意: {text}"));
            }
        }
        ui.weak("閉じていない辺、開け口と留め、のりしろの貼る相手、展開図の重なりを見ています。");
        ui.add_space(8.0);

        self.motion_controls(ui);
    }

    /// 組み立てと、開け閉め。
    fn motion_controls(&mut self, ui: &mut egui::Ui) {
        ui.strong("組み立て");
        if ui.add(egui::Slider::new(&mut self.fold, 0.0..=1.0).text("展開図 → 箱")).changed() {
            self.animate_to = None;
        }
        ui.horizontal(|ui| {
            if ui.button("組み立てる").clicked() {
                self.animate_to = Some(1.0);
                // 展開図に戻す前の視点があれば、そこへ戻す。
                self.view_to = self.view_before_flat.take();
            }
            if ui.button("展開図に戻す").clicked() {
                self.animate_to = Some(0.0);
                // 展開図は、印刷面が手前を向く正面から見る（裏の白い面が見えてしまわないように）。
                if self.view_before_flat.is_none() {
                    self.view_before_flat = Some((self.yaw, self.pitch));
                }
                self.view_to = Some((0.0, 0.0));
            }
        });
        if self.template.open_angle > 0.0 {
            ui.add_space(4.0);
            ui.strong("開け閉め");
            if ui.add(egui::Slider::new(&mut self.open, 0.0..=1.0).text("閉じた → 開いた")).changed() {
                // 開け閉めは、組み上がった箱に対する動き。
                self.fold = 1.0;
                self.animate_to = None;
                self.open_to = None;
            }
            ui.horizontal(|ui| {
                if ui.button("開ける").clicked() {
                    self.fold = 1.0;
                    self.open_to = Some(1.0);
                }
                if ui.button("閉じる").clicked() {
                    self.fold = 1.0;
                    self.open_to = Some(0.0);
                }
            });
        }
    }

    /// デザイン: 絵柄を置く。
    fn design_panel(&mut self, ui: &mut egui::Ui) {
        self.decal_controls(ui);
        ui.add_space(10.0);

        ui.strong("展開図まるごとの絵");
        ui.label("「出力」で保存した型紙の PSD に絵を描いて、ウィンドウにドロップすると箱に貼られます。");
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
        if !self.psd_info.is_empty() {
            ui.label(&self.psd_info);
        }
        if self.custom.is_some() && ui.button("絵を外す").clicked() {
            self.custom = None;
            self.custom_source = None;
            self.psd_info.clear();
            self.art_dirty = true;
        }
        ui.add_space(10.0);

        ui.strong("表示");
        if ui.checkbox(&mut self.show_guides, "切り線、折り線、のりしろを表示").changed() {
            self.art_dirty = true;
        }
        ui.add_space(8.0);
        self.motion_controls(ui);
    }

    /// 出力: 見せるための画像や、型紙を書き出す。
    fn output_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("型紙");
        ui.label("切り線、折り線、のりしろと、置いた絵柄を、レイヤーに分けた PSD で保存します。");
        if ui.button("型紙を PSD で保存").clicked() {
            self.note = match save_psd(&self.sheet, &self.decal_layers()).and_then(|b| std::fs::write(PSD_NAME, b).map_err(|e| e.to_string())) {
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
        ui.weak("この型紙はデザイン確認用です。製造、抜型、裁断、印刷位置の保証に使える正式なデータではありません。");
        ui.add_space(10.0);

        ui.strong("見せるための出力");
        ui.weak("まだありません。次のものを、ボタン 1 つで書き出せるようにします。");
        for item in ["16:9 の画像（スライド用）", "箱開け動画", "店の棚に並べた絵", "実寸の型紙（A4 や A3 に分けて印刷）"] {
            ui.weak(format!("・{item}"));
        }
    }

    /// 画面の下に出す、いまの状態。
    fn status(&self) -> String {
        let hover = self.hover.and_then(|i| self.template.panels.get(i)).map_or(String::new(), |p| format!("カーソルの下: {}　", p.name));
        format!("{hover}パネル {} 枚　合わせ目 {} か所　三角形 {} 個　描画: {}", self.template.panels.len(), self.surface.seams.len(), self.triangles, self.backend)
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
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
        let mesh = self.template.build_mesh(self.fold, 1.0 - self.open);
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

        // カーソルの下にある、立体の上の点（パネルと、展開図上の位置）。
        let forward = normalize(sub(center, eye));
        let side = normalize(cross(forward, [0.0, 1.0, 0.0]));
        let upward = cross(side, forward);
        let template = &self.template;
        let under = |pos: egui::Pos2| {
            let (x, y) = ((pos.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (pos.y - rect.top()) / rect.height() * 2.0);
            pick(template, &mesh, eye, ray_through(forward, side, upward, fov, aspect, x, y))
        };
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let hit = pointer.and_then(under);
        self.hover = hit.map(|(panel, _)| panel);
        let places = spots(&self.decals);
        let decal_under = hit.and_then(|(panel, at)| self.surface.decal_at(template, &places, panel, at));
        if response.drag_started() {
            // 押した場所に柄があれば、柄をつかむ。なければ、箱を回す。
            let pressed = ui.input(|i| i.pointer.press_origin()).and_then(under);
            self.grab = pressed.and_then(|(panel, at)| self.surface.decal_at(template, &places, panel, at));
            if let Some((index, _)) = self.grab {
                self.selected = Some(index);
                self.art_dirty = true;
            }
        }
        if response.dragged() {
            match (self.grab, hit) {
                (Some((index, grab)), Some((panel, at))) => {
                    if let Some(decal) = self.decals.get_mut(index) {
                        self.surface.drag(template, decal, panel, at, grab);
                        self.art_dirty = true;
                    }
                }
                (Some(_), None) => {}
                (None, _) => {
                    // 自分で回したら、視点の自動の動きは止める。
                    self.view_to = None;
                    let delta = response.drag_delta();
                    self.yaw -= delta.x * 0.01;
                    self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.5, 1.5);
                }
            }
        }
        if response.drag_stopped() {
            self.grab = None;
        }
        if response.clicked() {
            self.selected = decal_under.map(|(index, _)| index);
            self.art_dirty = true;
        }
        if self.grab.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if decal_under.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        let renderer = self.renderer.get_or_insert_with(|| BoxRenderer::new(&rs));
        if self.art_dirty {
            let (w, h) = (self.sheet.w, self.sheet.h);
            let rgba = compose(&self.sheet, self.custom.as_deref(), self.show_guides, |art| paint(&self.surface, &self.template, &self.decals, self.selected, w, h, art));
            renderer.set_art(&rs, w, h, &rgba);
            self.art_dirty = false;
        }

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
        // お客さんが開け閉めするアニメーション。約 1.5 秒。
        if let Some(target) = self.open_to {
            let step = ui.input(|i| i.stable_dt).min(0.05) / 1.5;
            if (target - self.open).abs() <= step {
                self.open = target;
                self.open_to = None;
            } else {
                self.open += step * (target - self.open).signum();
                ui.ctx().request_repaint();
            }
        }
        // 視点のアニメーション。近いほうの回り方で、行き先へなめらかに寄せる。
        if let Some((yaw, pitch)) = self.view_to {
            let turn = (yaw - self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let rise = pitch - self.pitch;
            if turn.abs() < 0.002 && rise.abs() < 0.002 {
                (self.yaw, self.pitch) = (yaw, pitch);
                self.view_to = None;
            } else {
                let step = (ui.input(|i| i.stable_dt).min(0.05) * 5.0).min(1.0);
                self.yaw += turn * step;
                self.pitch += rise * step;
                ui.ctx().request_repaint();
            }
        }
        // 組み立てのアニメーション。全体で約 3 秒。
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
        for path in &dropped {
            // PNG は柄として置く。それ以外は、展開図まるごとの絵（PSD）として読む。
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
                self.add_png(path);
            } else {
                self.psd_path = path.display().to_string();
                self.open_psd(path);
            }
        }

        // 上: ワークスペースの切り替え。左から順に進めば完成する。
        egui::Panel::top("workspaces").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.strong("パケノバ");
                ui.add_space(16.0);
                for (workspace, label) in [(Workspace::Shape, "型"), (Workspace::Design, "デザイン"), (Workspace::Output, "出力")] {
                    ui.selectable_value(&mut self.workspace, workspace, format!("  {label}  "));
                }
            });
        });
        // 下: いまの状態。
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.weak(self.status());
        });
        // 左: ワークスペースごとの道具。幅は固定する（中身がはみ出してもパネルが広がらないように）。
        egui::Panel::left("tools").resizable(false).exact_size(340.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                match self.workspace {
                    Workspace::Shape => self.shape_panel(ui),
                    Workspace::Design => self.design_panel(ui),
                    Workspace::Output => self.output_panel(ui),
                }
            });
        });
        self.rebuild_if_needed();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui, frame));
    }
}
