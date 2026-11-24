//! 案件のファイル（`.pknb`）の読み書き。
//!
//! 案件 1 つは、1 つのファイル。中身は ZIP で、次のものが入る。
//!
//! - `project.toml`: 設定のテキスト。型と寸法、素材の一覧、置いた柄、展開図まるごとの絵。
//! - `assets/<中身の印>.<拡張子>`: 素材と、展開図まるごとの絵のファイル。取り込んだ元のファイルのまま。
//! - `masks/<中身の印>.gray`: 柄のレイヤーマスク（1 画素 1 バイトの白黒。幅と高さは設定のテキストに書く）。
//!
//! 決まり（くわしくは `docs/project-file.md`）。
//!
//! - 外のファイルとのつながりは持たない。案件のファイルだけで、同じ絵が出る。
//! - 素材のファイルは、中身の印（SHA-256）を名前にする。同じ中身は 1 回しか入らない。
//! - 柄の置き場所のパネルは、番号ではなく名前で書く。同じ名前のパネルがある型では、何番目かも書く。
//! - 保存は、別の名前で書ききってから、元のファイルと入れ替える。途中で止まっても、前のファイルが残る。
//! - 同じ案件は、いつ保存しても同じバイト列になる（日時を書き込まない）。
//! - 自分の案件かどうかは、拡張子ではなく、`project.toml` の `format` で判断する。
//!
//! 読むときは、できるだけ開く。素材が欠けている、パネルが見つからない、といった部分的な傷は、
//! その柄や素材だけを外して開き、何を外したかを `Opened::problems` で返す。

use pakenoba_i18n::t;
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::material::{Coat, DEFAULT_EMBOSS_EDGE, DEFAULT_EMBOSS_HEIGHT, DEFAULT_WAVE, EMBOSS_EDGE, EMBOSS_HEIGHT, Finish, FinishKind, Foil, Material, Paper};
use crate::surface::Decal;
use crate::template::{P2, Template, TuckFlaps};

/// 案件のファイルの拡張子（点は含まない）。
pub const EXTENSION: &str = "pknb";
/// 設定のテキストの `format` に書く印。
pub const FORMAT: &str = "pakenoba-project";
/// いま書く形式の版。
pub const VERSION: u32 = 1;

const MANIFEST: &str = "project.toml";
/// 設定のテキストの大きさの上限。これより大きいものは、壊れているとみなす。
const MANIFEST_LIMIT: u64 = 16 * 1024 * 1024;

/// 寸法の範囲（mm。ふくらみだけは度）。範囲の外の値は、読むときに範囲の中へ寄せる。
pub const WIDTH: RangeInclusive<f32> = 40.0..=300.0;
pub const DEPTH: RangeInclusive<f32> = 20.0..=200.0;
pub const HEIGHT: RangeInclusive<f32> = 40.0..=300.0;
pub const THICKNESS: RangeInclusive<f32> = 0.3..=6.0;
pub const BULGE: RangeInclusive<f32> = 15.0..=90.0;
pub const ROOF: RangeInclusive<f32> = 10.0..=150.0;
/// 角柱の角の数の範囲。
pub const SIDES: RangeInclusive<usize> = 3..=12;

/// 型の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 差し込み箱。
    TuckBox,
    /// ゲーブル型。
    Gable,
    /// ピロー型。
    Pillow,
    /// 正多角柱。値は角の数。
    Prism(usize),
}

/// 型と寸法。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    /// 差し込み箱とゲーブル型では幅、角柱では一辺の長さ、ピロー型では横の長さ。
    pub w: f32,
    /// 奥行き（差し込み箱とゲーブル型だけが使う）。
    pub d: f32,
    /// 差し込み箱と角柱では高さ、ゲーブル型では胴の高さ、ピロー型では 1 枚の面の高さ。
    pub h: f32,
    /// 紙の厚み。
    pub t: f32,
    /// ピロー型のふくらみ（度）。
    pub bulge: f32,
    /// ゲーブル型の屋根の高さ。
    pub roof: f32,
    /// 差し込み箱のフラップの長さ（決めていないものは自動）。
    pub flaps: TuckFlaps,
}

impl Shape {
    /// この型と寸法の型紙。
    pub fn template(&self) -> Template {
        match self.kind {
            Kind::TuckBox => Template::tuck_box_with(self.w, self.d, self.h, self.t, self.flaps),
            Kind::Gable => Template::gable(self.w, self.d, self.h, self.roof, self.t),
            Kind::Pillow => Template::pillow(self.w, self.h, self.t, self.bulge.to_radians()),
            Kind::Prism(n) => Template::prism(n, self.w, self.h, self.t),
        }
    }

    fn values(&mut self) -> [(&'static str, &mut f32, RangeInclusive<f32>); 6] {
        [(t!("Width"), &mut self.w, WIDTH), (t!("Depth"), &mut self.d, DEPTH), (t!("Height"), &mut self.h, HEIGHT), (t!("Paper thickness"), &mut self.t, THICKNESS), (t!("Bulge"), &mut self.bulge, BULGE), (t!("Roof height"), &mut self.roof, ROOF)]
    }
}

/// 素材 1 つ。取り込んだ元のファイルを、そのまま持つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    /// 元のファイルの名前（場所は含まない）。
    pub name: String,
    /// 取り込んだ日時。画面に出すための文字で、中核は中身を解釈しない。
    pub imported: String,
    /// 置くときの大きさ（mm）。
    pub size: P2,
    id: String,
    bytes: Arc<Vec<u8>>,
}

impl Asset {
    /// 元のファイルの名前と中身から、素材を作る。
    pub fn new(name: impl Into<String>, bytes: Vec<u8>, size: P2, imported: impl Into<String>) -> Asset {
        let id = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        Asset { name: name.into(), imported: imported.into(), size, id, bytes: Arc::new(bytes) }
    }

    /// 中身の印（SHA-256 を 16 進で書いたもの）。中身が同じ素材は、同じ印になる。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 元のファイルの中身。
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    /// 案件のファイルの中での名前。
    fn file(&self) -> String {
        match extension(&self.name) {
            Some(ext) => format!("assets/{}.{ext}", self.id),
            None => format!("assets/{}", self.id),
        }
    }
}

/// 名前の拡張子を、小文字にして返す。英数字 1〜8 文字でなければ、なしとする。
fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    let fits = (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    fits.then(|| ext.to_ascii_lowercase())
}

/// 素材のファイルの名前として正しい形か（`assets/` と、16 進 64 文字と、あれば拡張子）。
fn is_asset_file(file: &str) -> bool {
    let Some(rest) = file.strip_prefix("assets/") else { return false };
    let (id, ext) = rest.split_once('.').map_or((rest, None), |(id, ext)| (id, Some(ext)));
    let hex = id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    hex && ext.is_none_or(|ext| (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// 柄の絵が、どこから来たか。
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// 付属の見本。値は見本の名前（どんな見本があるかは、アプリの側が決める）。
    Builtin(String),
    /// この案件の素材。値は `Project::assets` の中での番号。
    Asset(usize),
    /// 文字。絵は、開くときにアプリが文字から描き直す。
    Text(TextSpec),
    /// 図形（四角、丸）。絵は持たず、形と色から描く。
    Shape(ShapeSpec),
    /// グループの見出し。中身は、並びの中で、この見出しのすぐ前に続く、見出しより深い（`Layer::depth` が大きい）レイヤー。
    /// `open` はレイヤーの欄で開いているか。
    /// 場所と大きさは使わない。
    Group { open: bool },
    /// 調整レイヤー（`Layer::adjust` を、下の絵すべてに効かせる）。場所と大きさは使わない。
    Adjustment,
}

/// 図形の形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    /// 四角（角を丸められる）。
    #[default]
    Rect,
    /// 丸（楕円）。
    Ellipse,
}

/// 図形の柄。大きさは柄の大きさ（`Decal::size`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeSpec {
    pub kind: ShapeKind,
    /// 塗りの色（None なら塗らない）。
    pub fill: Option<[u8; 3]>,
    /// 線の色（None なら線を引かない）。
    pub stroke: Option<[u8; 3]>,
    /// 線の太さ（mm）。
    pub stroke_width: f32,
    /// 四角の角の丸み（半径、mm）。
    pub radius: f32,
}

/// 図形の線の太さと、角の丸みの範囲（mm）。
pub const SHAPE_STROKE: RangeInclusive<f32> = 0.1..=50.0;
pub const SHAPE_RADIUS: RangeInclusive<f32> = 0.0..=500.0;

/// 文字の柄の中身と書式。
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    /// 文字（改行で行を分ける）。
    pub text: String,
    /// フォントの名前（ファミリー名）。開いた PC にないときは、アプリが別のフォントで描く。
    pub font: String,
    /// 文字の大きさ（mm。全角 1 文字の高さ）。
    pub size: f32,
    /// 色（sRGB）。
    pub color: [u8; 3],
    pub align: TextAlign,
    /// 字間（全角の 1/1000 単位。0 がふつう）。
    pub spacing: f32,
    /// 行送り（文字の大きさの何倍か）。
    pub leading: f32,
}

/// 文字の揃え。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    fn name(self) -> &'static str {
        match self {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        }
    }

    fn from_name(name: &str) -> Option<TextAlign> {
        Some(match name {
            "left" => TextAlign::Left,
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => return None,
        })
    }
}

/// 文字の範囲（読むときに、範囲の外は寄せる）。
pub const TEXT_SIZE: RangeInclusive<f32> = 0.5..=500.0;
pub const TEXT_SPACING: RangeInclusive<f32> = -500.0..=2000.0;
pub const TEXT_LEADING: RangeInclusive<f32> = 0.5..=5.0;

/// 置いた柄 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// 場所と大きさ。`panel` は、`Project::shape` の型紙でのパネルの番号。
    pub decal: Decal,
    /// 紙の裏（箱の内側の面）に置いた柄か。
    pub back: bool,
    pub source: Source,
    /// レイヤーとしての名前、表示、ロック。
    pub layer: Layer,
}

/// 柄のレイヤーとしての設定。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layer {
    /// レイヤーごとの決まった印（並べ替えても変わらない。コマンドで指すため）。0 はまだ付けていない（アプリが付ける）。
    pub id: u32,
    /// 柄だけに効く調整（下から順に効かせる）。調整レイヤーでは、下の絵すべてに効く調整。
    pub adjust: Vec<Adjustment>,
    /// レイヤーマスク。
    pub mask: Option<Mask>,
    /// グループの入れ子の深さ（0 はどのグループにも入っていない）。グループの中身は、並びの中で、見出しのすぐ前に、見出しより深くして続けて置く。
    pub depth: u8,
    /// すぐ下のレイヤーでクリップしている（クリッピングマスク。下のレイヤーの見えている所にだけ出る）。
    pub clipped: bool,
    /// 使う人が付けた名前。None なら、画面では「番号（種類）」と出す。
    pub name: Option<String>,
    /// 隠している（箱にも、書き出す型紙にも描かない。型紙の PSD では、隠したレイヤーとして入れる）。
    pub hidden: bool,
    /// すべてをロックしている（つかめない、動かせない、設定も絵も変えられない、消せない）。
    pub locked: bool,
    /// 画像をロックしている（調整とマスクを変えられない。動かすことはできる）。
    pub lock_pixels: bool,
    /// 位置をロックしている（動かせない、大きさや向きも変えられない。調整やマスクは変えられる）。
    pub lock_position: bool,
    /// 不透明度（0〜100%）。
    pub opacity: u8,
    /// 塗り（0〜100%。Photoshop の「塗り」。レイヤーの中身だけに効く。いまはレイヤースタイルがないので、不透明度と同じ効き方。グループにはない）。
    pub fill: u8,
    /// 描画モード（下の絵との重ね方）。
    pub blend: Blend,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { id: 0, adjust: Vec::new(), mask: None, depth: 0, clipped: false, name: None, hidden: false, locked: false, lock_pixels: false, lock_position: false, opacity: 100, fill: 100, blend: Blend::Normal }
    }
}

impl Layer {
    /// 動かせない（すべてか、位置をロック）。
    pub fn fixed(&self) -> bool {
        self.locked || self.lock_position
    }

    /// 絵（調整とマスク）を変えられない（すべてか、画像をロック）。
    pub fn sealed(&self) -> bool {
        self.locked || self.lock_pixels
    }
}

/// 描画モード。Photoshop のレイヤーの描画モードと同じ並びと計算（ディザ合成は除く）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// 通過（グループだけ。中身を、そのまま外へ重ねる）。
    PassThrough,
}

impl Blend {
    /// 画面の並び（Photoshop と同じく、似たものごとのまとまり。`None` は区切り線）。
    pub const MENU: [Option<Blend>; 31] = [
        Some(Blend::Normal),
        None,
        Some(Blend::Darken),
        Some(Blend::Multiply),
        Some(Blend::ColorBurn),
        Some(Blend::LinearBurn),
        Some(Blend::DarkerColor),
        None,
        Some(Blend::Lighten),
        Some(Blend::Screen),
        Some(Blend::ColorDodge),
        Some(Blend::LinearDodge),
        Some(Blend::LighterColor),
        None,
        Some(Blend::Overlay),
        Some(Blend::SoftLight),
        Some(Blend::HardLight),
        Some(Blend::VividLight),
        Some(Blend::LinearLight),
        Some(Blend::PinLight),
        Some(Blend::HardMix),
        None,
        Some(Blend::Difference),
        Some(Blend::Exclusion),
        Some(Blend::Subtract),
        Some(Blend::Divide),
        None,
        Some(Blend::Hue),
        Some(Blend::Saturation),
        Some(Blend::Color),
        Some(Blend::Luminosity),
    ];

    /// 案件のファイルに書く名前。
    pub fn key(self) -> &'static str {
        match self {
            Blend::Normal => "normal",
            Blend::Darken => "darken",
            Blend::Multiply => "multiply",
            Blend::ColorBurn => "color_burn",
            Blend::LinearBurn => "linear_burn",
            Blend::DarkerColor => "darker_color",
            Blend::Lighten => "lighten",
            Blend::Screen => "screen",
            Blend::ColorDodge => "color_dodge",
            Blend::LinearDodge => "linear_dodge",
            Blend::LighterColor => "lighter_color",
            Blend::Overlay => "overlay",
            Blend::SoftLight => "soft_light",
            Blend::HardLight => "hard_light",
            Blend::VividLight => "vivid_light",
            Blend::LinearLight => "linear_light",
            Blend::PinLight => "pin_light",
            Blend::HardMix => "hard_mix",
            Blend::Difference => "difference",
            Blend::Exclusion => "exclusion",
            Blend::Subtract => "subtract",
            Blend::Divide => "divide",
            Blend::Hue => "hue",
            Blend::Saturation => "saturation",
            Blend::Color => "color",
            Blend::Luminosity => "luminosity",
            Blend::PassThrough => "pass_through",
        }
    }

    pub fn from_key(key: &str) -> Option<Blend> {
        Blend::MENU.iter().flatten().copied().chain([Blend::PassThrough]).find(|b| b.key() == key)
    }

    /// 画面に出す名前（英語。訳は画面の側で）。
    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Darken => "Darken",
            Blend::Multiply => "Multiply",
            Blend::ColorBurn => "Color Burn",
            Blend::LinearBurn => "Linear Burn",
            Blend::DarkerColor => "Darker Color",
            Blend::Lighten => "Lighten",
            Blend::Screen => "Screen",
            Blend::ColorDodge => "Color Dodge",
            Blend::LinearDodge => "Linear Dodge (Add)",
            Blend::LighterColor => "Lighter Color",
            Blend::Overlay => "Overlay",
            Blend::SoftLight => "Soft Light",
            Blend::HardLight => "Hard Light",
            Blend::VividLight => "Vivid Light",
            Blend::LinearLight => "Linear Light",
            Blend::PinLight => "Pin Light",
            Blend::HardMix => "Hard Mix",
            Blend::Difference => "Difference",
            Blend::Exclusion => "Exclusion",
            Blend::Subtract => "Subtract",
            Blend::Divide => "Divide",
            Blend::Hue => "Hue",
            Blend::Saturation => "Saturation",
            Blend::Color => "Color",
            Blend::Luminosity => "Luminosity",
            Blend::PassThrough => "Pass Through",
        }
    }
}

/// 調整 1 つ。`on` が偽なら、効かせない（目のアイコンで切り替える）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Adjustment {
    pub kind: AdjustKind,
    pub on: bool,
}

impl Adjustment {
    pub fn new(kind: AdjustKind) -> Adjustment {
        Adjustment { kind, on: true }
    }
}

/// 調整の種類と、その数値。数値は、保存しても同じに戻るよう整数で持つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdjustKind {
    /// 明るさ（-150〜150）とコントラスト（-50〜100）。Photoshop の「明るさ・コントラスト」と同じ範囲。
    BrightnessContrast { brightness: i16, contrast: i16 },
    /// 色相（-180〜180 度）、彩度（-100〜100）、明度（-100〜100）。
    HueSaturation { hue: i16, saturation: i16, lightness: i16 },
    /// レベル補正。黒と白の点（0〜255）と、中間の明るさ（ガンマの 100 倍。10〜999）。
    Levels { black: u8, white: u8, gamma: u16 },
    /// 階調の反転。
    Invert,
    /// 縁の色と、それにつながる近い色を透明にする。`tolerance` は近さ（1〜200）。
    ClearBackground { tolerance: u8 },
    /// 写真の、ムラのある明るさや影をならす。
    FlattenShading,
    /// 近い色をまとめて、色の数を絞る（2〜64）。
    ReduceColors { colors: u8 },
    /// 主な被写体だけを残す（被写体のまわりを透明にする）。`fine` なら、大きいモデルで細かく。
    Subject { fine: bool },
    /// トーンカーブ（RGB まとめて）。点は入力と出力（0〜255）。`count` 個だけ使う（2〜8）。
    Curves { points: [[u8; 2]; 8], count: u8 },
    /// 露光量（100 倍、-2000〜2000）、オフセット（1000 倍、-500〜500）、ガンマ（100 倍、1〜999）。
    Exposure { exposure: i16, offset: i16, gamma: u16 },
    /// 自然な彩度と彩度（-100〜100）。
    Vibrance { vibrance: i16, saturation: i16 },
    /// カラーバランス。シャドウ、中間調、ハイライトごとに、シアン-レッド、マゼンタ-グリーン、イエロー-ブルー（-100〜100）。
    ColorBalance { shadows: [i8; 3], midtones: [i8; 3], highlights: [i8; 3], preserve: bool },
    /// 白黒。レッド、イエロー、グリーン、シアン、ブルー、マゼンタの重み（%、-200〜300）。
    BlackWhite { weights: [i16; 6] },
    /// レンズフィルター。色と、適用量（%）。`preserve` なら明るさを保つ。
    PhotoFilter { color: [u8; 3], density: u8, preserve: bool },
    /// ポスタリゼーション（階調の数、2〜255）。
    Posterize { levels: u8 },
    /// 2 階調化（しきい値、1〜255）。
    Threshold { level: u8 },
    /// グラデーションマップ（暗い所の色から、明るい所の色へ）。
    GradientMap { from: [u8; 3], to: [u8; 3], reverse: bool },
}

/// トーンカーブの、はじめの点（まっすぐ）。
pub const CURVE_STRAIGHT: [[u8; 2]; 8] = [[0, 0], [255, 255], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0]];
/// 白黒の、はじめの重み（Photoshop の初期値）。
pub const BLACK_WHITE_DEFAULT: [i16; 6] = [40, 60, 40, 60, 20, 80];

impl AdjustKind {
    /// 案件のファイルに書く名前。
    pub fn key(self) -> &'static str {
        match self {
            AdjustKind::BrightnessContrast { .. } => "brightness_contrast",
            AdjustKind::HueSaturation { .. } => "hue_saturation",
            AdjustKind::Levels { .. } => "levels",
            AdjustKind::Invert => "invert",
            AdjustKind::ClearBackground { .. } => "clear_background",
            AdjustKind::FlattenShading => "flatten_shading",
            AdjustKind::ReduceColors { .. } => "reduce_colors",
            AdjustKind::Subject { .. } => "subject",
            AdjustKind::Curves { .. } => "curves",
            AdjustKind::Exposure { .. } => "exposure",
            AdjustKind::Vibrance { .. } => "vibrance",
            AdjustKind::ColorBalance { .. } => "color_balance",
            AdjustKind::BlackWhite { .. } => "black_white",
            AdjustKind::PhotoFilter { .. } => "photo_filter",
            AdjustKind::Posterize { .. } => "posterize",
            AdjustKind::Threshold { .. } => "threshold",
            AdjustKind::GradientMap { .. } => "gradient_map",
        }
    }

    /// 画素ごとに決まる調整か（調整レイヤーとして、下の絵すべてに効かせられるもの）。
    pub fn per_pixel(self) -> bool {
        !matches!(self, AdjustKind::ClearBackground { .. } | AdjustKind::FlattenShading | AdjustKind::ReduceColors { .. } | AdjustKind::Subject { .. })
    }

    /// 数値を、それぞれの範囲の中へ寄せる。
    pub fn clamped(self) -> AdjustKind {
        match self {
            AdjustKind::BrightnessContrast { brightness, contrast } => AdjustKind::BrightnessContrast { brightness: brightness.clamp(-150, 150), contrast: contrast.clamp(-50, 100) },
            AdjustKind::HueSaturation { hue, saturation, lightness } => AdjustKind::HueSaturation { hue: hue.clamp(-180, 180), saturation: saturation.clamp(-100, 100), lightness: lightness.clamp(-100, 100) },
            AdjustKind::Levels { black, white, gamma } => {
                let black = black.min(253);
                AdjustKind::Levels { black, white: white.max(black + 2), gamma: gamma.clamp(10, 999) }
            }
            AdjustKind::ClearBackground { tolerance } => AdjustKind::ClearBackground { tolerance: tolerance.clamp(1, 200) },
            AdjustKind::ReduceColors { colors } => AdjustKind::ReduceColors { colors: colors.clamp(2, 64) },
            AdjustKind::Curves { mut points, count } => {
                let count = count.clamp(2, 8);
                // 入力の順に並べ、同じ入力の点は 1 つにする。
                let mut used: Vec<[u8; 2]> = points[..count as usize].to_vec();
                used.sort_by_key(|p| p[0]);
                used.dedup_by_key(|p| p[0]);
                if used.len() < 2 {
                    used = vec![[0, 0], [255, 255]];
                }
                points = [[0, 0]; 8];
                points[..used.len()].copy_from_slice(&used);
                AdjustKind::Curves { points, count: used.len() as u8 }
            }
            AdjustKind::Exposure { exposure, offset, gamma } => AdjustKind::Exposure { exposure: exposure.clamp(-2000, 2000), offset: offset.clamp(-500, 500), gamma: gamma.clamp(1, 999) },
            AdjustKind::Vibrance { vibrance, saturation } => AdjustKind::Vibrance { vibrance: vibrance.clamp(-100, 100), saturation: saturation.clamp(-100, 100) },
            AdjustKind::ColorBalance { shadows, midtones, highlights, preserve } => {
                let c = |v: [i8; 3]| v.map(|x| x.clamp(-100, 100));
                AdjustKind::ColorBalance { shadows: c(shadows), midtones: c(midtones), highlights: c(highlights), preserve }
            }
            AdjustKind::BlackWhite { weights } => AdjustKind::BlackWhite { weights: weights.map(|w| w.clamp(-200, 300)) },
            AdjustKind::PhotoFilter { color, density, preserve } => AdjustKind::PhotoFilter { color, density: density.min(100), preserve },
            AdjustKind::Posterize { levels } => AdjustKind::Posterize { levels: levels.max(2) },
            AdjustKind::Threshold { level } => AdjustKind::Threshold { level: level.max(1) },
            other => other,
        }
    }
}

/// レイヤーマスク。柄の絵と同じ向きの、白黒の絵（255 が見せる、0 が隠す）。柄と一緒に動く。
#[derive(Clone, Debug, Eq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Arc<Vec<u8>>,
    /// 白と黒を逆にして使う。
    pub invert: bool,
    /// 偽なら、マスクを効かせない（Photoshop の「レイヤーマスクを使用しない」）。
    pub enabled: bool,
}

impl Mask {
    /// どこも見せるマスク。
    pub fn reveal_all(w: u32, h: u32) -> Mask {
        Mask { w, h, data: Arc::new(vec![255; (w * h) as usize]), invert: false, enabled: true }
    }

    /// 案件のファイルの中での名前（中身の印から）。
    fn file(&self) -> String {
        let id: String = Sha256::digest(self.data.as_slice()).iter().map(|b| format!("{b:02x}")).collect();
        format!("masks/{id}.gray")
    }
}

impl PartialEq for Mask {
    fn eq(&self, other: &Mask) -> bool {
        (self.w, self.h, self.invert, self.enabled) == (other.w, other.h, other.invert, other.enabled) && (Arc::ptr_eq(&self.data, &other.data) || self.data == other.data)
    }
}

impl std::hash::Hash for Mask {
    /// 中身は大きいので、場所（同じ中身を共有しているか）で見分ける。描くたびに中身を作り直すので、描いたら印も変わる。
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.w, self.h, self.invert, self.enabled, Arc::as_ptr(&self.data) as usize, self.data.len()).hash(state);
    }
}

/// マスクのファイルの名前として正しい形か（`masks/`、16 進 64 文字、`.gray`）。
fn is_mask_file(file: &str) -> bool {
    file.strip_prefix("masks/").and_then(|rest| rest.strip_suffix(".gray")).is_some_and(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// 案件 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub shape: Shape,
    /// 紙とコート。
    pub material: Material,
    /// 加工のレイヤー（箔押しなど）。並びは重なりの順。
    pub finishes: Vec<Finish>,
    pub assets: Vec<Asset>,
    /// 置いた柄。並びは重なりの順（後ろのものが上）。
    pub decals: Vec<Placement>,
    /// 展開図まるごとの絵（外のアプリで描いた PSD など）。柄の下に敷く。`size` は使わない。
    pub base: Option<Asset>,
    /// 紙の裏の、展開図まるごとの絵（裏の型紙の PSD に描いたもの）。裏から見た向きのまま持つ。`size` は使わない。
    pub back_base: Option<Asset>,
    /// 操作の記録（古い順）。`history/log.jsonl` に、1 行 1 件で入れる。
    pub log: Vec<LogEntry>,
    /// 残した版（古い順）。`versions/` に入れる。
    pub versions: Vec<Version>,
}

/// 残した版。その時点の案件（設定のテキストと、指している素材とマスク）。
/// 素材とマスクは中身の印で入れるので、いまの案件と同じ中身なら、ファイルは増えない。
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    /// 名前（自動の版は「エージェントの依頼の前」など）。
    pub name: String,
    /// いつ残したか（ISO 8601 の、その土地の時刻）。
    pub time: String,
    /// 自動で残した版（新しい 10 個だけ残す）。名前を付けて残した版は、使う人が消すまで残る。
    pub auto: bool,
    /// その時点の案件（記録と版は持たない）。
    pub project: Project,
}

/// 版の一覧のファイルの 1 行。
#[derive(Serialize, Deserialize)]
struct VersionEntry {
    file: String,
    name: String,
    time: String,
    #[serde(default)]
    auto: bool,
}

const VERSIONS_INDEX: &str = "versions/index.json";
/// 自動の版を残す数。
pub const AUTO_VERSIONS: usize = 10;

/// 操作の記録の 1 件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// いつ（ISO 8601 の、その土地の時刻）。
    pub time: String,
    /// 誰が。`person`（人が画面で）、`agent:<名前>`、`mcp:<名前>` など。
    pub actor: String,
    /// コマンドの名前（例: `layer.blend.set`）。
    pub command: String,
    /// 引数の要点（JSON）。絵のデータは入れない。
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub args: serde_json::Value,
}

/// 操作の記録を、案件のファイルに残す数（新しいものから）。
pub const LOG_LIMIT: usize = 2000;
const LOG_FILE: &str = "history/log.jsonl";
/// 記録の読み込みの上限（壊れたファイルで、メモリーを使い切らないため）。
const LOG_BYTES_LIMIT: u64 = 64 * 1024 * 1024;

/// 開いた結果。
#[derive(Debug)]
pub struct Opened {
    pub project: Project,
    /// 開くときに直した、または外したもの。空なら、保存したときのまま開けている。
    pub problems: Vec<String>,
}

/// 読み書きの失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイルを読めない、書けない。
    Io(io::Error),
    /// パケノバの案件ではない。
    NotProject,
    /// 新しい版のパケノバで保存された案件。値は、そのファイルの形式の版。
    Newer(u32),
    /// 中身が壊れている。または、書けない内容が入っている。
    Broken(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => f.write_str(&t!("Could not read or write the file: {e}")),
            Error::NotProject => f.write_str(t!("This is not a PAKENOBA project file")),
            Error::Newer(found) => f.write_str(&t!("This project was saved by a newer version of PAKENOBA (format version {found}; this version reads up to {VERSION})")),
            Error::Broken(what) => f.write_str(&t!("The project file is damaged: {what}")),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

// ---- 設定のテキストの形（版 1） ----

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    /// 書いたパケノバの版。調べるときの手がかりで、読むときには使わない。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    written_by: String,
    shape: ShapeEntry,
    /// 紙とコート。書いていなければ、はじめの素材（コートボール、コートなし）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    material: Option<MaterialEntry>,
    /// 展開図まるごとの絵。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<BaseEntry>,
    /// 紙の裏の、展開図まるごとの絵。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    back_base: Option<BaseEntry>,
    #[serde(rename = "asset", default, skip_serializing_if = "Vec::is_empty")]
    assets: Vec<AssetEntry>,
    #[serde(rename = "decal", default, skip_serializing_if = "Vec::is_empty")]
    decals: Vec<DecalEntry>,
    #[serde(rename = "finish", default, skip_serializing_if = "Vec::is_empty")]
    finishes: Vec<FinishEntry>,
}

/// 加工のレイヤー 1 つ。
#[derive(Serialize, Deserialize)]
struct FinishEntry {
    id: u32,
    /// `foil` か `emboss`（あとで、部分 UV ニスなどを足す）。
    kind: String,
    /// 箔の種類（箔押しのとき）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    foil: String,
    /// 形を借りる柄のレイヤーの印。
    link: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
    /// 箔のうねり（%）。書いていなければ、はじめの値。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wave: Option<u8>,
    /// エンボスの高さと縁の幅（mm）。書いていなければ、はじめの値。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    height: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    deboss: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edge: Option<f64>,
    /// つや消しの部分 UV ニス。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    matte: bool,
}

#[derive(Serialize, Deserialize)]
struct MaterialEntry {
    paper: String,
    coat: String,
}

#[derive(Serialize, Deserialize)]
struct ShapeEntry {
    /// `tuck_box`、`gable`、`pillow`、`prism` のどれか。
    kind: String,
    /// 角柱の角の数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sides: Option<usize>,
    width: f64,
    depth: f64,
    height: f64,
    thickness: f64,
    bulge: f64,
    roof: f64,
    /// 差し込み箱の、のりしろの幅、左右のフラップ、差し込みの長さ（mm）。書いていなければ自動。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    glue_flap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dust_flap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tuck_flap: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct AssetEntry {
    name: String,
    /// 案件のファイルの中での名前。
    file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    imported: String,
    size: [f64; 2],
}

#[derive(Serialize, Deserialize)]
struct BaseEntry {
    name: String,
    /// 案件のファイルの中での名前。
    file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    imported: String,
}

#[derive(Serialize, Deserialize)]
struct DecalEntry {
    /// レイヤーの印（0 なら書かない）。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    id: u32,
    /// 素材の番号（`[[asset]]` の並びで、0 から）。`builtin` と、どちらか一方を書く。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset: Option<usize>,
    /// 付属の見本の名前。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    builtin: Option<String>,
    /// 置かれているパネルの名前。
    panel: String,
    /// 同じ名前のパネルのうち、何番目か（1 から）。
    #[serde(default = "one", skip_serializing_if = "is_one")]
    nth: usize,
    at: [f64; 2],
    size: [f64; 2],
    /// 回転（ラジアン、反時計回り）。
    angle: f64,
    back: bool,
    /// レイヤーの名前（付けたときだけ書く）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    locked: bool,
    /// 画像をロック（調整とマスクを変えない）。
    #[serde(default, skip_serializing_if = "is_false")]
    lock_pixels: bool,
    /// 位置をロック（動かさない）。
    #[serde(default, skip_serializing_if = "is_false")]
    lock_position: bool,
    /// 不透明度（%）。100 のときは書かない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opacity: Option<u8>,
    /// 描画モード。通常のときは書かない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blend: Option<String>,
    /// 塗り（%）。100 のときは書かない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill: Option<u8>,
    /// 柄でないレイヤーの種類（`group` か `adjustment`）。柄のときは書かない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    /// グループを、レイヤーの欄で閉じている。
    #[serde(default, skip_serializing_if = "is_false")]
    closed: bool,
    /// 2026-11-24 の版で書いた、グループに入っている印（深さ 1 として読む）。
    #[serde(default, skip_serializing)]
    in_group: bool,
    /// グループの入れ子の深さ。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    depth: u8,
    /// すぐ下のレイヤーでクリップしている。
    #[serde(default, skip_serializing_if = "is_false")]
    clipped: bool,
    /// 文字の柄（`asset`、`builtin` のかわりに書く）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextEntry>,
    /// 図形の柄（`asset`、`builtin` のかわりに書く）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shape: Option<FigureEntry>,
    /// レイヤーマスク。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<MaskEntry>,
    /// 調整（下から順）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    adjust: Vec<AdjustEntry>,
}

#[derive(Serialize, Deserialize)]
struct MaskEntry {
    /// 案件のファイルの中での名前（`masks/<中身の印>.gray`）。
    file: String,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    invert: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    disabled: bool,
}

/// 調整 1 つ。`kind` で種類を書き、その種類の数値だけを書く。
#[derive(Serialize, Deserialize, Default)]
struct AdjustEntry {
    kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    off: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brightness: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contrast: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hue: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saturation: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lightness: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    black: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    white: Option<i64>,
    /// 中間の明るさ（ガンマの 100 倍）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gamma: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tolerance: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colors: Option<i64>,
    #[serde(default, skip_serializing_if = "is_false")]
    fine: bool,
    /// トーンカーブの点（入力、出力）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    points: Vec<[i64; 2]>,
    /// 露光量（100 倍）、オフセット（1000 倍）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exposure: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vibrance: Option<i64>,
    /// カラーバランス（シアン-レッド、マゼンタ-グリーン、イエロー-ブルー）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shadows: Option<[i64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    midtones: Option<[i64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    highlights: Option<[i64; 3]>,
    /// 明るさを保つ（カラーバランス、レンズフィルター）。
    #[serde(default, skip_serializing_if = "is_false")]
    preserve: bool,
    /// 白黒の重み（レッド、イエロー、グリーン、シアン、ブルー、マゼンタ）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weights: Option<[i64; 6]>,
    /// 色（`#rrggbb`。レンズフィルター）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    density: Option<i64>,
    /// 階調の数（ポスタリゼーション）、しきい値（2 階調化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    levels: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<i64>,
    /// グラデーションマップの両端の色（`#rrggbb`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    reverse: bool,
}

impl AdjustEntry {
    fn from(a: &Adjustment) -> AdjustEntry {
        let n = |v: i64| Some(v);
        let mut e = AdjustEntry { kind: a.kind.key().to_owned(), off: !a.on, ..Default::default() };
        match a.kind {
            AdjustKind::BrightnessContrast { brightness, contrast } => (e.brightness, e.contrast) = (n(brightness.into()), n(contrast.into())),
            AdjustKind::HueSaturation { hue, saturation, lightness } => (e.hue, e.saturation, e.lightness) = (n(hue.into()), n(saturation.into()), n(lightness.into())),
            AdjustKind::Levels { black, white, gamma } => (e.black, e.white, e.gamma) = (n(black.into()), n(white.into()), n(gamma.into())),
            AdjustKind::ClearBackground { tolerance } => e.tolerance = n(tolerance.into()),
            AdjustKind::ReduceColors { colors } => e.colors = n(colors.into()),
            AdjustKind::Subject { fine } => e.fine = fine,
            AdjustKind::Invert | AdjustKind::FlattenShading => {}
            AdjustKind::Curves { points, count } => e.points = points[..count as usize].iter().map(|p| [p[0].into(), p[1].into()]).collect(),
            AdjustKind::Exposure { exposure, offset, gamma } => (e.exposure, e.offset, e.gamma) = (n(exposure.into()), n(offset.into()), n(gamma.into())),
            AdjustKind::Vibrance { vibrance, saturation } => (e.vibrance, e.saturation) = (n(vibrance.into()), n(saturation.into())),
            AdjustKind::ColorBalance { shadows, midtones, highlights, preserve } => {
                let w = |v: [i8; 3]| Some(v.map(i64::from));
                (e.shadows, e.midtones, e.highlights, e.preserve) = (w(shadows), w(midtones), w(highlights), preserve);
            }
            AdjustKind::BlackWhite { weights } => e.weights = Some(weights.map(i64::from)),
            AdjustKind::PhotoFilter { color, density, preserve } => (e.color, e.density, e.preserve) = (Some(hex(color)), n(density.into()), preserve),
            AdjustKind::Posterize { levels } => e.levels = n(levels.into()),
            AdjustKind::Threshold { level } => e.level = n(level.into()),
            AdjustKind::GradientMap { from, to, reverse } => (e.from, e.to, e.reverse) = (Some(hex(from)), Some(hex(to)), reverse),
        }
        e
    }

    /// 知らない種類なら `None`。数値は範囲の中へ寄せる。書いていない数値は、はじめの値にする。
    fn to_adjustment(&self) -> Option<Adjustment> {
        let i16_ = |v: Option<i64>, d: i16| v.map_or(d, |v| v.clamp(-1000, 1000) as i16);
        let u8_ = |v: Option<i64>, d: u8| v.map_or(d, |v| v.clamp(0, 255) as u8);
        let kind = match self.kind.as_str() {
            "brightness_contrast" => AdjustKind::BrightnessContrast { brightness: i16_(self.brightness, 0), contrast: i16_(self.contrast, 0) },
            "hue_saturation" => AdjustKind::HueSaturation { hue: i16_(self.hue, 0), saturation: i16_(self.saturation, 0), lightness: i16_(self.lightness, 0) },
            "levels" => AdjustKind::Levels { black: u8_(self.black, 0), white: u8_(self.white, 255), gamma: self.gamma.map_or(100, |v| v.clamp(10, 999) as u16) },
            "invert" => AdjustKind::Invert,
            "clear_background" => AdjustKind::ClearBackground { tolerance: u8_(self.tolerance, 40) },
            "flatten_shading" => AdjustKind::FlattenShading,
            "reduce_colors" => AdjustKind::ReduceColors { colors: u8_(self.colors, 8) },
            "subject" => AdjustKind::Subject { fine: self.fine },
            "curves" => {
                let mut points = [[0u8; 2]; 8];
                let used: Vec<[u8; 2]> = self.points.iter().take(8).map(|p| [p[0].clamp(0, 255) as u8, p[1].clamp(0, 255) as u8]).collect();
                points[..used.len()].copy_from_slice(&used);
                AdjustKind::Curves { points, count: used.len() as u8 }
            }
            "exposure" => AdjustKind::Exposure { exposure: i16_(self.exposure, 0), offset: i16_(self.offset, 0), gamma: self.gamma.map_or(100, |v| v.clamp(1, 999) as u16) },
            "vibrance" => AdjustKind::Vibrance { vibrance: i16_(self.vibrance, 0), saturation: i16_(self.saturation, 0) },
            "color_balance" => {
                let r = |v: Option<[i64; 3]>| v.unwrap_or([0; 3]).map(|x| x.clamp(-100, 100) as i8);
                AdjustKind::ColorBalance { shadows: r(self.shadows), midtones: r(self.midtones), highlights: r(self.highlights), preserve: self.preserve }
            }
            "black_white" => AdjustKind::BlackWhite { weights: self.weights.map_or(BLACK_WHITE_DEFAULT, |w| w.map(|x| x.clamp(-200, 300) as i16)) },
            "photo_filter" => AdjustKind::PhotoFilter { color: self.color.as_deref().and_then(from_hex).unwrap_or([236, 138, 0]), density: u8_(self.density, 25), preserve: self.preserve },
            "posterize" => AdjustKind::Posterize { levels: u8_(self.levels, 4) },
            "threshold" => AdjustKind::Threshold { level: u8_(self.level, 128) },
            "gradient_map" => AdjustKind::GradientMap { from: self.from.as_deref().and_then(from_hex).unwrap_or([0, 0, 0]), to: self.to.as_deref().and_then(from_hex).unwrap_or([255, 255, 255]), reverse: self.reverse },
            _ => return None,
        };
        Some(Adjustment { kind: kind.clamped(), on: !self.off })
    }
}

#[derive(Serialize, Deserialize)]
struct FigureEntry {
    /// `rect` か `ellipse`。
    kind: String,
    /// 塗りの色（`#rrggbb`）。書かなければ塗らない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill: Option<String>,
    /// 線の色（`#rrggbb`）。書かなければ線を引かない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stroke: Option<String>,
    #[serde(default = "normal_stroke")]
    stroke_width: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    radius: f64,
}

fn normal_stroke() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize)]
struct TextEntry {
    text: String,
    font: String,
    /// 文字の大きさ（mm）。
    size: f64,
    /// 色（`#rrggbb`）。
    color: String,
    /// `left`、`center`、`right` のどれか。
    #[serde(default = "left", skip_serializing_if = "is_left")]
    align: String,
    /// 字間（全角の 1/1000）。
    #[serde(default, skip_serializing_if = "is_zero")]
    spacing: f64,
    /// 行送り（文字の大きさの何倍か）。
    #[serde(default = "normal_leading")]
    leading: f64,
}

fn left() -> String {
    "left".to_owned()
}

fn is_left(value: &str) -> bool {
    value == "left"
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn normal_leading() -> f64 {
    1.5
}

fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn from_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_zero_u8(value: &u8) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn one() -> usize {
    1
}

fn is_one(n: &usize) -> bool {
    *n == 1
}

/// 設定のテキストに書く数。読み戻すと、必ず元の値に戻る。
///
/// f32 をそのまま f64 にすると `0.10000000149011612` のような長い数になるので、
/// f32 として最も短い書き方（`0.1`）で書く。それで元に戻らないときだけ、長い書き方にする。
fn tidy(x: f32) -> f64 {
    let short: f64 = x.to_string().parse().unwrap_or(f64::from(x));
    if short as f32 == x { short } else { f64::from(x) }
}

fn tidy2(p: P2) -> [f64; 2] {
    [tidy(p[0]), tidy(p[1])]
}

/// 型紙でのパネルの番号を、名前と、同じ名前の中で何番目か（1 から）にする。
fn panel_ref(tpl: &Template, index: usize) -> (String, usize) {
    let name = tpl.panels[index].name;
    let nth = tpl.panels[..index].iter().filter(|p| p.name == name).count() + 1;
    (name.to_owned(), nth)
}

/// 名前と何番目かから、型紙でのパネルの番号を探す。
fn panel_index(tpl: &Template, name: &str, nth: usize) -> Option<usize> {
    tpl.panels.iter().enumerate().filter(|(_, p)| p.name == name).nth(nth.checked_sub(1)?).map(|(index, _)| index)
}

impl Project {
    /// 書く前の検査。番号の範囲と、数でない値を見る。
    fn check(&self, tpl: &Template) -> Result<(), Error> {
        let mut shape = self.shape;
        if let Some((label, ..)) = shape.values().into_iter().find(|(_, value, _)| !value.is_finite()) {
            return Err(Error::Broken(t!("Dimension \"{label}\" is not a number")));
        }
        for (index, asset) in self.assets.iter().enumerate() {
            if !asset.size.iter().all(|v| v.is_finite()) {
                return Err(Error::Broken(t!("The size of asset {} ({}) is not a number", index + 1, asset.name)));
            }
        }
        for (index, placed) in self.decals.iter().enumerate() {
            let number = index + 1;
            let decal = &placed.decal;
            if decal.panel >= tpl.panels.len() {
                return Err(Error::Broken(t!("Artwork {number} points to a panel that is not in the dieline")));
            }
            if ![decal.at[0], decal.at[1], decal.size[0], decal.size[1], decal.angle].iter().all(|v| v.is_finite()) {
                return Err(Error::Broken(t!("The position or size of artwork {number} is not a number")));
            }
            if let Some(mask) = &placed.layer.mask
                && (mask.w == 0 || mask.h == 0 || mask.data.len() != mask.w as usize * mask.h as usize)
            {
                return Err(Error::Broken(t!("The mask of artwork {number} does not match its size")));
            }
            if let Source::Asset(asset) = placed.source
                && asset >= self.assets.len()
            {
                return Err(Error::Broken(t!("Artwork {number} points to an asset that does not exist")));
            }
        }
        Ok(())
    }

    fn manifest(&self, tpl: &Template) -> Manifest {
        let s = &self.shape;
        let (kind, sides) = match s.kind {
            Kind::TuckBox => ("tuck_box", None),
            Kind::Gable => ("gable", None),
            Kind::Pillow => ("pillow", None),
            Kind::Prism(n) => ("prism", Some(n)),
        };
        Manifest {
            format: FORMAT.to_owned(),
            version: VERSION,
            written_by: format!("pakenoba {}", env!("CARGO_PKG_VERSION")),
            shape: ShapeEntry { kind: kind.to_owned(), sides, width: tidy(s.w), depth: tidy(s.d), height: tidy(s.h), thickness: tidy(s.t), bulge: tidy(s.bulge), roof: tidy(s.roof), glue_flap: s.flaps.glue.map(tidy), dust_flap: s.flaps.dust.map(tidy), tuck_flap: s.flaps.tuck.map(tidy) },
            material: (self.material != Material::default()).then(|| MaterialEntry { paper: self.material.paper.key().to_owned(), coat: self.material.coat.key().to_owned() }),
            base: self.base.as_ref().map(|a| BaseEntry { name: a.name.clone(), file: a.file(), imported: a.imported.clone() }),
            back_base: self.back_base.as_ref().map(|a| BaseEntry { name: a.name.clone(), file: a.file(), imported: a.imported.clone() }),
            assets: self.assets.iter().map(|a| AssetEntry { name: a.name.clone(), file: a.file(), imported: a.imported.clone(), size: tidy2(a.size) }).collect(),
            decals: self
                .decals
                .iter()
                .map(|placed| {
                    let (panel, nth) = panel_ref(tpl, placed.decal.panel);
                    let kind = match &placed.source {
                        Source::Group { .. } => Some("group".to_owned()),
                        Source::Adjustment => Some("adjustment".to_owned()),
                        _ => None,
                    };
                    let closed = matches!(placed.source, Source::Group { open: false });
                    let (asset, builtin, text, shape) = match &placed.source {
                        Source::Group { .. } | Source::Adjustment => (None, None, None, None),
                        Source::Asset(index) => (Some(*index), None, None, None),
                        Source::Builtin(name) => (None, Some(name.clone()), None, None),
                        Source::Text(spec) => (
                            None,
                            None,
                            Some(TextEntry { text: spec.text.clone(), font: spec.font.clone(), size: tidy(spec.size), color: hex(spec.color), align: spec.align.name().to_owned(), spacing: tidy(spec.spacing), leading: tidy(spec.leading) }),
                            None,
                        ),
                        Source::Shape(spec) => {
                            let kind = match spec.kind {
                                ShapeKind::Rect => "rect",
                                ShapeKind::Ellipse => "ellipse",
                            };
                            (None, None, None, Some(FigureEntry { kind: kind.to_owned(), fill: spec.fill.map(hex), stroke: spec.stroke.map(hex), stroke_width: tidy(spec.stroke_width), radius: tidy(spec.radius) }))
                        }
                    };
                    DecalEntry { id: placed.layer.id, asset, builtin, panel, nth, at: tidy2(placed.decal.at), size: tidy2(placed.decal.size), angle: tidy(placed.decal.angle), back: placed.back, name: placed.layer.name.clone(), hidden: placed.layer.hidden, locked: placed.layer.locked, lock_pixels: placed.layer.lock_pixels, lock_position: placed.layer.lock_position, opacity: (placed.layer.opacity < 100).then_some(placed.layer.opacity), fill: (placed.layer.fill < 100).then_some(placed.layer.fill), blend: (placed.layer.blend != Blend::Normal).then(|| placed.layer.blend.key().to_owned()), kind, closed, in_group: false, depth: placed.layer.depth, clipped: placed.layer.clipped, text, shape, mask: placed.layer.mask.as_ref().map(|m| MaskEntry { file: m.file(), width: m.w, height: m.h, invert: m.invert, disabled: !m.enabled }), adjust: placed.layer.adjust.iter().map(AdjustEntry::from).collect() }
                })
                .collect(),
            finishes: self
                .finishes
                .iter()
                .map(|f| {
                    let foil = f.kind == FinishKind::Foil;
                    let emboss = f.kind == FinishKind::Emboss;
                    FinishEntry {
                        id: f.id,
                        kind: f.kind.key().to_owned(),
                        foil: if foil { f.foil.key().to_owned() } else { String::new() },
                        link: f.link,
                        name: f.name.clone(),
                        hidden: f.hidden,
                        wave: (foil && f.wave != DEFAULT_WAVE).then_some(f.wave),
                        height: (emboss && f.height != DEFAULT_EMBOSS_HEIGHT).then(|| tidy(f.height)),
                        deboss: emboss && f.deboss,
                        edge: (emboss && f.edge != DEFAULT_EMBOSS_EDGE).then(|| tidy(f.edge)),
                        matte: f.kind == FinishKind::SpotUv && f.matte,
                    }
                })
                .collect(),
        }
    }

    /// 案件を、案件のファイルの形で書き出す。
    pub fn write<W: Write + Seek>(&self, out: W) -> Result<(), Error> {
        let mut zip = ZipWriter::new(out);
        let mut written: Vec<String> = Vec::new();
        self.write_parts(&mut zip, MANIFEST, &mut written)?;
        if !self.log.is_empty() {
            let mut lines = String::new();
            for entry in &self.log[self.log.len().saturating_sub(LOG_LIMIT)..] {
                lines.push_str(&serde_json::to_string(entry).map_err(|e| Error::Broken(e.to_string()))?);
                lines.push('\n');
            }
            zip.start_file(LOG_FILE, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zip_error)?;
            zip.write_all(lines.as_bytes())?;
        }
        if !self.versions.is_empty() {
            let mut index = Vec::new();
            for (n, version) in self.versions.iter().enumerate() {
                let file = format!("versions/{:03}.toml", n + 1);
                version.project.write_parts(&mut zip, &file, &mut written)?;
                index.push(VersionEntry { file, name: version.name.clone(), time: version.time.clone(), auto: version.auto });
            }
            let text = serde_json::to_string_pretty(&index).map_err(|e| Error::Broken(e.to_string()))?;
            zip.start_file(VERSIONS_INDEX, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zip_error)?;
            zip.write_all(text.as_bytes())?;
        }
        let mut out = zip.finish().map_err(zip_error)?;
        out.flush()?;
        Ok(())
    }

    /// 設定のテキストを `manifest` の名前で書き、素材とマスクのファイルを書く（`written` にあるものは書かない）。
    fn write_parts<W: Write + Seek>(&self, zip: &mut ZipWriter<W>, manifest: &str, written: &mut Vec<String>) -> Result<(), Error> {
        let tpl = self.shape.template();
        self.check(&tpl)?;
        let text = toml::to_string(&self.manifest(&tpl)).map_err(|e| Error::Broken(t!("Could not make the settings text: {e}")))?;
        zip.start_file(manifest, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zip_error)?;
        zip.write_all(text.as_bytes())?;
        for asset in self.assets.iter().chain(&self.base).chain(&self.back_base) {
            let file = asset.file();
            if written.contains(&file) {
                continue;
            }
            // すでに圧縮されている形式は、そのまま入れる（圧縮しても小さくならず、時間だけかかる）。
            let packed = matches!(extension(&asset.name).as_deref(), Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "zip"));
            let method = if packed { CompressionMethod::Stored } else { CompressionMethod::Deflated };
            let options = SimpleFileOptions::default().compression_method(method).large_file(asset.bytes.len() as u64 >= u64::from(u32::MAX));
            zip.start_file(file.as_str(), options).map_err(zip_error)?;
            zip.write_all(&asset.bytes)?;
            written.push(file);
        }
        for mask in self.decals.iter().filter_map(|placed| placed.layer.mask.as_ref()) {
            let file = mask.file();
            if written.contains(&file) {
                continue;
            }
            zip.start_file(file.as_str(), SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zip_error)?;
            zip.write_all(&mask.data)?;
            written.push(file);
        }
        Ok(())
    }

    /// 案件のファイルの形から読む。
    pub fn read<R: Read + Seek>(input: R) -> Result<Opened, Error> {
        let mut zip = ZipArchive::new(input).map_err(|e| match e {
            zip::result::ZipError::Io(e) => Error::Io(e),
            _ => Error::NotProject,
        })?;
        let mut opened = read_manifest(&mut zip, MANIFEST)?;
        opened.project.log = read_log(&mut zip, &mut opened.problems);
        opened.project.versions = read_versions(&mut zip, &mut opened.problems);
        Ok(opened)
    }

    /// 案件を、ファイルに保存する。別の名前で書ききってから入れ替えるので、途中で止まっても、前のファイルが残る。
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        replace_file(path, |file| self.write(BufWriter::with_capacity(1 << 20, file)))
    }

    /// 設定のテキスト（`project.toml` の中身）だけから読む。素材とマスクを使わない案件（付属の見本など）のため。
    pub fn from_text(text: &str) -> Result<Opened, Error> {
        let mut zip = ZipWriter::new(io::Cursor::new(Vec::new()));
        zip.start_file(MANIFEST, SimpleFileOptions::default()).map_err(|e| Error::Io(io::Error::other(e)))?;
        zip.write_all(text.as_bytes())?;
        let bytes = zip.finish().map_err(|e| Error::Io(io::Error::other(e)))?.into_inner();
        Project::read(io::Cursor::new(bytes))
    }

    /// 案件のファイルを開く。
    pub fn open(path: &Path) -> Result<Opened, Error> {
        Project::read(BufReader::with_capacity(1 << 20, File::open(path)?))
    }
}

/// 設定のテキスト（`name`）と、それが指す素材とマスクを読む。
fn read_manifest<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Opened, Error> {
    let text = {
        let entry = zip.by_name(name).map_err(|e| match e {
            zip::result::ZipError::Io(e) => Error::Io(e),
            _ => Error::NotProject,
        })?;
        let mut bytes = Vec::new();
        entry.take(MANIFEST_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| Error::Broken(t!("Could not extract the settings text: {e}")))?;
        if bytes.len() as u64 > MANIFEST_LIMIT {
            return Err(Error::Broken(t!("The settings text is too large").to_owned()));
        }
        String::from_utf8(bytes).map_err(|_| Error::Broken(t!("The characters of the settings text cannot be read").to_owned()))?
    };
    let table: toml::Table = toml::from_str(&text).map_err(|e| Error::Broken(t!("Could not read the settings text: {}", e.message())))?;
    if table.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
        return Err(Error::NotProject);
    }
    let version = table.get("version").and_then(|v| v.as_integer()).ok_or_else(|| Error::Broken(t!("The format version is missing").to_owned()))?;
    match u32::try_from(version) {
        Ok(1) => from_v1(table, zip),
        Ok(found) if found > VERSION => Err(Error::Newer(found)),
        _ => Err(Error::Broken(t!("The format version ({version}) is not valid"))),
    }
}

/// 残した版を読む。読めない版は外して知らせる（案件そのものは開く）。
fn read_versions<R: Read + Seek>(zip: &mut ZipArchive<R>, problems: &mut Vec<String>) -> Vec<Version> {
    let Ok(bytes) = read_entry(zip, VERSIONS_INDEX) else { return Vec::new() };
    let Ok(index) = serde_json::from_slice::<Vec<VersionEntry>>(&bytes) else {
        problems.push(t!("The list of saved versions could not be read, so the versions were left out").to_owned());
        return Vec::new();
    };
    let mut versions = Vec::new();
    for entry in index {
        if !entry.file.starts_with("versions/") {
            continue;
        }
        match read_manifest(zip, &entry.file) {
            Ok(opened) => {
                let mut project = opened.project;
                (project.log, project.versions) = (Vec::new(), Vec::new());
                versions.push(Version { name: entry.name, time: entry.time, auto: entry.auto, project });
            }
            Err(e) => problems.push(t!("The version \"{}\" could not be read, so it was left out ({e})", entry.name)),
        }
    }
    versions
}

/// 操作の記録を読む。ないときは空。読めない行は飛ばして、知らせる。
fn read_log<R: Read + Seek>(zip: &mut ZipArchive<R>, problems: &mut Vec<String>) -> Vec<LogEntry> {
    let Ok(entry) = zip.by_name(LOG_FILE) else { return Vec::new() };
    let mut text = String::new();
    if entry.take(LOG_BYTES_LIMIT).read_to_string(&mut text).is_err() {
        problems.push(t!("The operation history could not be read, so it was left out").to_owned());
        return Vec::new();
    }
    let mut log = Vec::new();
    let mut bad = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<LogEntry>(line) {
            Ok(entry) => log.push(entry),
            Err(_) => bad += 1,
        }
    }
    if bad > 0 {
        problems.push(t!("{bad} line(s) of the operation history could not be read, so they were left out"));
    }
    log
}

/// 調整を JSON にする（コマンドの引数や結果に使う。案件のファイルの書き方と同じ）。
pub fn adjustment_to_json(adjust: &Adjustment) -> serde_json::Value {
    serde_json::to_value(AdjustEntry::from(adjust)).unwrap_or_default()
}

/// JSON から調整を作る。知らない種類なら、理由を返す。
pub fn adjustment_from_json(value: &serde_json::Value) -> Result<Adjustment, String> {
    let entry: AdjustEntry = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    entry.to_adjustment().ok_or_else(|| t!("Unknown adjustment \"{}\"", entry.kind))
}

fn zip_error(e: zip::result::ZipError) -> Error {
    match e {
        zip::result::ZipError::Io(e) => Error::Io(e),
        other => Error::Broken(t!("Could not write it as ZIP: {other}")),
    }
}

/// 版 1 の設定のテキストから、案件を組み立てる。
fn from_v1<R: Read + Seek>(table: toml::Table, zip: &mut ZipArchive<R>) -> Result<Opened, Error> {
    let manifest: Manifest = table.try_into().map_err(|e: toml::de::Error| Error::Broken(t!("Could not read the settings text: {}", e.message())))?;
    let mut problems = Vec::new();

    // 型と寸法。
    let entry = &manifest.shape;
    let kind = match (entry.kind.as_str(), entry.sides) {
        ("tuck_box", _) => Kind::TuckBox,
        ("gable", _) => Kind::Gable,
        ("pillow", _) => Kind::Pillow,
        ("prism", Some(n)) if SIDES.contains(&n) => Kind::Prism(n),
        ("prism", _) => return Err(Error::Broken(t!("The number of sides of the prism is not valid").to_owned())),
        (other, _) => return Err(Error::Broken(t!("This version of PAKENOBA does not know the shape \"{other}\""))),
    };
    let mut shape = Shape { kind, w: entry.width as f32, d: entry.depth as f32, h: entry.height as f32, t: entry.thickness as f32, bulge: entry.bulge as f32, roof: entry.roof as f32, flaps: TuckFlaps::default() };
    for (label, value, range) in shape.values() {
        if !value.is_finite() {
            return Err(Error::Broken(t!("Dimension \"{label}\" is not a number")));
        }
        if !range.contains(value) {
            let fixed = value.clamp(*range.start(), *range.end());
            problems.push(t!("Dimension \"{label}\" was out of range ({value}), so it was set to {fixed}"));
            *value = fixed;
        }
    }
    // 差し込み箱のフラップ。数でないものは自動に戻し、範囲の外は範囲の中へ寄せる。
    if shape.kind == Kind::TuckBox {
        let ranges = TuckFlaps::ranges(shape.w, shape.d, shape.h, shape.t);
        let entries = [(t!("Glue flap width"), entry.glue_flap), (t!("Side flap length"), entry.dust_flap), (t!("Tuck length"), entry.tuck_flap)];
        let mut values = [None; 3];
        for (i, (label, value)) in entries.into_iter().enumerate() {
            let Some(value) = value else { continue };
            let value = value as f32;
            if !value.is_finite() {
                problems.push(t!("Dimension \"{label}\" is not a number, so it was set automatically"));
                continue;
            }
            let fixed = value.clamp(*ranges[i].start(), *ranges[i].end());
            if fixed != value {
                problems.push(t!("Dimension \"{label}\" was out of range ({value}), so it was set to {fixed}"));
            }
            values[i] = Some(fixed);
        }
        shape.flaps = TuckFlaps { glue: values[0], dust: values[1], tuck: values[2] };
    }
    let tpl = shape.template();

    // 素材。読めなかったものは外して、番号の対応を覚えておく。
    let mut assets: Vec<Asset> = Vec::new();
    let mut kept: Vec<Option<usize>> = Vec::new();
    let mut loaded: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    // 中身は、同じファイルなら 1 回だけ読む。中身の印は、ファイルの名前から取る（中身の傷は、ZIP の検査で見つかる）。
    let mut load = |zip: &mut ZipArchive<R>, file: &str, size: P2| -> Result<(String, Arc<Vec<u8>>), String> {
        if !is_asset_file(file) || !size.iter().all(|v| v.is_finite()) {
            return Err(t!("The contents are not valid").to_owned());
        }
        let bytes = match loaded.get(file) {
            Some(bytes) => bytes.clone(),
            None => {
                let bytes = Arc::new(read_entry(zip, file)?);
                loaded.insert(file.to_owned(), bytes.clone());
                bytes
            }
        };
        Ok((file["assets/".len()..].split('.').next().unwrap_or_default().to_owned(), bytes))
    };
    for entry in &manifest.assets {
        let size = [entry.size[0] as f32, entry.size[1] as f32];
        match load(zip, &entry.file, size) {
            Ok((id, bytes)) => {
                kept.push(Some(assets.len()));
                assets.push(Asset { name: entry.name.clone(), imported: entry.imported.clone(), size, id, bytes });
            }
            Err(why) => {
                kept.push(None);
                problems.push(t!("Asset \"{}\" could not be read, so it was removed ({why})", entry.name));
            }
        }
    }
    // 展開図まるごとの絵。
    let mut base_of = |entry: &Option<BaseEntry>, what: &str| {
        entry.as_ref().and_then(|entry| match load(zip, &entry.file, [0.0, 0.0]) {
            Ok((id, bytes)) => Some(Asset { name: entry.name.clone(), imported: entry.imported.clone(), size: [0.0, 0.0], id, bytes }),
            Err(why) => {
                problems.push(t!("{what} \"{}\" could not be read, so it was removed ({why})", entry.name));
                None
            }
        })
    };
    let base = base_of(&manifest.base, t!("Dieline artwork"));
    let back_base = base_of(&manifest.back_base, t!("Dieline artwork for the back of the sheet"));

    // 柄。
    let mut decals = Vec::new();
    for (index, entry) in manifest.decals.iter().enumerate() {
        let number = index + 1;
        let source = match (entry.asset, &entry.builtin) {
            _ if entry.kind.is_some() => match entry.kind.as_deref() {
                Some("group") => Source::Group { open: !entry.closed },
                Some("adjustment") => Source::Adjustment,
                Some(other) => {
                    problems.push(t!("Layer {number} is a kind this version does not know (\"{other}\"), so it was removed"));
                    continue;
                }
                None => unreachable!(),
            },
            (None, None) if entry.shape.is_some() => {
                let Some(shape) = &entry.shape else { unreachable!() };
                let kind = match shape.kind.as_str() {
                    "rect" => ShapeKind::Rect,
                    "ellipse" => ShapeKind::Ellipse,
                    other => {
                        problems.push(t!("The shape type of artwork {number} ({other}) cannot be read, so it was made a rectangle"));
                        ShapeKind::Rect
                    }
                };
                let mut color = |text: &Option<String>, what: &str| {
                    text.as_ref().map(|t| {
                        from_hex(t).unwrap_or_else(|| {
                            problems.push(t!("The {what} color of shape artwork {number} cannot be read, so it was made black"));
                            [0, 0, 0]
                        })
                    })
                };
                let (fill, stroke) = (color(&shape.fill, t!("Fill")), color(&shape.stroke, t!("Stroke")));
                let clamp = |v: f64, range: &RangeInclusive<f32>, fallback: f32| if v.is_finite() { (v as f32).clamp(*range.start(), *range.end()) } else { fallback };
                Source::Shape(ShapeSpec { kind, fill, stroke, stroke_width: clamp(shape.stroke_width, &SHAPE_STROKE, 1.0), radius: clamp(shape.radius, &SHAPE_RADIUS, 0.0) })
            }
            (None, None) if entry.text.is_some() => {
                let Some(text) = &entry.text else { unreachable!() };
                let clamp = |v: f64, range: &RangeInclusive<f32>, what: &str, problems: &mut Vec<String>| {
                    let v = v as f32;
                    if !v.is_finite() {
                        problems.push(t!("The {what} of text artwork {number} was not valid, so the normal value was used"));
                        return None;
                    }
                    let c = v.clamp(*range.start(), *range.end());
                    if c != v {
                        problems.push(t!("The {what} of text artwork {number} was out of range, so it was set to {c}"));
                    }
                    Some(c)
                };
                let color = from_hex(&text.color).unwrap_or_else(|| {
                    problems.push(t!("The color of text artwork {number} cannot be read, so it was made black"));
                    [0, 0, 0]
                });
                let align = TextAlign::from_name(&text.align).unwrap_or_else(|| {
                    problems.push(t!("The alignment of text artwork {number} cannot be read, so it was aligned left"));
                    TextAlign::Left
                });
                Source::Text(TextSpec {
                    text: text.text.clone(),
                    font: text.font.clone(),
                    size: clamp(text.size, &TEXT_SIZE, t!("Size"), &mut problems).unwrap_or(10.0),
                    color,
                    align,
                    spacing: clamp(text.spacing, &TEXT_SPACING, t!("Tracking"), &mut problems).unwrap_or(0.0),
                    leading: clamp(text.leading, &TEXT_LEADING, t!("Leading"), &mut problems).unwrap_or(1.5),
                })
            }
            (Some(asset), None) => match kept.get(asset) {
                Some(Some(now)) => Source::Asset(*now),
                Some(None) => {
                    problems.push(t!("Artwork {number} was removed because asset \"{}\" could not be read", manifest.assets[asset].name));
                    continue;
                }
                None => {
                    problems.push(t!("Artwork {number} was removed because it pointed to an asset that does not exist"));
                    continue;
                }
            },
            (None, Some(name)) => Source::Builtin(name.clone()),
            _ => {
                problems.push(t!("Artwork {number} was removed because its picture setting was not valid"));
                continue;
            }
        };
        let Some(panel) = panel_index(&tpl, &entry.panel, entry.nth) else {
            problems.push(t!("Artwork {number} was removed because its panel \"{}\" is not in the dieline", pakenoba_i18n::trc("panel", &entry.panel)));
            continue;
        };
        let decal = Decal { panel, at: [entry.at[0] as f32, entry.at[1] as f32], size: [entry.size[0] as f32, entry.size[1] as f32], angle: entry.angle as f32 };
        let numbers = [decal.at[0], decal.at[1], decal.size[0], decal.size[1], decal.angle];
        if !numbers.iter().all(|v| v.is_finite()) || decal.size[0] <= 0.0 || decal.size[1] <= 0.0 {
            problems.push(t!("Artwork {number} was removed because its position or size was not valid"));
            continue;
        }
        let blend = match entry.blend.as_deref() {
            None => Blend::Normal,
            Some(key) => Blend::from_key(key).unwrap_or_else(|| {
                problems.push(t!("Artwork {number} uses a blend mode this version does not know (\"{key}\"), so it was set to Normal"));
                Blend::Normal
            }),
        };
        let opacity = entry.opacity.unwrap_or(100).min(100);
        let blend = if blend == Blend::PassThrough && !matches!(source, Source::Group { .. }) { Blend::Normal } else { blend };
        let mut adjust = Vec::new();
        for a in &entry.adjust {
            match a.to_adjustment() {
                Some(a) if matches!(source, Source::Adjustment) && !a.kind.per_pixel() => problems.push(t!("Adjustment layer {number} cannot use \"{}\", so it was removed", a.kind.key())),
                Some(a) => adjust.push(a),
                None => problems.push(t!("Artwork {number} uses an adjustment this version does not know (\"{}\"), so it was removed", a.kind)),
            }
        }
        let mask = match &entry.mask {
            Some(m) if is_mask_file(&m.file) && m.width > 0 && m.height > 0 && u64::from(m.width) * u64::from(m.height) <= 1 << 26 => match read_entry(zip, &m.file) {
                Ok(data) if data.len() == m.width as usize * m.height as usize => Some(Mask { w: m.width, h: m.height, data: Arc::new(data), invert: m.invert, enabled: !m.disabled }),
                Ok(_) => {
                    problems.push(t!("The mask of artwork {number} does not match its size, so it was removed"));
                    None
                }
                Err(why) => {
                    problems.push(t!("The mask of artwork {number} could not be read, so it was removed ({why})"));
                    None
                }
            },
            Some(_) => {
                problems.push(t!("The mask of artwork {number} could not be read, so it was removed ({why})", why = t!("The contents are not valid")));
                None
            }
            None => None,
        };
        let mask = if matches!(source, Source::Group { .. } | Source::Adjustment) { None } else { mask };
        let fill = if matches!(source, Source::Group { .. }) { 100 } else { entry.fill.unwrap_or(100).min(100) };
        decals.push(Placement { decal, back: entry.back, source, layer: Layer { id: entry.id, adjust, mask, depth: if entry.depth > 0 { entry.depth } else { entry.in_group as u8 }, clipped: entry.clipped, name: entry.name.clone().filter(|n| !n.trim().is_empty()), hidden: entry.hidden, locked: entry.locked, lock_pixels: entry.lock_pixels, lock_position: entry.lock_position, opacity, fill, blend } });
    }
    tidy_groups(&mut decals);

    // 紙とコート。知らない名前は、はじめのものにして知らせる。
    let mut material = Material::default();
    if let Some(entry) = &manifest.material {
        match Paper::from_key(&entry.paper) {
            Some(paper) => material.paper = paper,
            None => problems.push(t!("This version of PAKENOBA does not know the paper \"{}\", so it was made coated board", entry.paper)),
        }
        match Coat::from_key(&entry.coat) {
            Some(coat) => material.coat = coat,
            None => problems.push(t!("This version of PAKENOBA does not know the coating \"{}\", so it was removed", entry.coat)),
        }
    }

    // 加工のレイヤー。知らない種類は外して知らせる。
    let mut finishes = Vec::new();
    for (i, entry) in manifest.finishes.iter().enumerate() {
        let number = i + 1;
        let base = |mut f: Finish| {
            f.id = entry.id;
            f.name = entry.name.clone().filter(|n| !n.trim().is_empty());
            f.hidden = entry.hidden;
            f
        };
        match (FinishKind::from_key(&entry.kind), Foil::from_key(&entry.foil)) {
            (Some(FinishKind::Foil), Some(foil)) => finishes.push(base(Finish { wave: entry.wave.unwrap_or(DEFAULT_WAVE).min(100), ..Finish::foil(foil, entry.link) })),
            (Some(FinishKind::SpotUv), _) => finishes.push(base(Finish::spot_uv(entry.link, entry.matte))),
            (Some(FinishKind::White), _) => finishes.push(base(Finish::white(entry.link))),
            (Some(FinishKind::Emboss), _) => {
                let clamp = |v: Option<f64>, range: &std::ops::RangeInclusive<f32>, default: f32| v.filter(|v| v.is_finite()).map_or(default, |v| (v as f32).clamp(*range.start(), *range.end()));
                finishes.push(base(Finish { height: clamp(entry.height, &EMBOSS_HEIGHT, DEFAULT_EMBOSS_HEIGHT), edge: clamp(entry.edge, &EMBOSS_EDGE, DEFAULT_EMBOSS_EDGE), ..Finish::emboss(entry.link, entry.deboss) }))
            }
            _ => problems.push(t!("Finish layer {number} ({kind}, {foil}) is not known to this version of PAKENOBA, so it was removed", kind = entry.kind, foil = entry.foil)),
        }
    }

    Ok(Opened { project: Project { shape, material, finishes, assets, decals, base, back_base, log: Vec::new(), versions: Vec::new() }, problems })
}

/// グループの入れ子とクリッピングマスクの決まりに合うように、印を直す（読むとき、並べ替えたあと）。
pub fn tidy_groups(decals: &mut [Placement]) {
    let groups: Vec<bool> = decals.iter().map(|p| matches!(p.source, Source::Group { .. })).collect();
    let backs: Vec<bool> = decals.iter().map(|p| p.back).collect();
    let mut depth: Vec<u8> = decals.iter().map(|p| p.layer.depth).collect();
    let mut clipped: Vec<bool> = decals.iter().map(|p| p.layer.clipped).collect();
    nest_rule(&groups, &backs, &mut depth, &mut clipped);
    for ((p, d), c) in decals.iter_mut().zip(depth).zip(clipped) {
        (p.layer.depth, p.layer.clipped) = (d, c);
    }
}

/// グループの入れ子の深さの上限。
pub const MAX_DEPTH: u8 = 8;

/// 入れ子とクリップの決まりを、印の並びに当てはめる（アプリの側の並びにも使う）。`groups[i]` は見出しか、`backs[i]` は紙の裏か。
///
/// - 並びは下から。面ごとに、上から見ていく。見出しを見つけたら、そのすぐ下に続く、見出しより深いものが中身。
/// - 深さは、いま開いているグループの数より深くできない（深すぎるものは浅くする）。
/// - クリップできるのは、同じ深さのすぐ下にレイヤーがある、見出しでないもの。
pub fn nest_rule(groups: &[bool], backs: &[bool], depth: &mut [u8], clipped: &mut [bool]) {
    let n = depth.len();
    for side in [false, true] {
        let items: Vec<usize> = (0..n).filter(|&i| backs[i] == side).collect();
        // 上から: 開いている見出しの深さ。
        let mut open: Vec<u8> = Vec::new();
        for &i in items.iter().rev() {
            while open.last().is_some_and(|&d| d >= depth[i]) {
                open.pop();
            }
            depth[i] = depth[i].min(open.len() as u8);
            if groups[i] {
                if depth[i] >= MAX_DEPTH {
                    depth[i] = MAX_DEPTH - 1;
                }
                open.push(depth[i]);
            }
        }
        // 下から: クリップの土台があるか。
        for (k, &i) in items.iter().enumerate() {
            if !clipped[i] {
                continue;
            }
            let base = k.checked_sub(1).map(|b| items[b]);
            clipped[i] = !groups[i] && base.is_some_and(|b| depth[b] == depth[i]);
        }
    }
}

/// ZIP の中のファイルを 1 つ、全部読む。中身の傷（CRC の不一致）も、ここで見つかる。
fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip.by_name(name).map_err(|e| match e {
        zip::result::ZipError::FileNotFound => t!("Not found in the file").to_owned(),
        other => t!("Cannot extract: {other}"),
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|e| t!("The contents are damaged: {e}"))?;
    Ok(bytes)
}

/// ファイルを、別の名前で書ききってから、元のファイルと入れ替える。
///
/// `write` が失敗したとき、または途中でアプリが止まったときは、元のファイルはそのまま残る。
/// 書きかけのファイル（同じフォルダの `<名前>.tmp-<番号>`）は、失敗したら消す。
pub fn replace_file(path: &Path, write: impl FnOnce(&mut File) -> Result<(), Error>) -> Result<(), Error> {
    let name = path.file_name().ok_or_else(|| Error::Io(io::Error::new(io::ErrorKind::InvalidInput, "The file has no name")))?;
    let mut temp_name = name.to_owned();
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let result = (|| {
        let mut file = File::create(&temp)?;
        write(&mut file)?;
        // ディスクに書ききってから入れ替える。
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn shape(kind: Kind) -> Shape {
        let (w, d, h) = match kind {
            Kind::TuckBox => (200.0, 160.0, 85.0),
            Kind::Gable => (160.0, 100.0, 110.0),
            Kind::Pillow => (180.0, 60.0, 110.0),
            Kind::Prism(_) => (70.0, 60.0, 160.0),
        };
        Shape { kind, w, d, h, t: 0.5, bulge: 70.0, roof: 45.0, flaps: if kind == Kind::TuckBox { TuckFlaps { glue: Some(9.5), dust: None, tuck: Some(22.25) } } else { TuckFlaps::default() } }
    }

    /// 中身の決まった、にせの画像ファイル。
    fn fake_file(seed: u32, len: usize) -> Vec<u8> {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect()
    }

    /// 型ごとに、すべてのパネルへ柄を置いた案件。
    fn sample(kind: Kind) -> Project {
        let shape = shape(kind);
        let tpl = shape.template();
        let assets = vec![
            Asset::new("ロゴ 最終版.png", fake_file(1, 3000), [46.0, 46.0], "2026-11-22T00:10:00+09:00"),
            Asset::new("pattern.PSD", fake_file(2, 9000), [120.0, 80.5], ""),
            Asset::new("名前に点がない", fake_file(3, 10), [10.0, 0.1], "2026-01-01T00:00:00Z"),
        ];
        let decals = (0..tpl.panels.len())
            .map(|panel| {
                let k = panel as f32;
                let source = match panel % 5 {
                    0 => Source::Builtin("logo".to_owned()),
                    1 => Source::Builtin("band".to_owned()),
                    4 => Source::Text(TextSpec {
                        text: format!("新発売\n{panel} 番の \"文字\" = 'テスト'"),
                        font: "Noto Sans CJK JP".to_owned(),
                        size: 4.0 + k * 0.7,
                        color: [k as u8 * 9, 200, 17],
                        align: [TextAlign::Left, TextAlign::Center, TextAlign::Right][panel % 3],
                        spacing: if panel % 2 == 0 { 0.0 } else { -25.5 },
                        leading: 1.25 + k / 10.0,
                    }),
                    3 if panel % 2 == 0 => Source::Shape(ShapeSpec {
                        kind: if panel % 4 == 0 { ShapeKind::Ellipse } else { ShapeKind::Rect },
                        fill: (panel % 3 != 0).then_some([10, k as u8, 200]),
                        stroke: Some([0, 0, k as u8]),
                        stroke_width: 0.75 + k / 4.0,
                        radius: k,
                    }),
                    n => Source::Asset(n - 2),
                };
                Placement { decal: Decal { panel, at: [0.1 + k / 37.0, 1.0 / 3.0 - k], size: [46.0 + k * 0.1, 14.0 / (k + 1.0)], angle: std::f32::consts::FRAC_PI_2 * k - 0.3 }, back: panel % 3 == 0, source, layer: Layer {
                    id: panel as u32 + 1,
                    name: (panel % 4 == 1).then(|| format!("名前 {panel}")),
                    hidden: panel % 5 == 2,
                    locked: panel % 7 == 3,
                    lock_pixels: panel % 4 == 2,
                    lock_position: panel % 6 == 1,
                    opacity: [100, 35, 0][panel % 3],
                    fill: [100, 60, 5][panel % 3],
                    blend: Blend::MENU.iter().flatten().copied().nth(panel % 26).unwrap_or_default(),
                    adjust: match panel % 4 {
                        0 => vec![],
                        1 => vec![Adjustment::new(AdjustKind::BrightnessContrast { brightness: -20, contrast: 35 }), Adjustment { kind: AdjustKind::Subject { fine: panel % 3 == 0 }, on: false }],
                        2 => vec![Adjustment::new(AdjustKind::Levels { black: 10, white: 240, gamma: 125 }), Adjustment::new(AdjustKind::ReduceColors { colors: 6 }), Adjustment::new(AdjustKind::ClearBackground { tolerance: 33 })],
                        _ => vec![Adjustment::new(AdjustKind::Invert), Adjustment::new(AdjustKind::FlattenShading), Adjustment::new(AdjustKind::HueSaturation { hue: -45, saturation: 20, lightness: -5 })],
                    },
                    mask: (panel % 3 == 1).then(|| Mask { w: 3 + panel as u32, h: 2, data: Arc::new((0..(3 + panel) * 2).map(|i| (i * 37) as u8).collect()), invert: panel % 2 == 0, enabled: panel % 5 != 1 }),
                    depth: 0,
                    clipped: panel % 2 == 1,
                } }
            })
            .collect::<Vec<_>>();
        // 調整レイヤーと、グループ（中身は図形と調整レイヤー）。グループの中身は、見出しのすぐ前に続ける。
        let mut decals = decals;
        let nowhere = Decal { panel: 0, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 };
        let shape_spec = ShapeSpec { kind: ShapeKind::Rect, fill: Some([1, 2, 3]), stroke: None, stroke_width: 1.0, radius: 0.0 };
        decals.push(Placement { decal: nowhere, back: false, source: Source::Adjustment, layer: Layer { adjust: vec![
            Adjustment::new(AdjustKind::HueSaturation { hue: 30, saturation: -100, lightness: 0 }),
            Adjustment::new(AdjustKind::Curves { points: [[0, 10], [64, 50], [190, 220], [255, 250], [0, 0], [0, 0], [0, 0], [0, 0]], count: 4 }),
            Adjustment::new(AdjustKind::Exposure { exposure: 150, offset: -20, gamma: 90 }),
            Adjustment { kind: AdjustKind::Vibrance { vibrance: 40, saturation: -10 }, on: false },
            Adjustment::new(AdjustKind::ColorBalance { shadows: [10, -5, 0], midtones: [0, 20, -30], highlights: [-100, 0, 100], preserve: true }),
            Adjustment::new(AdjustKind::BlackWhite { weights: [10, 20, -30, 40, 250, -200] }),
            Adjustment::new(AdjustKind::PhotoFilter { color: [12, 34, 56], density: 60, preserve: false }),
            Adjustment::new(AdjustKind::Posterize { levels: 5 }),
            Adjustment::new(AdjustKind::Threshold { level: 77 }),
            Adjustment::new(AdjustKind::GradientMap { from: [1, 2, 3], to: [250, 200, 100], reverse: true }),
        ], opacity: 50, blend: Blend::Multiply, ..Layer::default() } });
        decals.push(Placement { decal: Decal { panel: 1, at: [0.5, 0.5], size: [20.0, 10.0], angle: 0.0 }, back: false, source: Source::Shape(shape_spec), layer: Layer { depth: 2, ..Layer::default() } });
        decals.push(Placement { decal: nowhere, back: false, source: Source::Group { open: true }, layer: Layer { depth: 1, blend: Blend::Screen, ..Layer::default() } });
        decals.push(Placement { decal: nowhere, back: false, source: Source::Adjustment, layer: Layer { depth: 1, clipped: true, adjust: vec![Adjustment::new(AdjustKind::Invert)], ..Layer::default() } });
        decals.push(Placement { decal: nowhere, back: false, source: Source::Group { open: false }, layer: Layer { name: Some("グループ".to_owned()), blend: Blend::PassThrough, opacity: 80, ..Layer::default() } });
        decals.push(Placement { decal: nowhere, back: true, source: Source::Group { open: true }, layer: Layer::default() });
        tidy_groups(&mut decals);
        // 展開図まるごとの絵は、型によって、あるものとないもの。素材と同じ中身のこともある。
        let base = match kind {
            Kind::TuckBox => Some(Asset::new("型紙に描いた絵.psd", fake_file(4, 5000), [0.0, 0.0], "2026-11-22T09:00:00Z")),
            Kind::Gable => Some(Asset::new("同じ中身.psd", fake_file(2, 9000), [0.0, 0.0], "")),
            _ => None,
        };
        let back_base = matches!(kind, Kind::Prism(_)).then(|| Asset::new("裏に描いた絵.psd", fake_file(5, 4000), [0.0, 0.0], "2026-11-22T09:01:00Z"));
        let log = vec![
            LogEntry { time: "2026-11-24T12:00:00+09:00".to_owned(), actor: "person".to_owned(), command: "layer.blend.set".to_owned(), args: serde_json::json!({ "ids": [1, 2], "blend": "multiply" }) },
            LogEntry { time: "2026-11-24T12:01:00+09:00".to_owned(), actor: "agent:claude".to_owned(), command: "layer.visible".to_owned(), args: serde_json::Value::Null },
        ];
        Project { shape, material: Material { paper: Paper::Kraft, coat: Coat::GlossPp }, finishes: vec![Finish { id: 900, name: Some("箔".to_owned()), hidden: true, wave: 0, ..Finish::foil(Foil::Hologram, 1) }, Finish { id: 901, height: 0.8, edge: 1.5, ..Finish::emboss(1, true) }, Finish { id: 902, ..Finish::spot_uv(1, true) }, Finish { id: 903, ..Finish::white(1) }], assets, decals, base, back_base, log, versions: Vec::new() }
    }

    fn bytes_of(project: &Project) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        project.write(&mut out).expect("書ける");
        out.into_inner()
    }

    fn reopen(project: &Project) -> Opened {
        Project::read(Cursor::new(bytes_of(project))).expect("読める")
    }

    /// 設定のテキストだけを書き換えた案件のファイルを作る。
    fn with_manifest(project: &Project, edit: impl FnOnce(String) -> String) -> Vec<u8> {
        let mut zip = ZipArchive::new(Cursor::new(bytes_of(project))).unwrap();
        let mut text = String::new();
        zip.by_name(MANIFEST).unwrap().read_to_string(&mut text).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).filter(|n| n != MANIFEST).collect();
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        out.start_file(MANIFEST, SimpleFileOptions::default()).unwrap();
        out.write_all(edit(text).as_bytes()).unwrap();
        for name in names {
            let mut bytes = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
            out.start_file(name, SimpleFileOptions::default()).unwrap();
            out.write_all(&bytes).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// ZIP の中の 1 つのファイルを、別の中身に入れ替える。
    fn with_file(bytes: &[u8], target: &str, content: &[u8]) -> Vec<u8> {
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            let mut data = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut data).unwrap();
            out.start_file(name.as_str(), SimpleFileOptions::default()).unwrap();
            out.write_all(if name == target { content } else { &data }).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    const KINDS: [Kind; 6] = [Kind::TuckBox, Kind::Gable, Kind::Pillow, Kind::Prism(3), Kind::Prism(6), Kind::Prism(8)];

    #[test]
    fn 保存して開くと_同じ状態に戻る() {
        for kind in KINDS {
            let project = sample(kind);
            let opened = reopen(&project);
            assert!(opened.problems.is_empty(), "{kind:?}: {:?}", opened.problems);
            assert_eq!(opened.project, project, "{kind:?}");
        }
    }

    #[test]
    fn 入れ子の深さとクリップは_決まりに合うように直す() {
        // 下から: 柄(深さ2), 見出し(1), 柄(1), 見出し(0), 柄(1。開いているグループがない), 柄(クリップ)
        let groups = [false, true, false, true, false, false];
        let backs = [false; 6];
        let mut depth = [2, 1, 1, 0, 1, 0];
        let mut clipped = [false, false, true, false, false, true];
        nest_rule(&groups, &backs, &mut depth, &mut clipped);
        assert_eq!(depth, [2, 1, 1, 0, 0, 0]);
        // 柄(1) は、下の見出し(1) と同じ深さなのでクリップできる。いちばん上の柄は、下の柄(0) でクリップできる。
        assert_eq!(clipped, [false, false, true, false, false, true]);
        // いちばん下や、グループの中のいちばん下はクリップできない。見出しはクリップできない。
        let groups = [false, true, false];
        let mut depth = [1, 0, 0];
        let mut clipped = [true, true, false];
        nest_rule(&groups, &[false; 3], &mut depth, &mut clipped);
        assert_eq!(clipped, [false, false, false]);
    }

    #[test]
    fn 知らない調整とレイヤーの種類は_外して知らせる() {
        let project = sample(Kind::TuckBox);
        let bytes = with_manifest(&project, |text| text.replacen("kind = \"invert\"", "kind = \"magic\"", 1).replacen("kind = \"adjustment\"", "kind = \"sparkle\"", 1));
        let opened = Project::read(Cursor::new(bytes)).unwrap();
        assert!(opened.problems.iter().any(|p| p.contains("magic")), "{:?}", opened.problems);
        assert!(opened.problems.iter().any(|p| p.contains("sparkle")), "{:?}", opened.problems);
        assert_eq!(opened.project.decals.len(), project.decals.len() - 1);
    }

    #[test]
    fn 柄も素材もない案件を_保存して開ける() {
        let project = Project { shape: shape(Kind::TuckBox), material: Material::default(), finishes: Vec::new(), assets: Vec::new(), decals: Vec::new(), base: None, back_base: None, log: Vec::new(), versions: Vec::new() };
        let opened = reopen(&project);
        assert!(opened.problems.is_empty());
        assert_eq!(opened.project, project);
    }

    #[test]
    fn 同じ案件は_いつ書いても同じバイト列になる() {
        let project = sample(Kind::Gable);
        assert_eq!(bytes_of(&project), bytes_of(&project.clone()));
    }

    #[test]
    fn 書いた数は_読み戻すと必ず元の値に戻る() {
        for x in [0.0f32, -0.0, 0.1, 1.0 / 3.0, std::f32::consts::PI, 1e-7, 123_456.79, f32::MIN_POSITIVE, f32::MAX, -f32::MAX, 1.0e-45] {
            assert_eq!((tidy(x) as f32).to_bits(), x.to_bits(), "{x}");
        }
        // いろいろなビットの並びで確かめる。
        let mut state = 12345u32;
        for _ in 0..300_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = f32::from_bits(state);
            if x.is_finite() {
                assert_eq!((tidy(x) as f32).to_bits(), x.to_bits(), "{x}");
            }
        }
        // 短い書き方になっている。
        assert_eq!(tidy(0.1), 0.1);
        assert_eq!(tidy(46.0), 46.0);
    }

    #[test]
    fn パネルは_名前と何番目かで_行き来できる() {
        for kind in KINDS {
            let tpl = shape(kind).template();
            for index in 0..tpl.panels.len() {
                let (name, nth) = panel_ref(&tpl, index);
                assert_eq!(panel_index(&tpl, &name, nth), Some(index), "{kind:?} {name} {nth}");
            }
            assert_eq!(panel_index(&tpl, "ないパネル", 1), None);
            assert_eq!(panel_index(&tpl, tpl.panels[0].name, 0), None);
            assert_eq!(panel_index(&tpl, tpl.panels[0].name, 99), None);
        }
    }

    #[test]
    fn 同じ中身の素材は_ファイルの中に1つだけ入る() {
        let mut project = sample(Kind::TuckBox);
        project.assets.push(Asset::new("別の名前.png", fake_file(1, 3000), [20.0, 20.0], ""));
        assert_eq!(project.assets[0].id(), project.assets[3].id());
        let bytes = bytes_of(&project);
        let zip = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        // 設定のテキストと、中身の違う素材 3 つと、展開図の絵（マスクと操作の記録のファイルは数えない）。
        assert_eq!(zip.file_names().filter(|n| { let n = n.as_ref().unwrap(); !n.starts_with("masks/") && !n.starts_with("history/") }).count(), 5);
        let opened = Project::read(Cursor::new(bytes)).unwrap();
        assert!(opened.problems.is_empty());
        assert_eq!(opened.project, project);
        // 中身は、1 つを共有している。
        assert!(Arc::ptr_eq(opened.project.assets[0].bytes(), opened.project.assets[3].bytes()));
    }

    #[test]
    fn 素材の印は_中身で決まる() {
        let a = Asset::new("a.png", b"abc".to_vec(), [1.0, 1.0], "");
        // SHA-256("abc")
        assert_eq!(a.id(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(a.file(), format!("assets/{}.png", a.id()));
        assert!(is_asset_file(&a.file()));
        assert_eq!(Asset::new("名前だけ", Vec::new(), [1.0, 1.0], "").file().len(), "assets/".len() + 64);
        assert_eq!(extension("a.tar.GZ").as_deref(), Some("gz"));
        assert_eq!(extension("変な.拡張子"), None);
        assert_eq!(extension("../../evil.toolongext"), None);
        for bad in ["assets/../project.toml", "assets/abc.png", "project.toml", "assets/", "/etc/passwd"] {
            assert!(!is_asset_file(bad), "{bad}");
        }
    }

    #[test]
    fn 案件でないものは_案件でないと分かる() {
        // ZIP でない。
        assert!(matches!(Project::read(Cursor::new("PNG ではないが、ZIP でもない".as_bytes().to_vec())), Err(Error::NotProject)));
        assert!(matches!(Project::read(Cursor::new(Vec::new())), Err(Error::NotProject)));
        // ZIP だが、設定のテキストがない。
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("word/document.xml", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"<xml/>").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(matches!(Project::read(Cursor::new(bytes)), Err(Error::NotProject)));
        // 設定のテキストはあるが、印が違う。
        let other = with_manifest(&sample(Kind::TuckBox), |text| text.replace(FORMAT, "something-else"));
        assert!(matches!(Project::read(Cursor::new(other)), Err(Error::NotProject)));
    }

    #[test]
    fn 新しい版の案件は_開かずに知らせる() {
        let newer = with_manifest(&sample(Kind::TuckBox), |text| text.replace("version = 1", "version = 2"));
        match Project::read(Cursor::new(newer)) {
            Err(Error::Newer(2)) => {}
            other => panic!("{other:?}"),
        }
        let zero = with_manifest(&sample(Kind::TuckBox), |text| text.replace("version = 1", "version = 0"));
        assert!(matches!(Project::read(Cursor::new(zero)), Err(Error::Broken(_))));
    }

    #[test]
    fn 知らない項目があっても開ける() {
        // 同じ版のまま、あとから項目を足しても、いまのパケノバで開ける。
        let project = sample(Kind::TuckBox);
        let more = with_manifest(&project, |text| format!("{text}\n[view]\nzoom = 2.0\n").replacen("[shape]", "[shape]\ncolor = \"white\"", 1));
        let opened = Project::read(Cursor::new(more)).unwrap();
        assert!(opened.problems.is_empty());
        assert_eq!(opened.project, project);
    }

    #[test]
    fn 設定のテキストが壊れていたら_壊れていると分かる() {
        let project = sample(Kind::TuckBox);
        for edit in [
            (|text: String| text.replace("[shape]", "[shape")) as fn(String) -> String,
            |text| text.replace("kind = \"tuck_box\"", "kind = \"mystery\""),
            |text| text.replace("width = 200.0", "width = \"広い\""),
            |text| text.replace("width = 200.0", "width = nan"),
            |text| text.replace("version = 1", "version = \"1\""),
        ] {
            match Project::read(Cursor::new(with_manifest(&project, edit))) {
                Err(Error::Broken(_)) => {}
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn 範囲の外の寸法は_範囲に寄せて知らせる() {
        let project = sample(Kind::TuckBox);
        let wide = with_manifest(&project, |text| text.replace("width = 200.0", "width = 5000.0"));
        let opened = Project::read(Cursor::new(wide)).unwrap();
        assert_eq!(opened.project.shape.w, 300.0);
        assert_eq!(opened.problems.len(), 1);
        assert!(opened.problems[0].contains("Width"), "{:?}", opened.problems);
        // 柄は、そのまま残る。
        assert_eq!(opened.project.decals, project.decals);
    }

    #[test]
    fn 差し込み箱のフラップは_書いていなければ自動で_おかしな値は直して知らせる() {
        let project = sample(Kind::TuckBox);
        let edited = with_manifest(&project, |text| text.replace("glue_flap = 9.5\n", "").replace("tuck_flap = 22.25", "tuck_flap = 500.0"));
        let opened = Project::read(Cursor::new(edited)).unwrap();
        assert_eq!(opened.project.shape.flaps, TuckFlaps { glue: None, dust: None, tuck: Some(68.0) });
        assert_eq!(opened.problems.len(), 1, "{:?}", opened.problems);
        assert!(opened.problems[0].contains("Tuck length"), "{:?}", opened.problems);
        let broken = with_manifest(&project, |text| text.replace("glue_flap = 9.5", "glue_flap = nan"));
        let opened = Project::read(Cursor::new(broken)).unwrap();
        assert_eq!(opened.project.shape.flaps.glue, None);
        assert!(opened.problems[0].contains("Glue flap width"), "{:?}", opened.problems);
    }

    #[test]
    fn 素材が欠けていたら_その素材と柄だけを外して開く() {
        let project = sample(Kind::TuckBox);
        // 素材のファイルを 1 つ抜いた ZIP を作る。
        let gone = project.assets[0].file();
        let mut zip = ZipArchive::new(Cursor::new(bytes_of(&project))).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).filter(|n| *n != gone).collect();
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            let mut bytes = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
            out.start_file(name, SimpleFileOptions::default()).unwrap();
            out.write_all(&bytes).unwrap();
        }
        let opened = Project::read(Cursor::new(out.finish().unwrap().into_inner())).unwrap();

        let used = project.decals.iter().filter(|d| d.source == Source::Asset(0)).count();
        assert!(used > 0);
        // 素材 1 つと、それを使う柄のぶんだけ、知らせが出る。
        assert_eq!(opened.problems.len(), 1 + used, "{:?}", opened.problems);
        assert!(opened.problems[0].contains("ロゴ 最終版.png"));
        assert_eq!(opened.project.assets.len(), 2);
        assert_eq!(opened.project.decals.len(), project.decals.len() - used);
        // 残った柄は、番号が詰まっても、同じ素材を指している。
        for placed in &opened.project.decals {
            if let Source::Asset(index) = placed.source {
                let before = project.decals.iter().find(|d| d.decal == placed.decal).unwrap();
                let Source::Asset(old) = before.source else { panic!() };
                assert_eq!(opened.project.assets[index], project.assets[old]);
            }
        }
    }

    #[test]
    fn 操作の記録は_1行1件で入り_開くと戻り_古いものは捨てる() {
        let mut project = sample(Kind::TuckBox);
        let opened = Project::read(Cursor::new(bytes_of(&project))).unwrap();
        assert_eq!(opened.project.log, project.log);
        assert!(opened.problems.is_empty(), "{:?}", opened.problems);
        let entry = project.log[0].clone();
        project.log = vec![entry; LOG_LIMIT + 5];
        project.log[5].command = "first.kept".to_owned();
        let opened = Project::read(Cursor::new(bytes_of(&project))).unwrap();
        assert_eq!(opened.project.log.len(), LOG_LIMIT);
        assert_eq!(opened.project.log[0].command, "first.kept");
        // 記録がなければ、ファイルも作らない。
        project.log.clear();
        let bytes = bytes_of(&project);
        assert!(!ZipArchive::new(Cursor::new(&bytes)).unwrap().file_names().any(|n| n.as_ref().unwrap().starts_with("history/")));
    }

    #[test]
    fn 版は_素材を共有して入り_開くと戻る() {
        let mut project = sample(Kind::TuckBox);
        let mut older = project.clone();
        older.log.clear();
        older.decals.truncate(1);
        project.versions = vec![
            Version { name: "はじめ".to_owned(), time: "2026-11-24T14:00:00+09:00".to_owned(), auto: false, project: older.clone() },
            Version { name: "依頼の前".to_owned(), time: "2026-11-24T14:05:00+09:00".to_owned(), auto: true, project: older },
        ];
        let bytes = bytes_of(&project);
        let opened = Project::read(Cursor::new(&bytes)).unwrap();
        assert!(opened.problems.is_empty(), "{:?}", opened.problems);
        let versions = &opened.project.versions;
        assert_eq!(versions.len(), 2);
        assert_eq!((versions[0].name.as_str(), versions[0].auto, versions[1].auto), ("はじめ", false, true));
        assert_eq!(versions[0].project.decals.len(), 1);
        assert_eq!(versions[0].project.assets.len(), project.assets.len());
        assert_eq!(opened.project.decals.len(), project.decals.len());
        // 素材のファイルは、版があっても増えない。
        let without = Project { versions: Vec::new(), ..project.clone() };
        let count = |b: &[u8]| ZipArchive::new(Cursor::new(b)).unwrap().file_names().filter(|n| n.as_ref().unwrap().starts_with("assets/")).count();
        assert_eq!(count(&bytes), count(&bytes_of(&without)));
        // 読めない版は外して知らせ、案件は開く。
        let broken = with_file(&bytes, "versions/001.toml", b"format = 1");
        let opened = Project::read(Cursor::new(&broken)).unwrap();
        assert_eq!(opened.project.versions.len(), 1);
        assert_eq!(opened.problems.len(), 1);
    }

    #[test]
    fn 調整はjsonと行き来できる() {
        let adjust = Adjustment::new(AdjustKind::Levels { black: 10, white: 240, gamma: 125 });
        let json = adjustment_to_json(&adjust);
        assert_eq!(json["kind"], "levels");
        assert_eq!(adjustment_from_json(&json).unwrap(), adjust);
        assert!(adjustment_from_json(&serde_json::json!({ "kind": "nope" })).is_err());
    }

    #[test]
    fn 展開図の絵も_保存して開ける() {
        // 素材と同じ中身の絵は、ファイルの中に 1 つだけ入り、開くと両方に戻る。
        let project = sample(Kind::Gable);
        let bytes = bytes_of(&project);
        assert_eq!(ZipArchive::new(Cursor::new(&bytes)).unwrap().file_names().filter(|n| { let n = n.as_ref().unwrap(); !n.starts_with("masks/") && !n.starts_with("history/") }).count(), 4);
        let opened = Project::read(Cursor::new(bytes)).unwrap();
        assert!(opened.problems.is_empty());
        assert_eq!(opened.project.base, project.base);
        assert!(Arc::ptr_eq(opened.project.base.as_ref().unwrap().bytes(), opened.project.assets[1].bytes()));

        // 絵のファイルが欠けていたら、絵だけを外して開く。柄と素材は残る。
        let project = sample(Kind::TuckBox);
        let gone = project.base.as_ref().unwrap().file();
        let mut zip = ZipArchive::new(Cursor::new(bytes_of(&project))).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).filter(|n| *n != gone).collect();
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            let mut bytes = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
            out.start_file(name, SimpleFileOptions::default()).unwrap();
            out.write_all(&bytes).unwrap();
        }
        let opened = Project::read(Cursor::new(out.finish().unwrap().into_inner())).unwrap();
        assert_eq!(opened.problems.len(), 1, "{:?}", opened.problems);
        assert!(opened.problems[0].contains("型紙に描いた絵.psd"));
        assert_eq!(opened.project.base, None);
        assert_eq!(opened.project.decals, project.decals);
        assert_eq!(opened.project.assets, project.assets);
    }

    #[test]
    fn 素材の中身が傷んでいたら_その素材を外して開く() {
        let project = sample(Kind::TuckBox);
        let mut bytes = bytes_of(&project);
        // 圧縮しないで入れた PNG の中身を、1 バイトだけ変える。
        let needle = &project.assets[0].bytes()[100..132];
        let at = bytes.windows(needle.len()).position(|w| w == needle).expect("そのまま入っている");
        bytes[at] ^= 0xff;
        let opened = Project::read(Cursor::new(bytes)).unwrap();
        assert!(opened.problems.iter().any(|p| p.contains("ロゴ 最終版.png")), "{:?}", opened.problems);
        assert_eq!(opened.project.assets.len(), 2);
    }

    #[test]
    fn 正しくない柄は_その柄だけを外して開く() {
        let project = sample(Kind::TuckBox);
        let edited = with_manifest(&project, |text| {
            text.replacen("panel = \"正面\"", "panel = \"どこにもない面\"", 1).replacen("asset = 1", "asset = 99", 1).replacen("builtin = \"band\"", "builtin = \"band\"\nasset = 0", 1)
        });
        let opened = Project::read(Cursor::new(edited)).unwrap();
        assert_eq!(opened.problems.len(), 3, "{:?}", opened.problems);
        assert_eq!(opened.project.decals.len(), project.decals.len() - 3);
        assert_eq!(opened.project.assets, project.assets);
    }

    #[test]
    fn 書けない内容は_書く前に断る() {
        let mut project = sample(Kind::TuckBox);
        project.decals[0].decal.at[0] = f32::NAN;
        assert!(matches!(project.write(Cursor::new(Vec::new())), Err(Error::Broken(_))));
        let mut project = sample(Kind::TuckBox);
        project.decals[0].decal.panel = 999;
        assert!(matches!(project.write(Cursor::new(Vec::new())), Err(Error::Broken(_))));
        let mut project = sample(Kind::TuckBox);
        project.decals[2].source = Source::Asset(3);
        assert!(matches!(project.write(Cursor::new(Vec::new())), Err(Error::Broken(_))));
        let mut project = sample(Kind::TuckBox);
        project.shape.w = f32::INFINITY;
        assert!(matches!(project.write(Cursor::new(Vec::new())), Err(Error::Broken(_))));
    }

    #[test]
    fn 途中で切れたファイルや_傷んだファイルで_落ちない() {
        let bytes = bytes_of(&sample(Kind::Gable));
        // 途中で切れたファイル。
        for len in (0..bytes.len()).step_by(bytes.len() / 97 + 1) {
            let _ = Project::read(Cursor::new(bytes[..len].to_vec()));
        }
        // どこか 1 バイトが化けたファイル。
        let mut state = 7u32;
        for _ in 0..400 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mut broken = bytes.clone();
            let at = state as usize % broken.len();
            broken[at] ^= (state >> 8) as u8 | 1;
            let _ = Project::read(Cursor::new(broken));
        }
    }

    /// テストごとの、空の作業フォルダ。
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pakenoba-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn ファイルに保存して_開ける() {
        let dir = scratch("save");
        let path = dir.join("新しい箱.pknb");
        let big = sample(Kind::Gable);
        big.save(&path).unwrap();
        assert_eq!(Project::open(&path).unwrap().project, big);
        // 小さい案件で上書きしても、前の中身が後ろに残らない。
        let small = Project { shape: shape(Kind::TuckBox), material: Material::default(), finishes: Vec::new(), assets: Vec::new(), decals: Vec::new(), base: None, back_base: None, log: Vec::new(), versions: Vec::new() };
        small.save(&path).unwrap();
        assert_eq!(Project::open(&path).unwrap().project, small);
        assert_eq!(fs::read(&path).unwrap(), bytes_of(&small));
        // 書きかけのファイルは残らない。
        assert_eq!(files_in(&dir), ["新しい箱.pknb"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 保存が途中で止まっても_前のファイルが残る() {
        let dir = scratch("interrupted");
        let path = dir.join("箱.pknb");
        let project = sample(Kind::TuckBox);
        project.save(&path).unwrap();
        let before = fs::read(&path).unwrap();

        // 半分だけ書いて、失敗する。
        let half = before.len() / 2;
        let result = replace_file(&path, |file| {
            file.write_all(&before[..half])?;
            Err(Error::Io(io::Error::other("ディスクがいっぱい")))
        });
        assert!(matches!(result, Err(Error::Io(_))));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(Project::open(&path).unwrap().project, project);
        assert_eq!(files_in(&dir), ["箱.pknb"]);

        // 書けない内容の案件を保存しようとしても、前のファイルが残る。
        let mut bad = project.clone();
        bad.decals[0].decal.angle = f32::NAN;
        assert!(bad.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(files_in(&dir), ["箱.pknb"]);

        // 前に止まったときの書きかけが残っていても、開けるし、保存もできる。
        fs::write(dir.join("箱.pknb.tmp-1"), &before[..half]).unwrap();
        assert_eq!(Project::open(&path).unwrap().project, project);
        project.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ないフォルダへは保存できず_何も残さない() {
        let dir = scratch("missing");
        let path = dir.join("ないフォルダ").join("箱.pknb");
        assert!(matches!(sample(Kind::TuckBox).save(&path), Err(Error::Io(_))));
        assert!(matches!(Project::open(&path), Err(Error::Io(_))));
        assert!(files_in(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 版 1 の見本（`tests/data/v1-sample.pknb`）の中身。見本は、版 1 のパケノバが書いたファイルを、そのまま残してある。
    fn v1_sample() -> Project {
        let shape = Shape { kind: Kind::Prism(6), w: 70.0, d: 60.0, h: 160.0, t: 0.5, bulge: 40.0, roof: 45.0, flaps: TuckFlaps::default() };
        let tpl = shape.template();
        let panel = |name: &str, nth: usize| panel_index(&tpl, name, nth).unwrap();
        Project {
            material: Material::default(),
            finishes: Vec::new(),
            shape,
            assets: vec![Asset::new("ロゴ.png", fake_file(11, 600), [46.0, 46.0], "2026-11-22T00:30:00+09:00"), Asset::new("地紋.psd", fake_file(12, 2000), [120.0, 80.0], "2026-11-22T00:31:00+09:00")],
            decals: vec![
                Placement { decal: Decal { panel: panel("側面 1", 1), at: [0.5, 0.5], size: [46.0, 46.0], angle: 0.0 }, back: false, source: Source::Asset(0), layer: Layer::default() },
                Placement { decal: Decal { panel: panel("側面 3", 1), at: [0.25, 0.75], size: [120.0, 80.0], angle: std::f32::consts::FRAC_PI_2 }, back: true, source: Source::Asset(1), layer: Layer::default() },
                Placement { decal: Decal { panel: panel("上のフラップ", 3), at: [0.5, 0.4], size: [20.0, 20.0], angle: -0.3 }, back: false, source: Source::Builtin("logo".to_owned()), layer: Layer::default() },
                Placement { decal: Decal { panel: panel("側面 2", 1), at: [0.5, 0.1], size: [420.0, 14.0], angle: 0.0 }, back: false, source: Source::Builtin("band".to_owned()), layer: Layer::default() },
            ],
            base: Some(Asset::new("型紙に描いた絵.psd", fake_file(13, 1500), [0.0, 0.0], "2026-11-22T00:32:00+09:00")),
            back_base: None,
            log: Vec::new(),
            versions: Vec::new(),
        }
    }

    fn v1_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/v1-sample.pknb")
    }

    #[test]
    fn 版1の見本を_いまのパケノバで開ける() {
        let opened = Project::open(&v1_path()).expect("版 1 の見本を開ける");
        assert!(opened.problems.is_empty(), "{:?}", opened.problems);
        assert_eq!(opened.project, v1_sample());
    }

    /// 版 1 の見本を作り直す。形式を変えたときに、わざと実行するためのもの（ふだんは動かさない）。
    #[test]
    #[ignore = "版 1 の見本を上書きする"]
    fn 版1の見本を作る() {
        let path = v1_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        v1_sample().save(&path).unwrap();
    }

    /// 大きい素材を入れたときの、保存と開く時間を計る（`cargo test --release -- --ignored 大きい --nocapture`）。
    #[test]
    #[ignore = "300 MB の素材で時間を計る"]
    fn 大きい素材の_保存と開く時間を計る() {
        let dir = scratch("big");
        let path = dir.join("大きい.pknb");
        let mut project = sample(Kind::TuckBox);
        // 圧縮の効かない PNG 相当を 3 つ（200 MB）と、圧縮する PSD 相当を 1 つ（100 MB）。
        let started = std::time::Instant::now();
        for (index, (name, mb)) in [("a.png", 80), ("b.png", 70), ("c.png", 50), ("d.psd", 100)].into_iter().enumerate() {
            project.assets.push(Asset::new(name, fake_file(100 + index as u32, mb << 20), [100.0, 100.0], ""));
        }
        println!("取り込み（中身の印を作る）: {:.2} 秒", started.elapsed().as_secs_f32());
        let started = std::time::Instant::now();
        project.save(&path).unwrap();
        println!("保存: {:.2} 秒（{} MB）", started.elapsed().as_secs_f32(), fs::metadata(&path).unwrap().len() >> 20);
        let started = std::time::Instant::now();
        let opened = Project::open(&path).unwrap();
        println!("開く: {:.2} 秒", started.elapsed().as_secs_f32());
        assert!(opened.problems.is_empty());
        assert_eq!(opened.project.assets.len(), project.assets.len());
        fs::remove_dir_all(&dir).unwrap();
    }
}
