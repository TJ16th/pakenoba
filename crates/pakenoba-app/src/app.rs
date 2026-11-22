//! ウィンドウ（egui / eframe）。
//!
//! 画面の並びは `docs/ui-design.md` の案 C。上のタブで工程（型、デザイン、出力）を切り替え、
//! 左に「選ぶもの・道具」、右に「選んだものの設定」、下に「動き」を置く。
//! このファイルは、状態と、工程をまたぐ処理。画面の部品は `panels`、中央の表示は `views`。

mod assets;
mod base;
mod export;
mod file;
mod history;
mod icons;
mod library;
mod panels;
mod recovery;
mod select;
mod settings;
mod typing;
mod views;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui;
use eframe::egui_wgpu;

use pakenoba_core::check::check;
use pakenoba_core::project;
use pakenoba_core::surface::{Decal, Surface};
use pakenoba_core::template::{P2, Template};

use crate::art::{SheetArt, compose, compose_back, make_sheet_art};
use crate::gpu::BoxRenderer;
use crate::paint::{Canvas, Marks, Picture, Placed, paint};

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
    /// 箱の型。0 は差し込み箱、2 はゲーブル型、100 はピロー型、それ以外の 3 以上は正多角柱の角の数。
    pub sides: usize,
    /// 起動時に開くワークスペース（0: 型、1: デザイン、2: 出力）。
    pub workspace: usize,
    /// お客さんが開けた度合い（0 が閉じた状態、1 が開いた状態）。
    pub open: f32,
    /// 底の開き具合（0 が閉じた状態、1 が開けきった状態）。
    pub open_bottom: f32,
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
    /// デザインの工程で出しておく表示（立体、表、裏）。
    pub views: Option<[bool; 3]>,
    /// 起動時に、紙の裏へ移しておく柄の番号。
    pub back: Vec<usize>,
    /// 起動時に素材の欄を開いておく。真なら道具の帯の箱側、偽なら道具の帯の外側。
    pub assets: Option<bool>,
    /// 開発用の情報（パネルの数、描画の方式など）を、下の帯に出す。
    pub debug: bool,
    /// 起動時に開く案件のファイル（`.pknb`）。
    pub project: Option<PathBuf>,
    /// 画面を撮って確かめるとき用。ファイルを選ぶダイアログを出さずに、この場所を答えにする（開く、保存する）。
    pub answer_open: Option<PathBuf>,
    pub answer_save: Option<PathBuf>,
    /// 同じく、差し替える画像を選ぶダイアログの答え。
    pub answer_replace: Option<PathBuf>,
    /// 同じく、型紙の PSD の保存先を選ぶダイアログの答え。
    pub answer_export: Option<PathBuf>,
    /// 落ちたときの控えを置くフォルダ。指定がなければ、使う人のデータのフォルダの中。
    pub recovery_dir: Option<PathBuf>,
    /// 控えを書く間隔（秒）。
    pub backup_seconds: f64,
    /// 設定のファイル（テスト用。指定がなければ、いつもの場所）。
    pub settings: Option<PathBuf>,
    /// 起動したときに「素材」の画面を開く。
    pub library: bool,
    /// テスト用: 素材のフォルダを選ぶダイアログの代わりの答え。
    pub answer_folder: Option<PathBuf>,
}

impl Default for Initial {
    fn default() -> Self {
        Initial { fold: 1.0, yaw: 0.6, pitch: 0.35, thickness: 0.5, guides: true, psd: None, sides: 0, workspace: 0, open: 0.0, open_bottom: 0.0, bulge: None, width: None, height: None, png: None, logo: None, band: None, select: None, views: None, back: Vec::new(), assets: None, debug: false, project: None, answer_open: None, answer_save: None, answer_replace: None, answer_export: None, recovery_dir: None, backup_seconds: 30.0, settings: None, library: false, answer_folder: None }
    }
}

pub fn run(initial: Initial) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "パケノバ",
        options,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new(&initial)))
        }),
    )
}

/// 型の工程の、はじめの視点（斜め上から）。向きと、見下ろす角度。
const SHAPE_VIEW: (f32, f32) = (0.6, 0.35);

/// パケノバの色（ティール）。選択中と、いちばん主なボタンにだけ使う。
const TEAL: egui::Color32 = egui::Color32::from_rgb(8, 126, 139);

/// 文字の大きさと色。本文 13 px、補足 12 px。既定は暗いテーマ。
fn setup_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        for (text_style, size) in [(TextStyle::Body, 13.0), (TextStyle::Button, 13.0), (TextStyle::Small, 12.0), (TextStyle::Heading, 13.0)] {
            style.text_styles.insert(text_style, FontId::new(size, FontFamily::Proportional));
        }
        style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
        // 文字は、暗い地の上で、はっきり読める明るさにする（egui の既定は、本文も補足も暗すぎる）。
        // 本文は地との明るさの比がおよそ 12:1、補足でも 7:1。
        style.visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_gray(216);
        style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_gray(222);
        style.visuals.weak_text_color = Some(egui::Color32::from_gray(164));
        style.visuals.selection.bg_fill = TEAL;
        style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}

/// ピロー型を表す番号。角柱の角の数（3〜12）と重ならない値にしてある。
pub const PILLOW: usize = 100;

#[derive(Clone, Copy, PartialEq)]
struct Dims {
    /// 0 は差し込み箱、1 は丸みのある手提げ型、2 はゲーブル型、100 はピロー型、それ以外の 3 以上は正多角柱の角の数。
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
    /// その型の、はじめの寸法。差し込み箱は、旧パケノバの既定（幅 200、高さ 85、奥行き 160 の横長の箱）。
    fn for_shape(sides: usize, t: f32) -> Dims {
        let (w, d, h) = match sides {
            2 => (160.0, 100.0, 110.0),
            PILLOW => (180.0, 60.0, 110.0),
            n if n >= 3 => (70.0, 60.0, 160.0),
            _ => (200.0, 160.0, 85.0),
        };
        Dims { sides, w, d, h, t, bulge: if sides == PILLOW { 70.0 } else { 40.0 }, roof: 45.0 }
    }

    /// 案件のファイルに書く形の、型と寸法。
    fn shape(&self) -> project::Shape {
        let kind = match self.sides {
            2 => project::Kind::Gable,
            PILLOW => project::Kind::Pillow,
            n if n >= 3 => project::Kind::Prism(n),
            _ => project::Kind::TuckBox,
        };
        project::Shape { kind, w: self.w, d: self.d, h: self.h, t: self.t, bulge: self.bulge, roof: self.roof }
    }

    fn from_shape(shape: &project::Shape) -> Dims {
        let sides = match shape.kind {
            project::Kind::TuckBox => 0,
            project::Kind::Gable => 2,
            project::Kind::Pillow => PILLOW,
            project::Kind::Prism(n) => n,
        };
        Dims { sides, w: shape.w, d: shape.d, h: shape.h, t: shape.t, bulge: shape.bulge, roof: shape.roof }
    }

    /// 型紙。案件のファイルを開いたときと同じ作り方にする（パネルの並びがずれないように）。
    fn template(&self) -> Template {
        self.shape().template()
    }

    /// 型紙と、柄がたどれる面のつながりと、検査の結果。
    /// 合わせ目と検査には、厚みをほぼ 0 にした同じ寸法の型紙を使う。
    fn build(&self) -> (Rc<Template>, Surface, Vec<(bool, String)>) {
        let template = self.template();
        let thin = Dims { t: 0.02, ..*self }.template();
        let surface = Surface::new(&template, &thin);
        let problems = check(&template, &thin).iter().map(|p| (p.is_blocking(), p.describe(&template))).collect();
        (Rc::new(template), surface, problems)
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

/// 柄の素材にできる画像の拡張子。
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "psd"];

/// 柄の素材にできる画像のファイルか（拡張子で見る）。
fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// 画像のファイルの中身を、RGBA（アルファは乗算なし）にする。PNG、JPEG、PSD（統合した絵）を、中身の先頭で見分ける。
fn decode_image(bytes: &[u8]) -> Result<Picture, String> {
    if bytes.starts_with(b"\x89PNG") {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(bytes)
    } else if bytes.starts_with(b"8BPS") {
        let (w, h, rgba) = crate::art::psd_picture(bytes)?;
        Ok(Picture::Image { w, h, rgba: std::sync::Arc::new(rgba) })
    } else {
        Err("画像として読めませんでした（扱えるのは PNG、JPEG、PSD です）".to_owned())
    }
}

/// JPEG のファイルの中身を、RGBA にする。CMYK の JPEG も、RGB に直して読む。
fn decode_jpeg(bytes: &[u8]) -> Result<Picture, String> {
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA).set_max_width(1 << 15).set_max_height(1 << 15);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let rgba = decoder.decode().map_err(|e| format!("JPEG を読めませんでした: {e:?}"))?;
    let (w, h) = decoder.dimensions().ok_or("JPEG の大きさが分かりません")?;
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err("JPEG の大きさが合いません".to_owned());
    }
    Ok(Picture::Image { w, h, rgba: std::sync::Arc::new(rgba) })
}

/// PNG のファイルの中身を、RGBA（アルファは乗算なし）にする。
fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
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

/// デザインの工程の道具。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Select,
    Text,
    Shape,
    Ai,
    Hand,
}

impl Tool {
    const ALL: [Tool; 5] = [Tool::Select, Tool::Text, Tool::Shape, Tool::Ai, Tool::Hand];

    /// まだ作っていない道具は、押せない。
    fn ready(self) -> bool {
        matches!(self, Tool::Select | Tool::Text | Tool::Hand)
    }

    /// ツールチップ。名前とショートカット。押せない道具は、その理由。
    fn tip(self) -> &'static str {
        match self {
            Tool::Select => "選択 (V)",
            Tool::Text => "文字 (T)",
            Tool::Shape => "図形（これから作ります）",
            Tool::Ai => "生成AI（これから作ります）",
            Tool::Hand => "手のひら (H)",
        }
    }
}

/// 中央に出せる表示。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Solid,
    Front,
    Back,
}

/// どの表示を出しているか。少なくとも 1 つは出ている。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Shown {
    solid: bool,
    front: bool,
    back: bool,
}

impl Shown {
    const SOLID_ONLY: Shown = Shown { solid: true, front: false, back: false };

    fn get(self, view: View) -> bool {
        match view {
            View::Solid => self.solid,
            View::Front => self.front,
            View::Back => self.back,
        }
    }

    fn count(self) -> usize {
        self.solid as usize + self.front as usize + self.back as usize
    }

    /// 出す、隠すを切り替える。最後の 1 つは隠せない（そのときは false を返す）。
    fn toggle(&mut self, view: View) -> bool {
        if self.get(view) && self.count() == 1 {
            return false;
        }
        match view {
            View::Solid => self.solid = !self.solid,
            View::Front => self.front = !self.front,
            View::Back => self.back = !self.back,
        }
        true
    }

    /// その表示だけにする。
    fn solo(&mut self, view: View) {
        *self = Shown { solid: view == View::Solid, front: view == View::Front, back: view == View::Back };
    }
}

/// 展開図の表示の、拡大と位置。
#[derive(Clone, Copy)]
struct FlatCam {
    zoom: f32,
    pan: egui::Vec2,
}

impl Default for FlatCam {
    fn default() -> Self {
        FlatCam { zoom: 1.0, pan: egui::Vec2::ZERO }
    }
}

/// 元に戻すときに取っておく状態。柄の並びと、選んでいる柄と、型。
/// 絵のデータは共有しているので、取っておいても軽い。
#[derive(Clone)]
struct Snapshot {
    decals: Vec<Placed>,
    selected: Option<usize>,
    /// いっしょに選んでいた柄。
    extra: Vec<usize>,
    /// 型と寸法。戻すときに使うのは、型の種類が違うときだけ（寸法だけの変更は、履歴の対象外）。
    dims: Dims,
    /// この案件の素材（付属の見本は除く）。差し替えを元に戻すため。
    library: Vec<assets::Stash>,
    /// 展開図まるごとの絵（表と裏）。差し替えと外すを元に戻すため（中身は共有しているので、軽い）。
    bases: [Option<base::BaseArt>; 2],
}

struct App {
    dims: Dims,
    built_dims: Option<Dims>,
    /// 型紙。共有して持つ（表示の中で、App を書き換えながら読むため）。
    template: Rc<Template>,
    /// 柄がたどれる面のつながり（折り線と合わせ目）。
    surface: Surface,
    /// 型紙の検査の結果。（箱として成り立たない問題か、説明）。
    problems: Vec<(bool, String)>,
    workspace: Workspace,
    tool: Tool,
    /// デザインの工程で出している表示。
    shown: Shown,
    /// 展開図の表示（表、裏）の、拡大と位置。
    flat: [FlatCam; 2],
    /// 出力の工程で選んでいる出力の種類。
    output_kind: usize,
    /// 素材（付属の見本と、この案件に取り込んだ画像）。
    assets: Vec<assets::Asset>,
    /// 素材の欄を開いているか。
    assets_open: bool,
    /// 素材の欄の位置。真なら「道具、素材、箱」の並び、偽なら「素材、道具、箱」の並び。
    assets_inside: bool,
    /// 素材の欄から箱へ運んでいる途中の素材。
    carry: Option<usize>,
    /// 柄の操作の履歴（元に戻す、やり直す）。
    history: history::History<Snapshot>,
    /// 数値の欄で値を変えている途中か。ひと続きの変更を、1 回ぶんの「元に戻す」にまとめる。
    editing: bool,
    /// 型を変えようとして、確認を待っている型。
    pending_shape: Option<usize>,
    /// 次に型紙を作り直すとき、柄を外さない（元に戻すで型を戻すとき）。
    keep_decals: bool,
    /// 素材の欄から運んでいる素材を、いま放したら置かれる柄。箱の上に、仮に描いて見せる。
    preview: Option<Placed>,
    /// 素材の欄で選んでいる素材。
    asset_selected: Option<usize>,
    /// 素材の欄の、絵の大きさ。
    thumb_size: f32,
    /// 工程ごとの視点（向きと拡大）。工程を移っても、それぞれの見え方を保つ。
    cams: [(f32, f32, f32); 3],
    /// いまの視点が、どの工程のものか。
    cam_owner: Workspace,
    debug: bool,
    /// 立体の上で置いた柄。後ろのものほど上に重なる。
    decals: Vec<Placed>,
    selected: Option<usize>,
    /// いっしょに選んでいる柄（`selected` のほか）。
    extra: Vec<usize>,
    /// カーソルの下の柄。描いてある強調（`hover_decal`）と、この画面で見つけたもの（`hover_now`）。
    hover_decal: Option<usize>,
    hover_now: Option<usize>,
    /// つかんでいる柄の番号と、柄の座標でのつかんだ位置。
    grab: Option<(usize, P2)>,
    /// カーソルの下のパネル。
    hover: Option<usize>,
    sheet: SheetArt,
    /// 展開図まるごとの絵（表と裏）。元のファイルと、読んだ絵。
    bases: [Option<base::BaseArt>; 2],
    /// 展開図の大きさに合わせた、表と裏の絵。寸法を変えたら、合わせ直す。
    custom: [Option<Vec<u8>>; 2],
    /// 案件のファイルにかかわる状態（場所、保存していない変更、保存の進み具合）。
    file: file::FileState,
    /// 右クリックのメニューなどで頼まれた、素材の差し替え。画面を描き終えてから行う。
    replace_request: Option<assets::Replace>,
    /// 画面を撮って確かめるとき用。差し替える画像を選ぶダイアログの答え。
    answer_replace: Option<PathBuf>,
    /// 同じく、型紙の PSD の保存先を選ぶダイアログの答え。
    answer_export: Option<PathBuf>,
    /// 落ちたときの控え。
    recovery: recovery::Recovery,
    /// 案件をまたいで覚えておく設定（登録したフォルダ）。
    settings: settings::Settings,
    /// 「素材」の画面。
    library: library::Library,
    /// OS のフォント（文字の柄を描く）。
    fonts: crate::text::Fonts,
    /// 次に置く文字の書式。
    text_style: pakenoba_core::project::TextSpec,
    /// 文字を置いた直後。右の欄の文字の入力に、カーソルを置く。
    focus_text: bool,
    /// レイヤーの欄で、名前を変えている柄と、入力中の名前。
    renaming: Option<(usize, String)>,
    /// レイヤーの欄で、ドラッグしている柄。
    layer_drag: Option<usize>,
    show_guides: bool,
    /// 柄や絵が変わったので、箱に貼る画像を作り直す。
    art_dirty: bool,
    /// 箱に貼る画像（展開図の表）。展開図の表示のために、取っておく。
    front_rgba: Vec<u8>,
    front_tex: Option<egui::TextureHandle>,
    front_tex_dirty: bool,
    /// 紙の裏（箱の内側の面）の画像。紙の地と、裏に置いた柄と、ガイド線。
    back_rgba: Vec<u8>,
    back_tex: Option<egui::TextureHandle>,
    back_tex_dirty: bool,
    /// 裏の画像を作り直す。型紙やガイド線の表示が変わったときと、裏の柄が変わったとき。
    back_dirty: bool,
    /// いまの裏の画像に、柄が描いてあるか。
    back_has_decals: bool,
    fold: f32,
    /// アニメーションの行き先（0 か 1）。
    animate_to: Option<f32>,
    /// 視点を動かす先（左右と上下の角度）。展開図に戻すときに、印刷面が見える正面へ向ける。
    view_to: Option<(f32, f32)>,
    /// 展開図に戻す前の視点。組み立て直すときに、ここへ戻す。
    view_before_flat: Option<(f32, f32)>,
    /// お客さんが開けた度合い（0 が閉じた状態、1 が開いた状態）と、そのアニメーションの行き先。
    /// 開け口ごとの開き具合（0 が閉じた状態、1 が開けきった状態）。1 つめが主な開け口（上）、2 つめが底。
    open: [f32; 2],
    open_to: [Option<f32>; 2],
    yaw: f32,
    pitch: f32,
    zoom: f32,
    renderer: Option<BoxRenderer>,
    psd_path: String,
    psd_info: String,
    /// 下の帯に出す知らせと、出し始めた時刻。数秒で消す。
    message: Option<(String, Option<f64>)>,
    triangles: usize,
    backend: String,
}

impl App {
    fn new(initial: &Initial) -> Self {
        let sides = initial.sides;
        let base = Dims::for_shape(sides, initial.thickness);
        let dims = Dims { w: initial.width.unwrap_or(base.w), h: initial.height.unwrap_or(base.h), bulge: initial.bulge.unwrap_or(base.bulge), ..base };
        let (template, surface, problems) = dims.build();
        let sheet = make_sheet_art(&template);
        let (front, around) = dims.front_and_around(&template);
        let logo = 46.0;
        let mut decals = Vec::new();
        for (picture, size, place) in [(Picture::Band, [around, 14.0], initial.band), (Picture::Logo, [logo, logo], initial.logo)] {
            if let Some([panel, x, y, degrees]) = place {
                let panel = Some((panel as usize).min(template.panels.len() - 1)).filter(|&i| template.panels[i].takes_art()).unwrap_or(front);
                decals.push(Placed { picture, decal: Decal { panel, at: [x, y], size, angle: degrees.to_radians() }, back: false, layer: Default::default() });
            }
        }
        let mut app = App {
            dims,
            built_dims: Some(dims),
            template,
            surface,
            problems,
            workspace: [Workspace::Shape, Workspace::Design, Workspace::Output][initial.workspace.min(2)],
            tool: Tool::Select,
            shown: initial.views.map(|[solid, front, back]| Shown { solid, front, back }).filter(|s| s.count() > 0).unwrap_or(Shown::SOLID_ONLY),
            flat: [FlatCam::default(); 2],
            output_kind: 0,
            assets: assets::bundled(),
            assets_open: initial.assets.is_some(),
            assets_inside: initial.assets.unwrap_or(true),
            carry: None,
            history: history::History::default(),
            editing: false,
            pending_shape: None,
            keep_decals: false,
            preview: None,
            asset_selected: None,
            thumb_size: 72.0,
            cams: [(SHAPE_VIEW.0, SHAPE_VIEW.1, 1.0), (initial.yaw, initial.pitch, 1.0), (initial.yaw, initial.pitch, 1.0)],
            cam_owner: [Workspace::Shape, Workspace::Design, Workspace::Output][initial.workspace.min(2)],
            debug: initial.debug,
            decals,
            selected: None,
            extra: Vec::new(),
            hover_decal: None,
            hover_now: None,
            grab: None,
            hover: None,
            sheet,
            bases: [None, None],
            custom: [None, None],
            file: file::FileState::new(initial.answer_open.clone(), initial.answer_save.clone()),
            replace_request: None,
            answer_replace: initial.answer_replace.clone(),
            answer_export: initial.answer_export.clone(),
            // 自動のテストでは、使う人のフォルダに控えを書かない。
            recovery: if cfg!(test) { recovery::Recovery::off() } else { recovery::Recovery::start(initial.recovery_dir.clone(), initial.backup_seconds) },
            settings: if cfg!(test) { settings::Settings::off() } else { settings::Settings::load(initial.settings.clone()) },
            library: library::Library::new(initial.library, initial.answer_folder.clone()),
            renaming: None,
            layer_drag: None,
            fonts: Default::default(),
            text_style: typing::default_style(),
            focus_text: false,
            show_guides: initial.guides,
            art_dirty: true,
            front_rgba: Vec::new(),
            front_tex: None,
            front_tex_dirty: true,
            back_rgba: Vec::new(),
            back_tex: None,
            back_tex_dirty: true,
            back_dirty: true,
            back_has_decals: false,
            fold: initial.fold.clamp(0.0, 1.0),
            animate_to: None,
            view_to: None,
            view_before_flat: None,
            open: [initial.open.clamp(0.0, 1.0), initial.open_bottom.clamp(0.0, 1.0)],
            open_to: [None; 2],
            yaw: initial.yaw,
            pitch: initial.pitch,
            zoom: 1.0,
            renderer: None,
            psd_path: String::new(),
            psd_info: String::new(),
            message: None,
            triangles: 0,
            backend: String::new(),
        };
        if let Some(path) = &initial.psd {
            app.psd_path = path.display().to_string();
            app.open_psd(path);
        }
        if let Some(path) = &initial.png {
            app.add_image(path);
        }
        for &index in &initial.back {
            if let Some(decal) = app.decals.get_mut(index) {
                decal.back = true;
            }
        }
        app.selected = initial.select.filter(|&i| i < app.decals.len());
        // 起動したときの状態は、保存してある状態として扱う（何もしていないのに「未保存」と出さない）。
        app.mark_saved();
        if let Some(path) = &initial.project {
            app.open_path(path);
        }
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
        self.back_dirty = true;
        if shape_changed {
            // 型を変えたら、パネルの並びが変わるので、置いた柄は外す（元に戻すで型を戻すときは、外さない）。
            // 寸法だけを変えたときは、柄はパネルの中での位置で持っているので、そのまま付いてくる。
            if !std::mem::take(&mut self.keep_decals) {
                self.decals.clear();
                self.select_only(None);
            }
            self.grab = None;
            self.preview = None;
            self.flat = [FlatCam::default(); 2];
            // 型の工程は、型を変えるたびに、決まった斜めの向きから見る。
            self.cams[0] = (SHAPE_VIEW.0, SHAPE_VIEW.1, 1.0);
            if self.cam_owner == Workspace::Shape {
                (self.yaw, self.pitch, self.zoom) = self.cams[0];
                self.view_to = None;
                self.view_before_flat = None;
            }
        }
        // 読み込んだ絵は、読み直さずに、展開図の新しい大きさへ合わせ直すだけにする。
        self.refit_bases();
    }

    /// いまの、柄の状態。
    fn snapshot(&self) -> Snapshot {
        Snapshot { decals: self.decals.clone(), selected: self.selected, extra: self.extra.clone(), dims: self.dims, library: self.library(), bases: self.bases.clone() }
    }

    /// これから柄を変える。変える前の状態を、元に戻せるように覚える。
    fn checkpoint(&mut self) {
        self.history.record(self.snapshot());
    }

    fn restore(&mut self, state: Snapshot) {
        if state.dims.sides != self.dims.sides {
            // 型を変える前へ戻す。型紙を作り直してから、柄を戻す。
            self.dims = state.dims;
            self.keep_decals = true;
            self.rebuild_if_needed();
        }
        self.restore_library(state.library);
        if !(base::same_base(&state.bases[0], &self.bases[0]) && base::same_base(&state.bases[1], &self.bases[1])) {
            self.psd_info.clear();
            self.bases = state.bases;
            self.refit_bases();
        }
        self.decals = state.decals;
        self.selected = state.selected.filter(|&i| i < self.decals.len());
        self.extra = state.extra.into_iter().filter(|&i| i < self.decals.len()).collect();
        self.hover_decal = None;
        self.grab = None;
        (self.renaming, self.layer_drag) = (None, None);
        self.art_dirty = true;
        self.back_dirty = true;
    }

    /// 型を変える。置いた柄は外れるが、元に戻せる。
    fn set_shape(&mut self, sides: usize) {
        if sides == self.dims.sides {
            return;
        }
        self.checkpoint();
        let removed = self.decals.len();
        self.dims = Dims::for_shape(sides, self.dims.t);
        self.fold = 1.0;
        self.open = [0.0; 2];
        (self.animate_to, self.open_to) = (None, [None; 2]);
        if removed > 0 {
            self.say(format!("型を変えたので、置いた柄 {removed} つを外しました。Ctrl+Z で、前の型と柄に戻せます。"));
        }
    }

    fn undo(&mut self) {
        let current = self.snapshot();
        if let Some(state) = self.history.undo(|| current) {
            self.restore(state);
        }
    }

    fn redo(&mut self) {
        let current = self.snapshot();
        if let Some(state) = self.history.redo(|| current) {
            self.restore(state);
        }
    }

    /// 選んでいる柄を消す（いくつ選んでいても）。確認は出さない（元に戻せる）。ロックした柄は残す。
    fn delete_selected(&mut self) {
        let chosen = self.selection();
        if chosen.is_empty() {
            return;
        }
        let mut gone: Vec<usize> = chosen.iter().copied().filter(|&i| !self.decals[i].layer.locked).collect();
        if gone.is_empty() {
            self.say("ロックしている柄は消せません。レイヤーの欄で、鍵を外してください。");
            return;
        }
        let kept = chosen.len() - gone.len();
        self.checkpoint();
        gone.sort_unstable();
        for &index in gone.iter().rev() {
            self.decals.remove(index);
        }
        self.select_only(None);
        self.grab = None;
        self.hover_decal = None;
        let what = if gone.len() == 1 { "柄を消しました".to_owned() } else { format!("柄を {} 個消しました", gone.len()) };
        let locked = if kept > 0 { format!("（ロックしている {kept} 個は残しました）") } else { String::new() };
        self.say(format!("{what}{locked}。Ctrl+Z で戻せます。"));
    }

    /// 柄を 1 つ足して、選ぶ。正面（なければ最初のパネル）の中央に置く。
    fn add_decal(&mut self, picture: Picture, size: P2) {
        self.checkpoint();
        let panel = self.template.panels.iter().position(|p| p.name == "正面").unwrap_or(0);
        self.decals.push(Placed { picture, decal: Decal { panel, at: [0.5, 0.5], size, angle: 0.0 }, back: false, layer: Default::default() });
        self.select_only(Some(self.decals.len() - 1));
    }

    /// 画像（PNG）を、この案件の素材に取り込んで、柄として置く。元のファイルの中身も、そのまま持つ（案件のファイルに入れるため）。
    fn add_image(&mut self, path: &Path) {
        let known = self.assets.len();
        let Some(index) = self.import_image(path, false) else { return };
        let (picture, size) = (self.assets[index].picture.clone(), self.assets[index].size);
        if index < known {
            self.say(format!("{} は、すでに素材の欄にあります。柄として置きました。", self.assets[index].name));
        } else {
            self.say(format!("{} を柄として置きました。素材の欄にも入れてあります。", path.display()));
        }
        self.add_decal(picture, size);
    }

    /// 画像（PNG、JPEG、PSD）を、この案件の素材に取り込む（置かない）。素材の欄での番号を返す。
    /// 同じ中身の素材がすでにあれば、増やさずに、その番号を返す。`checkpoint` が真なら、取り込みを元に戻せるようにする。
    fn import_image(&mut self, path: &Path, checkpoint: bool) -> Option<usize> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.say(format!("{} を開けませんでした: {e}", path.display()));
                return None;
            }
        };
        let picture = match decode_image(&bytes) {
            Ok(picture) => picture,
            Err(e) => {
                self.say(e);
                return None;
            }
        };
        let Picture::Image { w, h, .. } = &picture else { return None };
        let size = [50.0, 50.0 * *h as f32 / *w as f32];
        let name = path.file_name().map_or_else(|| "画像".to_owned(), |n| n.to_string_lossy().into_owned());
        let stored = project::Asset::new(name.clone(), bytes, size, file::now_local());
        // どこから取り込んだ画像でも、最近使ったものに入れる（次の案件でも、すぐ使えるように）。
        self.settings.used_asset(path);
        if let Err(e) = self.settings.save() {
            eprintln!("{e}");
        }
        if let Some(index) = self.assets.iter().position(|asset| asset.stored.as_ref().is_some_and(|s| s.id() == stored.id())) {
            if checkpoint {
                self.say(format!("{name} は、すでにこの案件の素材にあります。"));
            }
            return Some(index);
        }
        if checkpoint {
            self.checkpoint();
        }
        self.assets.push(assets::Asset { name, picture, size, thumb: None, bundled: false, stored: Some(stored) });
        Some(self.assets.len() - 1)
    }

    /// 表か裏の柄を 1 つずつ、展開図の大きさの透明な画像に描く（PSD のレイヤー用）。名前は、付けた名前か「番号（種類）」。
    /// 向きは、どちらも表の展開図の向き（裏の型紙では、書くときに左右を裏返す）。隠した柄は、隠したレイヤーにする。
    fn decal_layers(&self, back: bool) -> Vec<crate::art::DecalLayer> {
        self.decals
            .iter()
            .enumerate()
            .filter(|(_, decal)| decal.back == back)
            .map(|(index, decal)| {
                let mut image = vec![0u8; self.sheet.w * self.sheet.h * 4];
                let mut shown = decal.clone();
                shown.layer.hidden = false;
                paint(&self.surface, &self.template, std::slice::from_ref(&shown), Marks::default(), back, Canvas { w: self.sheet.w, h: self.sheet.h, rgba: &mut image });
                let name = match &decal.layer.name {
                    Some(name) => format!("{}（{name}）", index + 1),
                    None => format!("{}（{}）", index + 1, decal.picture.label()),
                };
                crate::art::DecalLayer { name, image, visible: !decal.layer.hidden }
            })
            .collect()
    }

    /// レイヤーの欄などに出す、柄の名前。付けた名前がなければ「種類 番号」。
    fn layer_label(&self, index: usize) -> String {
        let decal = &self.decals[index];
        if let Some(name) = &decal.layer.name {
            return name.clone();
        }
        // 文字の柄は、文字の 1 行目を名前にする（Photoshop の文字のレイヤーと同じ）。
        if let Picture::Text(art) = &decal.picture {
            let first: String = art.spec.text.lines().next().unwrap_or("").chars().take(20).collect();
            if !first.trim().is_empty() {
                return first;
            }
        }
        format!("{} {}", decal.picture.label(), index + 1)
    }

    /// 下の帯に、知らせを 1 行出す。
    fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), None));
    }

    /// いまの型の名前と寸法（例: 「差し込み箱　120 × 60 × 160 mm」）。
    fn shape_summary(&self) -> String {
        let d = self.dims;
        let name = SHAPES.iter().find(|(sides, _)| *sides == d.sides).map_or("型", |(_, name)| name);
        match d.sides {
            PILLOW => format!("{name}　{:.0} × {:.0} mm", d.w, d.h),
            n if n >= 3 => format!("{name}　一辺 {:.0} × 高さ {:.0} mm", d.w, d.h),
            _ => format!("{name}　{:.0} × {:.0} × {:.0} mm", d.w, d.d, d.h),
        }
    }

    /// 同じ面（表か裏）で、すぐ上（+1）か、すぐ下（-1）に重なっている柄。
    fn neighbor(&self, index: usize, step: isize) -> Option<usize> {
        let back = self.decals.get(index)?.back;
        let same = |i: &usize| self.decals[*i].back == back;
        if step > 0 { (index + 1..self.decals.len()).find(same) } else { (0..index).rev().find(same) }
    }

    /// 選んでいる柄を、同じ面の中で、重なりの上（+1）か下（-1）へ 1 つ動かす。
    fn reorder_selected(&mut self, step: isize) {
        let Some(index) = self.selected else { return };
        let Some(other) = self.neighbor(index, step) else { return };
        self.checkpoint();
        self.decals.swap(index, other);
        self.select_only(Some(other));
        self.grab = None;
        self.art_dirty = true;
    }

    /// 組み立てる（1）か、展開図に戻す（0）。
    fn fold_to(&mut self, target: f32) {
        self.animate_to = Some(target);
        if target > 0.5 {
            // 展開図に戻す前の視点があれば、そこへ戻す。
            self.view_to = self.view_before_flat.take();
        } else {
            // 展開図は、印刷面が手前を向く正面から見る（裏の白い面が見えてしまわないように）。
            if self.view_before_flat.is_none() {
                self.view_before_flat = Some((self.yaw, self.pitch));
            }
            self.view_to = Some((0.0, 0.0));
        }
    }

    /// 柄や絵が変わっていたら、箱に貼る画像と、展開図の表示の画像を作り直す。
    fn refresh_art(&mut self, ctx: &egui::Context, rs: Option<&egui_wgpu::RenderState>, shown: Shown) {
        // 素材を運んでいる間は、放したら置かれる柄を、仮に足して描く。
        let with_preview: Vec<Placed>;
        let chosen: Vec<usize>;
        let (decals, marks): (&[Placed], Marks) = match &self.preview {
            Some(preview) => {
                with_preview = self.decals.iter().cloned().chain([preview.clone()]).collect();
                chosen = vec![with_preview.len() - 1];
                (&with_preview, Marks { selected: &chosen, hover: None })
            }
            None => {
                chosen = self.selection();
                (&self.decals, Marks { selected: &chosen, hover: self.hover_decal })
            }
        };
        if self.art_dirty {
            let (w, h) = (self.sheet.w, self.sheet.h);
            self.front_rgba = compose(&self.sheet, self.custom[0].as_deref(), self.show_guides, |art| paint(&self.surface, &self.template, decals, marks, false, Canvas { w, h, rgba: art }));
            if let Some(rs) = rs {
                self.renderer.get_or_insert_with(|| BoxRenderer::new(rs)).set_art(rs, false, w, h, &self.front_rgba);
            }
            self.art_dirty = false;
            self.front_tex_dirty = true;
            // 裏は、裏に柄があるとき（と、なくなった直後）だけ作り直す。表の柄を動かす間の負荷を増やさない。
            let has_back = decals.iter().any(|decal| decal.back);
            if has_back || self.back_has_decals {
                self.back_dirty = true;
            }
        }
        if self.back_dirty {
            let (w, h) = (self.sheet.w, self.sheet.h);
            self.back_rgba = compose_back(&self.sheet, self.custom[1].as_deref(), self.show_guides, |art| paint(&self.surface, &self.template, decals, marks, true, Canvas { w, h, rgba: art }));
            if let Some(rs) = rs {
                self.renderer.get_or_insert_with(|| BoxRenderer::new(rs)).set_art(rs, true, w, h, &self.back_rgba);
            }
            self.back_has_decals = decals.iter().any(|decal| decal.back);
            self.back_dirty = false;
            self.back_tex_dirty = true;
        }
        // 展開図の表示の画像は、その表示を出しているときだけ作る（柄を動かす間の負荷を抑える）。
        if shown.front && self.front_tex_dirty {
            self.front_tex = Some(views::texture(ctx, "sheet-front", self.sheet.w, self.sheet.h, &self.front_rgba));
            self.front_tex_dirty = false;
        }
        if shown.back && self.back_tex_dirty {
            self.back_tex = Some(views::texture(ctx, "sheet-back", self.sheet.w, self.sheet.h, &self.back_rgba));
            self.back_tex_dirty = false;
        }
    }

    /// 工程を移ったら、視点を、その工程のものに切り替える。
    fn switch_camera(&mut self) {
        if self.cam_owner == self.workspace {
            return;
        }
        let slot = |w: Workspace| [Workspace::Shape, Workspace::Design, Workspace::Output].iter().position(|x| *x == w).unwrap_or(0);
        self.cams[slot(self.cam_owner)] = (self.yaw, self.pitch, self.zoom);
        // 型の工程は、いつも決まった斜めの向きから見る（前の工程の向きに引きずられると、上下が分かりにくい）。
        self.cams[0] = (SHAPE_VIEW.0, SHAPE_VIEW.1, 1.0);
        (self.yaw, self.pitch, self.zoom) = self.cams[slot(self.workspace)];
        self.cam_owner = self.workspace;
        self.view_to = None;
        self.view_before_flat = None;
    }

    /// キーの割り当て。文字を打っている間は効かせない。
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (plain, command, shift, pressed) = ctx.input(|i| {
            let pressed = [egui::Key::V, egui::Key::T, egui::Key::H, egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num0, egui::Key::Z, egui::Key::Y, egui::Key::Delete, egui::Key::Backspace, egui::Key::A, egui::Key::Escape].map(|key| i.key_pressed(key));
            (i.modifiers.is_none(), i.modifiers.command, i.modifiers.shift, pressed)
        });
        let [v, t, h, one, two, three, zero, z, y, delete, backspace, a, escape] = pressed;
        if command && zero {
            // Ctrl+0: 全体を表示。
            self.zoom = 1.0;
            self.flat = [FlatCam::default(); 2];
        }
        // Ctrl+Z で元に戻す。Ctrl+Shift+Z か Ctrl+Y で、やり直す。
        if command && ((z && shift) || y) {
            self.redo();
        } else if command && z {
            self.undo();
        }
        if plain && (delete || backspace) && self.workspace == Workspace::Design {
            self.delete_selected();
        }
        // Ctrl+A ですべての柄を選び、Esc で選ぶのをやめる。
        if command && a && self.workspace == Workspace::Design {
            self.select_all();
        }
        if plain && escape && self.workspace == Workspace::Design && self.carry.is_none() {
            self.select_only(None);
        }
        if !plain || self.workspace != Workspace::Design {
            return;
        }
        if v {
            self.tool = Tool::Select;
        }
        if h {
            self.tool = Tool::Hand;
        }
        if t {
            self.tool = Tool::Text;
        }
        for (pressed, view) in [(one, View::Solid), (two, View::Front), (three, View::Back)] {
            if pressed {
                self.shown.solo(view);
            }
        }
    }
}

impl eframe::App for App {
    /// ふつうに終わるとき。落ちたときの控えを消す。
    fn on_exit(&mut self) {
        self.recovery.finish();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // お客さんが開け閉めするアニメーション。約 0.6 秒（フタが開いてから、内側のフラップが開く）。
        for which in 0..2 {
            let Some(target) = self.open_to[which] else { continue };
            let step = ui.input(|i| i.stable_dt).min(0.05) / 0.6;
            if (target - self.open[which]).abs() <= step {
                self.open[which] = target;
                self.open_to[which] = None;
            } else {
                self.open[which] += step * (target - self.open[which]).signum();
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
                let step = (ui.input(|i| i.stable_dt).min(0.05) * 12.0).min(1.0);
                self.yaw += turn * step;
                self.pitch += rise * step;
                ui.ctx().request_repaint();
            }
        }
        // 組み立てのアニメーション。全体で約 1 秒。
        if let Some(target) = self.animate_to {
            let step = ui.input(|i| i.stable_dt).min(0.05) / 1.0;
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
            // 案件のファイルは開く。画像（PNG、JPEG、PSD）は柄として置く。
            // ただし、パケノバが書き出した型紙の PSD（ガイドのレイヤーがある）は、展開図まるごとの絵として読む。
            let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
            let dieline = ext.as_deref() == Some("psd") && std::fs::read(path).is_ok_and(|bytes| crate::art::psd_is_dieline(&bytes));
            if ext.as_deref() == Some(project::EXTENSION) {
                self.request_open(Some(path.clone()));
            } else if dieline || !is_image(path) {
                self.psd_path = path.display().to_string();
                self.open_psd(path);
            } else if self.library.open {
                // 「素材」の画面では、箱がないので、取り込むだけ。
                if self.import_image(path, true).is_some() {
                    self.say(format!("{} を、この案件の素材に取り込みました。", path.display()));
                }
            } else {
                self.add_image(path);
            }
        }

        // 数値の欄から手を離したら、ひと続きの変更は終わり。
        if !ui.input(|i| i.pointer.any_down()) && !ui.ctx().egui_wants_keyboard_input() {
            self.editing = false;
        }
        self.file_frame(ui.ctx());
        self.recovery_frame(ui.ctx());
        self.file_shortcuts(ui.ctx());
        self.shortcuts(ui.ctx());
        self.top_bar(ui);
        self.run_file_action(ui.ctx());
        self.run_replace();
        self.switch_camera();
        self.status_bar(ui);
        self.confirm_shape(ui.ctx());
        self.file_dialogs(ui.ctx());
        self.recovery_dialog(ui.ctx());
        if self.library.open {
            self.library_screen(ui);
            self.rebuild_if_needed();
            self.settle_hover(ui.ctx());
            return;
        }
        match self.workspace {
            Workspace::Shape => {
                self.shape_list(ui);
                self.shape_settings(ui);
            }
            Workspace::Design => {
                self.options_bar(ui);
                // 素材の欄は、道具の帯の箱側（ア）か、外側（イ）。どちらがよいかは、触って決める。
                if self.assets_inside {
                    self.tool_strip(ui);
                    self.assets_panel(ui);
                } else {
                    self.assets_panel(ui);
                    self.tool_strip(ui);
                }
                self.design_side(ui);
            }
            Workspace::Output => {
                self.output_list(ui);
                self.output_settings(ui);
            }
        }
        self.rebuild_if_needed();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.views(ui, frame));
        self.settle_hover(ui.ctx());
    }
}
