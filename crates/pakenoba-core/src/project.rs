//! 案件のファイル（`.pknb`）の読み書き。
//!
//! 案件 1 つは、1 つのファイル。中身は ZIP で、次のものが入る。
//!
//! - `project.toml`: 設定のテキスト。型と寸法、素材の一覧、置いた柄。
//! - `assets/<中身の印>.<拡張子>`: 素材のファイル。取り込んだ元のファイルのまま。
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

use crate::surface::Decal;
use crate::template::{P2, Template};

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
}

impl Shape {
    /// この型と寸法の型紙。
    pub fn template(&self) -> Template {
        match self.kind {
            Kind::TuckBox => Template::tuck_box(self.w, self.d, self.h, self.t),
            Kind::Gable => Template::gable(self.w, self.d, self.h, self.roof, self.t),
            Kind::Pillow => Template::pillow(self.w, self.h, self.t, self.bulge.to_radians()),
            Kind::Prism(n) => Template::prism(n, self.w, self.h, self.t),
        }
    }

    fn values(&mut self) -> [(&'static str, &mut f32, RangeInclusive<f32>); 6] {
        [("幅", &mut self.w, WIDTH), ("奥行き", &mut self.d, DEPTH), ("高さ", &mut self.h, HEIGHT), ("紙の厚み", &mut self.t, THICKNESS), ("ふくらみ", &mut self.bulge, BULGE), ("屋根の高さ", &mut self.roof, ROOF)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// 付属の見本。値は見本の名前（どんな見本があるかは、アプリの側が決める）。
    Builtin(String),
    /// この案件の素材。値は `Project::assets` の中での番号。
    Asset(usize),
}

/// 置いた柄 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// 場所と大きさ。`panel` は、`Project::shape` の型紙でのパネルの番号。
    pub decal: Decal,
    /// 紙の裏（箱の内側の面）に置いた柄か。
    pub back: bool,
    pub source: Source,
}

/// 案件 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub shape: Shape,
    pub assets: Vec<Asset>,
    /// 置いた柄。並びは重なりの順（後ろのものが上）。
    pub decals: Vec<Placement>,
}

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
            Error::Io(e) => write!(f, "ファイルを読み書きできませんでした: {e}"),
            Error::NotProject => write!(f, "パケノバの案件のファイルではありません"),
            Error::Newer(found) => write!(f, "新しい版のパケノバで保存された案件です（形式の版 {found}。このパケノバが読めるのは版 {VERSION} まで）"),
            Error::Broken(what) => write!(f, "案件のファイルが壊れています: {what}"),
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
    #[serde(rename = "asset", default, skip_serializing_if = "Vec::is_empty")]
    assets: Vec<AssetEntry>,
    #[serde(rename = "decal", default, skip_serializing_if = "Vec::is_empty")]
    decals: Vec<DecalEntry>,
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
struct DecalEntry {
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
            return Err(Error::Broken(format!("寸法の「{label}」が数ではありません")));
        }
        for (index, asset) in self.assets.iter().enumerate() {
            if !asset.size.iter().all(|v| v.is_finite()) {
                return Err(Error::Broken(format!("素材 {}（{}）の大きさが数ではありません", index + 1, asset.name)));
            }
        }
        for (index, placed) in self.decals.iter().enumerate() {
            let number = index + 1;
            let decal = &placed.decal;
            if decal.panel >= tpl.panels.len() {
                return Err(Error::Broken(format!("柄 {number} の置き場所が、型紙にないパネルを指しています")));
            }
            if ![decal.at[0], decal.at[1], decal.size[0], decal.size[1], decal.angle].iter().all(|v| v.is_finite()) {
                return Err(Error::Broken(format!("柄 {number} の場所か大きさが数ではありません")));
            }
            if let Source::Asset(asset) = placed.source
                && asset >= self.assets.len()
            {
                return Err(Error::Broken(format!("柄 {number} が、ない素材を指しています")));
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
            shape: ShapeEntry { kind: kind.to_owned(), sides, width: tidy(s.w), depth: tidy(s.d), height: tidy(s.h), thickness: tidy(s.t), bulge: tidy(s.bulge), roof: tidy(s.roof) },
            assets: self.assets.iter().map(|a| AssetEntry { name: a.name.clone(), file: a.file(), imported: a.imported.clone(), size: tidy2(a.size) }).collect(),
            decals: self
                .decals
                .iter()
                .map(|placed| {
                    let (panel, nth) = panel_ref(tpl, placed.decal.panel);
                    let (asset, builtin) = match &placed.source {
                        Source::Asset(index) => (Some(*index), None),
                        Source::Builtin(name) => (None, Some(name.clone())),
                    };
                    DecalEntry { asset, builtin, panel, nth, at: tidy2(placed.decal.at), size: tidy2(placed.decal.size), angle: tidy(placed.decal.angle), back: placed.back }
                })
                .collect(),
        }
    }

    /// 案件を、案件のファイルの形で書き出す。
    pub fn write<W: Write + Seek>(&self, out: W) -> Result<(), Error> {
        let tpl = self.shape.template();
        self.check(&tpl)?;
        let text = toml::to_string(&self.manifest(&tpl)).map_err(|e| Error::Broken(format!("設定のテキストを作れませんでした: {e}")))?;

        let mut zip = ZipWriter::new(out);
        zip.start_file(MANIFEST, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zip_error)?;
        zip.write_all(text.as_bytes())?;
        let mut written: Vec<String> = Vec::new();
        for asset in &self.assets {
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
        let mut out = zip.finish().map_err(zip_error)?;
        out.flush()?;
        Ok(())
    }

    /// 案件のファイルの形から読む。
    pub fn read<R: Read + Seek>(input: R) -> Result<Opened, Error> {
        let mut zip = ZipArchive::new(input).map_err(|e| match e {
            zip::result::ZipError::Io(e) => Error::Io(e),
            _ => Error::NotProject,
        })?;
        let text = {
            let entry = zip.by_name(MANIFEST).map_err(|e| match e {
                zip::result::ZipError::Io(e) => Error::Io(e),
                _ => Error::NotProject,
            })?;
            let mut bytes = Vec::new();
            entry.take(MANIFEST_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| Error::Broken(format!("設定のテキストを取り出せませんでした: {e}")))?;
            if bytes.len() as u64 > MANIFEST_LIMIT {
                return Err(Error::Broken("設定のテキストが大きすぎます".to_owned()));
            }
            String::from_utf8(bytes).map_err(|_| Error::Broken("設定のテキストの文字が読めません".to_owned()))?
        };
        let table: toml::Table = toml::from_str(&text).map_err(|e| Error::Broken(format!("設定のテキストを読めませんでした: {}", e.message())))?;
        if table.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
            return Err(Error::NotProject);
        }
        let version = table.get("version").and_then(|v| v.as_integer()).ok_or_else(|| Error::Broken("形式の版が書かれていません".to_owned()))?;
        match u32::try_from(version) {
            Ok(1) => from_v1(table, &mut zip),
            Ok(found) if found > VERSION => Err(Error::Newer(found)),
            _ => Err(Error::Broken(format!("形式の版（{version}）が正しくありません"))),
        }
    }

    /// 案件を、ファイルに保存する。別の名前で書ききってから入れ替えるので、途中で止まっても、前のファイルが残る。
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        replace_file(path, |file| self.write(BufWriter::with_capacity(1 << 20, file)))
    }

    /// 案件のファイルを開く。
    pub fn open(path: &Path) -> Result<Opened, Error> {
        Project::read(BufReader::with_capacity(1 << 20, File::open(path)?))
    }
}

fn zip_error(e: zip::result::ZipError) -> Error {
    match e {
        zip::result::ZipError::Io(e) => Error::Io(e),
        other => Error::Broken(format!("ZIP として書けませんでした: {other}")),
    }
}

/// 版 1 の設定のテキストから、案件を組み立てる。
fn from_v1<R: Read + Seek>(table: toml::Table, zip: &mut ZipArchive<R>) -> Result<Opened, Error> {
    let manifest: Manifest = table.try_into().map_err(|e: toml::de::Error| Error::Broken(format!("設定のテキストを読めませんでした: {}", e.message())))?;
    let mut problems = Vec::new();

    // 型と寸法。
    let entry = &manifest.shape;
    let kind = match (entry.kind.as_str(), entry.sides) {
        ("tuck_box", _) => Kind::TuckBox,
        ("gable", _) => Kind::Gable,
        ("pillow", _) => Kind::Pillow,
        ("prism", Some(n)) if SIDES.contains(&n) => Kind::Prism(n),
        ("prism", _) => return Err(Error::Broken("角柱の角の数が正しくありません".to_owned())),
        (other, _) => return Err(Error::Broken(format!("このパケノバが知らない型（{other}）です"))),
    };
    let mut shape = Shape { kind, w: entry.width as f32, d: entry.depth as f32, h: entry.height as f32, t: entry.thickness as f32, bulge: entry.bulge as f32, roof: entry.roof as f32 };
    for (label, value, range) in shape.values() {
        if !value.is_finite() {
            return Err(Error::Broken(format!("寸法の「{label}」が数ではありません")));
        }
        if !range.contains(value) {
            let fixed = value.clamp(*range.start(), *range.end());
            problems.push(format!("寸法の「{label}」が範囲の外（{value}）だったので、{fixed} にしました"));
            *value = fixed;
        }
    }
    let tpl = shape.template();

    // 素材。読めなかったものは外して、番号の対応を覚えておく。
    let mut assets: Vec<Asset> = Vec::new();
    let mut kept: Vec<Option<usize>> = Vec::new();
    let mut loaded: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    for entry in &manifest.assets {
        let size = [entry.size[0] as f32, entry.size[1] as f32];
        let bytes = if !is_asset_file(&entry.file) || !size.iter().all(|v| v.is_finite()) {
            Err("書かれている内容が正しくありません".to_owned())
        } else if let Some(bytes) = loaded.get(&entry.file) {
            Ok(bytes.clone())
        } else {
            read_entry(zip, &entry.file).map(|bytes| {
                let bytes = Arc::new(bytes);
                loaded.insert(entry.file.clone(), bytes.clone());
                bytes
            })
        };
        match bytes {
            Ok(bytes) => {
                // 中身の印は、ファイルの名前から取る（中身の傷は、ZIP の検査で見つかる）。
                let id = entry.file["assets/".len()..].split('.').next().unwrap_or_default().to_owned();
                kept.push(Some(assets.len()));
                assets.push(Asset { name: entry.name.clone(), imported: entry.imported.clone(), size, id, bytes });
            }
            Err(why) => {
                kept.push(None);
                problems.push(format!("素材「{}」を読めなかったので、外しました（{why}）", entry.name));
            }
        }
    }

    // 柄。
    let mut decals = Vec::new();
    for (index, entry) in manifest.decals.iter().enumerate() {
        let number = index + 1;
        let source = match (entry.asset, &entry.builtin) {
            (Some(asset), None) => match kept.get(asset) {
                Some(Some(now)) => Source::Asset(*now),
                Some(None) => {
                    problems.push(format!("柄 {number} は、素材「{}」を読めなかったので、外しました", manifest.assets[asset].name));
                    continue;
                }
                None => {
                    problems.push(format!("柄 {number} は、ない素材を指していたので、外しました"));
                    continue;
                }
            },
            (None, Some(name)) => Source::Builtin(name.clone()),
            _ => {
                problems.push(format!("柄 {number} は、絵の指定が正しくないので、外しました"));
                continue;
            }
        };
        let Some(panel) = panel_index(&tpl, &entry.panel, entry.nth) else {
            problems.push(format!("柄 {number} は、置き場所のパネル「{}」が型紙にないので、外しました", entry.panel));
            continue;
        };
        let decal = Decal { panel, at: [entry.at[0] as f32, entry.at[1] as f32], size: [entry.size[0] as f32, entry.size[1] as f32], angle: entry.angle as f32 };
        let numbers = [decal.at[0], decal.at[1], decal.size[0], decal.size[1], decal.angle];
        if !numbers.iter().all(|v| v.is_finite()) || decal.size[0] <= 0.0 || decal.size[1] <= 0.0 {
            problems.push(format!("柄 {number} は、場所か大きさが正しくないので、外しました"));
            continue;
        }
        decals.push(Placement { decal, back: entry.back, source });
    }

    Ok(Opened { project: Project { shape, assets, decals }, problems })
}

/// ZIP の中のファイルを 1 つ、全部読む。中身の傷（CRC の不一致）も、ここで見つかる。
fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip.by_name(name).map_err(|e| match e {
        zip::result::ZipError::FileNotFound => "ファイルの中に見つかりません".to_owned(),
        other => format!("取り出せません: {other}"),
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|e| format!("中身が壊れています: {e}"))?;
    Ok(bytes)
}

/// ファイルを、別の名前で書ききってから、元のファイルと入れ替える。
///
/// `write` が失敗したとき、または途中でアプリが止まったときは、元のファイルはそのまま残る。
/// 書きかけのファイル（同じフォルダの `<名前>.tmp-<番号>`）は、失敗したら消す。
pub fn replace_file(path: &Path, write: impl FnOnce(&mut File) -> Result<(), Error>) -> Result<(), Error> {
    let name = path.file_name().ok_or_else(|| Error::Io(io::Error::new(io::ErrorKind::InvalidInput, "ファイルの名前がありません")))?;
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
        Shape { kind, w, d, h, t: 0.5, bulge: 70.0, roof: 45.0 }
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
                let source = match panel % 4 {
                    0 => Source::Builtin("logo".to_owned()),
                    1 => Source::Builtin("band".to_owned()),
                    n => Source::Asset(n - 2),
                };
                Placement { decal: Decal { panel, at: [0.1 + k / 37.0, 1.0 / 3.0 - k], size: [46.0 + k * 0.1, 14.0 / (k + 1.0)], angle: std::f32::consts::FRAC_PI_2 * k - 0.3 }, back: panel % 3 == 0, source }
            })
            .collect();
        Project { shape, assets, decals }
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
    fn 柄も素材もない案件を_保存して開ける() {
        let project = Project { shape: shape(Kind::TuckBox), assets: Vec::new(), decals: Vec::new() };
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
        // 設定のテキストと、中身の違う素材 3 つ。
        assert_eq!(zip.len(), 4);
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
        assert!(opened.problems[0].contains("幅"), "{:?}", opened.problems);
        // 柄は、そのまま残る。
        assert_eq!(opened.project.decals, project.decals);
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
        let small = Project { shape: shape(Kind::TuckBox), assets: Vec::new(), decals: Vec::new() };
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
        let shape = Shape { kind: Kind::Prism(6), w: 70.0, d: 60.0, h: 160.0, t: 0.5, bulge: 40.0, roof: 45.0 };
        let tpl = shape.template();
        let panel = |name: &str, nth: usize| panel_index(&tpl, name, nth).unwrap();
        Project {
            shape,
            assets: vec![Asset::new("ロゴ.png", fake_file(11, 600), [46.0, 46.0], "2026-11-22T00:30:00+09:00"), Asset::new("地紋.psd", fake_file(12, 2000), [120.0, 80.0], "2026-11-22T00:31:00+09:00")],
            decals: vec![
                Placement { decal: Decal { panel: panel("側面 1", 1), at: [0.5, 0.5], size: [46.0, 46.0], angle: 0.0 }, back: false, source: Source::Asset(0) },
                Placement { decal: Decal { panel: panel("側面 3", 1), at: [0.25, 0.75], size: [120.0, 80.0], angle: std::f32::consts::FRAC_PI_2 }, back: true, source: Source::Asset(1) },
                Placement { decal: Decal { panel: panel("上のフラップ", 3), at: [0.5, 0.4], size: [20.0, 20.0], angle: -0.3 }, back: false, source: Source::Builtin("logo".to_owned()) },
                Placement { decal: Decal { panel: panel("側面 2", 1), at: [0.5, 0.1], size: [420.0, 14.0], angle: 0.0 }, back: false, source: Source::Builtin("band".to_owned()) },
            ],
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
