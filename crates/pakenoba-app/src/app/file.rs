//! 案件のファイル（`.pknb`）を開く、保存する。
//!
//! 決まりは `docs/project-file.md`。
//!
//! - 保存は、使う人が行う（Ctrl+S）。自動では保存しない。
//! - 保存は裏で進める。素材が大きいと数秒かかるので、その間も画面を止めない。
//! - 保存していない変更があるかどうかは、状態の印（型、柄、素材、展開図の絵から作る数）を、
//!   最後に保存したときの印と比べて決める。元に戻して保存したときの状態に戻れば、変更なしに戻る。
//! - 傷んだファイルは、読める部分だけを開く。そのときは名前なしの案件として開き、
//!   元のファイルにそのまま上書きさせない。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use eframe::egui;

use pakenoba_core::project::{self, Placement, Project, Source};

use super::{App, Dims, FlatCam, SHAPE_VIEW, Workspace, assets, history};
use crate::art::make_sheet_art;
use crate::paint::{Picture, Placed};

/// 保存や確認のあとに続ける操作。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Then {
    Nothing,
    /// ウィンドウを閉じる。
    Close,
    /// 別の案件を開く。None なら、ファイルを選ぶところから。
    Open(Option<PathBuf>),
    /// 新しい案件を始める。
    New,
}

/// ファイルの操作。メニューとキーから受けて、画面を描き終えてから実行する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileAction {
    New,
    Open,
    Save,
    SaveAs,
}

/// 裏で進めている保存。
pub(super) struct Saving {
    result: Receiver<Result<(), String>>,
    path: PathBuf,
    /// 保存を始めたときの、状態の印。
    print: u64,
    then: Then,
}

/// 作業を止めて見せる知らせ（開けなかった、保存できなかった、傷んだ部分を外して開いた）。
pub(super) struct Notice {
    title: String,
    lines: Vec<String>,
}

/// 案件のファイルにかかわる状態。
pub(super) struct FileState {
    /// 案件のファイルの場所。まだ保存していなければ None。
    pub path: Option<PathBuf>,
    /// 最後に保存した（または開いた）ときの、状態の印。
    saved: u64,
    saving: Option<Saving>,
    /// 保存していない変更があるときに、確認を待っている操作。
    pending: Option<Then>,
    notice: Option<Notice>,
    /// メニューやキーで頼まれた操作。
    pub action: Option<FileAction>,
    /// 最近使った案件の一覧から選んだもの。
    recent: Option<PathBuf>,
    /// ウィンドウを閉じてよい（確認が済んだ）。
    allow_close: bool,
    /// いまウィンドウに出している題。
    title: String,
    /// 名前を付けて保存するときに、はじめに入れておく名前。
    suggest: String,
    /// 画面を撮って確かめるとき用。ファイルを選ぶダイアログを出さずに、この場所を答えにする。
    answer_open: Option<PathBuf>,
    answer_save: Option<PathBuf>,
}

impl FileState {
    pub fn new(answer_open: Option<PathBuf>, answer_save: Option<PathBuf>) -> FileState {
        FileState { path: None, saved: 0, saving: None, pending: None, notice: None, action: None, recent: None, allow_close: false, title: String::new(), suggest: UNTITLED.to_owned(), answer_open, answer_save }
    }
}

const UNTITLED: &str = "名前なし";

/// 最近使った案件を、いくつ覚えておくか。
const RECENT: usize = 10;

/// Windows の canonicalize が付ける `\\?\` を外す（画面に出すときと、ダイアログに渡すときに読みにくいため）。
pub(super) fn simplify(path: &Path) -> PathBuf {
    let text = path.display().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path.to_owned(),
    }
}

/// いまの日時を、その土地の時刻と時差つきで（`2026-11-22T09:46:42+09:00`）。素材を取り込んだ日時として覚える。
pub(super) fn now_local() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// 覚えている日時を、画面に出す形（その土地の時刻、`2026-11-22 09:46`）にする。読めなければ None。
pub(super) fn show_time(stamp: &str) -> Option<String> {
    let time = chrono::DateTime::parse_from_rfc3339(stamp).ok()?;
    Some(time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
}

/// ファイルの名前（場所は含まない）。
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// 保存先の名前に、案件の拡張子が付いていなければ、足す。
fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(project::EXTENSION)) {
        return path;
    }
    let mut name = path.file_name().map(|n| n.to_owned()).unwrap_or_default();
    name.push(format!(".{}", project::EXTENSION));
    path.with_file_name(name)
}

impl App {
    /// 柄の絵が、どこから来たか。案件の素材なら、素材の一覧（付属の見本を除く）での番号。
    fn source_of(&self, picture: &Picture) -> Option<Source> {
        match picture {
            Picture::Logo => Some(Source::Builtin("logo".to_owned())),
            Picture::Band => Some(Source::Builtin("band".to_owned())),
            Picture::Image { rgba, .. } => self
                .assets
                .iter()
                .filter(|asset| asset.stored.is_some())
                .position(|asset| matches!(&asset.picture, Picture::Image { rgba: other, .. } if std::sync::Arc::ptr_eq(rgba, other)))
                .map(Source::Asset),
        }
    }

    /// いまの状態を、案件のファイルに書く形にする。
    pub(super) fn project(&self) -> Result<Project, String> {
        let assets = self
            .assets
            .iter()
            .filter_map(|asset| {
                let mut stored = asset.stored.clone()?;
                stored.name = asset.name.clone();
                stored.size = asset.size;
                Some(stored)
            })
            .collect();
        let decals = self
            .decals
            .iter()
            .enumerate()
            .map(|(index, placed)| {
                let source = self.source_of(&placed.picture).ok_or_else(|| format!("柄 {} の絵が、素材の一覧にありません", index + 1))?;
                Ok(Placement { decal: placed.decal, back: placed.back, source, layer: placed.layer.clone() })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Project { shape: self.dims.shape(), assets, decals, base: self.base_file.clone() })
    }

    /// 状態の印。案件のファイルに入るもの（型、寸法、柄、素材、展開図の絵）から作る。
    /// 大きい素材の中身そのものは見ない（中身の印を使う）ので、毎回計算しても軽い。
    pub(super) fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        let d = &self.dims;
        (d.sides, [d.w, d.d, d.h, d.t, d.bulge, d.roof].map(f32::to_bits)).hash(&mut hasher);
        for asset in &self.assets {
            if let Some(stored) = &asset.stored {
                (&asset.name, asset.size.map(f32::to_bits), stored.id()).hash(&mut hasher);
            }
        }
        self.base_file.as_ref().map(|base| (base.id(), &base.name)).hash(&mut hasher);
        for placed in &self.decals {
            let decal = &placed.decal;
            (decal.panel, decal.at.map(f32::to_bits), decal.size.map(f32::to_bits), decal.angle.to_bits(), placed.back, &placed.layer).hash(&mut hasher);
            match self.source_of(&placed.picture) {
                Some(Source::Builtin(name)) => (0u8, name).hash(&mut hasher),
                Some(Source::Asset(index)) => (1u8, index).hash(&mut hasher),
                None => 2u8.hash(&mut hasher),
            }
        }
        hasher.finish()
    }

    /// いまの状態を「保存してある状態」として覚える（起動したとき）。
    pub(super) fn mark_saved(&mut self) {
        self.file.saved = self.fingerprint();
    }

    /// 保存していない変更があるか。
    pub(super) fn unsaved(&self) -> bool {
        self.fingerprint() != self.file.saved
    }

    /// 上の帯に出す、案件の名前。
    pub(super) fn project_name(&self) -> String {
        self.file.path.as_deref().map_or_else(|| UNTITLED.to_owned(), file_name)
    }

    pub(super) fn is_saving(&self) -> bool {
        self.file.saving.is_some()
    }

    /// 開いた案件を、画面の状態にする。表示できなかったものの知らせを返す。
    pub(super) fn apply_project(&mut self, project: Project) -> Vec<String> {
        let mut problems = Vec::new();
        self.dims = Dims::from_shape(&project.shape);
        (self.template, self.surface, self.problems) = self.dims.build();
        self.sheet = make_sheet_art(&self.template);
        self.built_dims = Some(self.dims);

        // 素材。付属の見本のあとに、案件の素材を並べる。
        self.assets = assets::bundled();
        self.replace_request = None;
        let first = self.assets.len();
        for stored in project.assets {
            let picture = match super::decode_image(stored.bytes()) {
                Ok(picture) => picture,
                Err(why) => {
                    problems.push(format!("素材「{}」は、絵として表示できません（{why}）。案件の中には残してあります。", stored.name));
                    // 中身は残したまま、灰色の四角で代わりに見せる。
                    Picture::Image { w: 2, h: 2, rgba: std::sync::Arc::new([150u8, 150, 150, 255].repeat(4)) }
                }
            };
            self.assets.push(assets::Asset { name: stored.name.clone(), picture, size: stored.size, thumb: None, bundled: false, stored: Some(stored) });
        }

        // 柄。
        self.decals.clear();
        for (index, placed) in project.decals.iter().enumerate() {
            let picture = match &placed.source {
                Source::Builtin(name) if name == "logo" => Picture::Logo,
                Source::Builtin(name) if name == "band" => Picture::Band,
                Source::Builtin(name) => {
                    problems.push(format!("柄 {} は、このパケノバにない見本（{name}）を使っているので、外しました", index + 1));
                    continue;
                }
                Source::Asset(asset) => match self.assets.get(first + asset) {
                    Some(asset) => asset.picture.clone(),
                    None => continue,
                },
            };
            self.decals.push(Placed { decal: placed.decal, picture, back: placed.back, layer: placed.layer.clone() });
        }

        // 展開図まるごとの絵。
        (self.custom, self.custom_source, self.base_file) = (None, None, None);
        self.psd_info.clear();
        self.psd_path.clear();
        if let Some(base) = project.base {
            if self.apply_psd(base.bytes(), &base.name) {
                self.base_file = Some(base);
            } else {
                problems.push(format!("展開図の絵「{}」を読めなかったので、外しました", base.name));
            }
        }

        // 前の案件の、途中の操作や履歴は持ち越さない。
        self.history = history::History::default();
        (self.selected, self.grab, self.hover, self.preview, self.carry, self.asset_selected, self.pending_shape) = (None, None, None, None, None, None, None);
        (self.editing, self.keep_decals) = (false, false);
        (self.renaming, self.layer_drag) = (None, None);
        self.flat = [FlatCam::default(); 2];
        self.fold = 1.0;
        self.open = [0.0; 2];
        (self.animate_to, self.open_to, self.view_to, self.view_before_flat) = (None, [None; 2], None, None);
        self.cams[0] = (SHAPE_VIEW.0, SHAPE_VIEW.1, 1.0);
        if self.cam_owner == Workspace::Shape {
            (self.yaw, self.pitch, self.zoom) = self.cams[0];
        }
        (self.art_dirty, self.back_dirty) = (true, true);
        problems
    }

    /// 案件のファイルを開く。
    pub(super) fn open_path(&mut self, path: &Path) {
        let opened = match Project::open(path) {
            Ok(opened) => opened,
            Err(e) => {
                self.file.notice = Some(Notice { title: "案件を開けませんでした".to_owned(), lines: vec![path.display().to_string(), e.to_string()] });
                return;
            }
        };
        let mut problems = opened.problems;
        problems.extend(self.apply_project(opened.project));
        let name = file_name(path);
        if problems.is_empty() {
            self.file.path = Some(path.to_owned());
            self.file.suggest = name.clone();
            self.mark_saved();
            self.remember_recent(path);
            self.say(format!("{name} を開きました"));
        } else {
            // 外したものがあるので、元のファイルにそのまま上書きさせない。名前なしの、未保存の案件として開く。
            self.file.path = None;
            self.file.suggest = format!("{}（開き直し）", name.trim_end_matches(&format!(".{}", project::EXTENSION)));
            self.file.saved = !self.fingerprint();
            let mut lines = vec![format!("{name} の一部を読めませんでした。読めた部分だけを開いています。")];
            lines.extend(problems.into_iter().map(|p| format!("・{p}")));
            lines.push("元のファイルは、そのまま残っています。この状態を残すには、名前を付けて保存してください。".to_owned());
            self.file.notice = Some(Notice { title: "一部を外して開きました".to_owned(), lines });
        }
    }

    /// 落ちたときの控えを開く。元の案件の場所につなぎ、保存していない状態にする。開けたら真。
    pub(super) fn open_backup(&mut self, backup: &Path, original: Option<PathBuf>, name: &str) -> bool {
        let opened = match Project::open(backup) {
            Ok(opened) => opened,
            Err(e) => {
                self.file.notice = Some(Notice { title: "控えを開けませんでした".to_owned(), lines: vec![name.to_owned(), e.to_string(), "控えは、そのまま残してあります。".to_owned()] });
                return false;
            }
        };
        let mut problems = opened.problems;
        problems.extend(self.apply_project(opened.project));
        // 元の場所のフォルダがなくなっていたら、名前なしにする（保存するときに、場所を選んでもらう）。
        self.file.path = original.filter(|p| p.parent().is_some_and(Path::exists));
        self.file.suggest = self.file.path.as_deref().map_or_else(|| UNTITLED.to_owned(), file_name);
        self.file.saved = !self.fingerprint();
        let mut lines = vec![format!("{name} を、控えから開きました。保存すると、元の案件のファイルが新しくなります。")];
        if self.file.path.is_none() {
            lines = vec![format!("{name} を、控えから開きました。名前を付けて保存してください。")];
        }
        lines.extend(problems.into_iter().map(|p| format!("・{p}")));
        self.file.notice = Some(Notice { title: "控えから開きました".to_owned(), lines });
        true
    }

    /// 開くファイルを選んでもらう。
    fn ask_open(&mut self) -> Option<PathBuf> {
        if let Some(answer) = self.file.answer_open.take() {
            return Some(answer);
        }
        let mut dialog = rfd::FileDialog::new().set_title("案件を開く").add_filter("パケノバの案件", &[project::EXTENSION]);
        if let Some(dir) = self.file.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file()
    }

    /// 保存先を選んでもらう。
    fn ask_save(&mut self) -> Option<PathBuf> {
        if let Some(answer) = self.file.answer_save.take() {
            return Some(with_extension(answer));
        }
        let mut dialog = rfd::FileDialog::new().set_title("名前を付けて保存").add_filter("パケノバの案件", &[project::EXTENSION]).set_file_name(with_extension(PathBuf::from(&self.file.suggest)).display().to_string());
        if let Some(dir) = self.file.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.save_file()?;
        let path = with_extension(picked.clone());
        // 拡張子を足した結果、別のファイルを指すことになった場合。上書きの確認が済んでいないので、止める。
        if path != picked && path.exists() {
            self.file.notice = Some(Notice { title: "保存しませんでした".to_owned(), lines: vec![format!("{} は、すでにあります。", path.display()), "別の名前を付けるか、拡張子（.pknb）まで入れて、もう一度保存してください。".to_owned()] });
            return None;
        }
        Some(path)
    }

    /// 別の案件を開く。保存していない変更があれば、先に確かめる。
    pub(super) fn request_open(&mut self, path: Option<PathBuf>) {
        if self.is_saving() {
            self.say("保存しています。終わってから、開いてください。");
        } else if self.unsaved() {
            self.file.pending = Some(Then::Open(path));
        } else {
            self.open_now(path);
        }
    }

    fn open_now(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path.or_else(|| self.ask_open()) {
            self.open_path(&path);
        }
    }

    /// 保存する。場所が決まっていなければ、名前を付けて保存する。保存を始めたら真。
    fn save(&mut self, ctx: &egui::Context, then: Then) -> bool {
        match self.file.path.clone() {
            Some(path) => self.start_saving(ctx, path, then),
            None => self.save_as(ctx, then),
        }
    }

    fn save_as(&mut self, ctx: &egui::Context, then: Then) -> bool {
        if self.is_saving() {
            return false;
        }
        match self.ask_save() {
            Some(path) => self.start_saving(ctx, path, then),
            None => false,
        }
    }

    /// 裏で保存を始める。
    fn start_saving(&mut self, ctx: &egui::Context, path: PathBuf, then: Then) -> bool {
        if self.is_saving() {
            return false;
        }
        let project = match self.project() {
            Ok(project) => project,
            Err(why) => {
                self.file.notice = Some(Notice { title: "保存できませんでした".to_owned(), lines: vec![why] });
                return false;
            }
        };
        let (sender, result) = channel();
        let (target, ctx) = (path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let outcome = project.save(&target).map_err(|e| e.to_string());
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        self.say(format!("{} を保存しています…", file_name(&path)));
        self.file.saving = Some(Saving { result, path, print: self.fingerprint(), then });
        true
    }

    /// 保存や確認のあとの操作を続ける。
    fn proceed(&mut self, ctx: &egui::Context, then: Then) {
        match then {
            Then::Nothing => {}
            Then::Close => {
                self.file.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Then::Open(path) => self.open_now(path),
            Then::New => self.new_now(),
        }
    }

    /// 新しい案件を始める。保存していない変更があれば、先に確かめる。
    pub(super) fn request_new(&mut self) {
        if self.is_saving() {
            self.say("保存しています。終わってから、新しい案件を始めてください。");
        } else if self.unsaved() {
            self.file.pending = Some(Then::New);
        } else {
            self.new_now();
        }
    }

    /// 起動したときと同じ、まっさらな案件にする（差し込み箱、柄も素材もなし）。型を選ぶところから始める。
    pub(super) fn new_now(&mut self) {
        let shape = Dims::for_shape(0, self.dims.t).shape();
        self.apply_project(Project { shape, assets: Vec::new(), decals: Vec::new(), base: None });
        self.file.path = None;
        self.file.suggest = UNTITLED.to_owned();
        self.mark_saved();
        self.workspace = Workspace::Shape;
        self.library.open = false;
        self.say("新しい案件を始めました");
    }

    /// 最近使った案件に入れる（先頭へ。10 件まで）。
    fn remember_recent(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).map(|p| simplify(&p)).unwrap_or_else(|_| path.to_owned());
        self.settings.recent.retain(|p| *p != path);
        self.settings.recent.insert(0, path);
        self.settings.recent.truncate(RECENT);
        if let Err(e) = self.settings.save() {
            eprintln!("{e}");
        }
    }

    /// 毎回、画面を描く前に行う。保存の結果を受け取り、ウィンドウを閉じる操作を見張り、題をそろえる。
    pub(super) fn file_frame(&mut self, ctx: &egui::Context) {
        // 裏の保存が終わったか。
        if let Some(saving) = &self.file.saving {
            let outcome = match saving.result.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("保存の途中で止まりました".to_owned())),
            };
            if let Some(outcome) = outcome {
                let Saving { path, print, then, .. } = self.file.saving.take().expect("保存中");
                match outcome {
                    Ok(()) => {
                        self.file.suggest = file_name(&path);
                        self.say(format!("{} を保存しました", file_name(&path)));
                        self.remember_recent(&path);
                        self.file.path = Some(path);
                        self.file.saved = print;
                        self.proceed(ctx, then);
                    }
                    Err(why) => {
                        self.message = None;
                        self.file.notice = Some(Notice { title: "保存できませんでした".to_owned(), lines: vec![path.display().to_string(), why, "前に保存したファイルは、そのまま残っています。".to_owned()] });
                    }
                }
            }
        }

        // ウィンドウを閉じようとしたとき。保存していない変更があれば、止めて確かめる。
        if ctx.input(|i| i.viewport().close_requested()) && !self.file.allow_close {
            if let Some(saving) = &mut self.file.saving {
                // 保存が終わるのを待ってから閉じる。
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                saving.then = Then::Close;
            } else if self.unsaved() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.file.pending = Some(Then::Close);
            }
        }

        // ウィンドウの題。案件の名前と、未保存の印。
        let title = format!("{}{} - パケノバ", self.project_name(), if self.unsaved() { " *" } else { "" });
        if title != self.file.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.file.title = title;
        }
    }

    /// ファイルのキー。文字を打っている途中でも効く。
    pub(super) fn file_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        let new = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        let action = ctx.input_mut(|i| {
            if i.consume_shortcut(&new) {
                Some(FileAction::New)
            } else if i.consume_shortcut(&save_as) {
                Some(FileAction::SaveAs)
            } else if i.consume_shortcut(&save) {
                Some(FileAction::Save)
            } else if i.consume_shortcut(&open) {
                Some(FileAction::Open)
            } else {
                None
            }
        });
        if action.is_some() {
            self.file.action = action;
        }
    }

    /// 上の帯の「ファイル」のメニュー。
    pub(super) fn file_menu(&mut self, ui: &mut egui::Ui) {
        let saving = self.is_saving();
        let mut chosen = None;
        let mut open = None;
        let mut clear = false;
        ui.menu_button("ファイル", |ui| {
            ui.set_min_width(220.0);
            let mut pick = |ui: &mut egui::Ui, enabled: bool, label: &str, keys: &str, action: FileAction| {
                if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked() {
                    chosen = Some(action);
                    ui.close();
                }
            };
            pick(ui, !saving, "新しい案件", "Ctrl+N", FileAction::New);
            pick(ui, !saving, "開く…", "Ctrl+O", FileAction::Open);
            ui.add_enabled_ui(!saving, |ui| {
                ui.menu_button("最近使った案件", |ui| {
                    ui.set_min_width(260.0);
                    if self.settings.recent.is_empty() {
                        ui.weak("まだありません");
                    }
                    for path in &self.settings.recent {
                        let exists = path.exists();
                        let label = if exists { file_name(path) } else { format!("{}（見つかりません）", file_name(path)) };
                        if ui.add_enabled(exists, egui::Button::new(label)).on_hover_text(path.display().to_string()).on_disabled_hover_text(path.display().to_string()).clicked() {
                            open = Some(path.clone());
                            ui.close();
                        }
                    }
                    if !self.settings.recent.is_empty() {
                        ui.separator();
                        if ui.button("一覧を消す").on_hover_text("ファイルは消えません").clicked() {
                            clear = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            pick(ui, !saving, "保存", "Ctrl+S", FileAction::Save);
            pick(ui, !saving, "名前を付けて保存…", "Ctrl+Shift+S", FileAction::SaveAs);
        });
        if chosen.is_some() {
            self.file.action = chosen;
        }
        if let Some(path) = open {
            self.file.recent = Some(path);
        }
        if clear {
            self.settings.recent.clear();
            if let Err(e) = self.settings.save() {
                self.say(e);
            }
        }
    }

    /// メニューやキーで頼まれた操作を行う。確認のダイアログが出ている間は、受け付けない。
    pub(super) fn run_file_action(&mut self, ctx: &egui::Context) {
        if self.file.pending.is_some() || self.file.notice.is_some() {
            (self.file.action, self.file.recent) = (None, None);
            return;
        }
        if let Some(path) = self.file.recent.take() {
            self.request_open(Some(path));
            return;
        }
        let Some(action) = self.file.action.take() else { return };
        match action {
            FileAction::New => self.request_new(),
            FileAction::Open => self.request_open(None),
            FileAction::Save => {
                if self.is_saving() {
                    // すでに保存しているところ。
                } else if self.file.path.is_some() && !self.unsaved() {
                    self.say("変更はありません（保存してあります）");
                } else {
                    self.save(ctx, Then::Nothing);
                }
            }
            FileAction::SaveAs => {
                self.save_as(ctx, Then::Nothing);
            }
        }
    }

    /// 作業を止めて確かめるダイアログ。保存していない変更の確認と、知らせ。
    pub(super) fn file_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(notice) = &self.file.notice {
            let response = egui::Modal::new(egui::Id::unique("file-notice")).show(ctx, |ui| {
                ui.set_width(460.0);
                ui.strong(&notice.title);
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for line in &notice.lines {
                        ui.label(line);
                    }
                });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.button("閉じる").clicked()).inner
            });
            if response.inner || response.should_close() {
                self.file.notice = None;
            }
            return;
        }

        let Some(then) = self.file.pending.clone() else { return };
        let name = self.project_name();
        let response = egui::Modal::new(egui::Id::unique("confirm-unsaved")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.strong(format!("「{name}」に、保存していない変更があります"));
            ui.add_space(4.0);
            ui.label(match then {
                Then::Close => "保存しないで閉じると、変更は失われます。",
                Then::Open(_) => "保存しないで別の案件を開くと、変更は失われます。",
                Then::New => "保存しないで新しい案件を始めると、変更は失われます。",
                Then::Nothing => "",
            });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = ui.button("保存する").clicked();
                let discard = ui.button("保存しない").clicked();
                let cancel = ui.button("キャンセル").clicked();
                (save, discard, cancel)
            })
            .inner
        });
        let (save, discard, cancel) = response.inner;
        if save {
            self.file.pending = None;
            // 保存が終わったら、続きを行う。保存先を選ぶのをやめたときは、何もしない。
            self.save(ctx, then);
        } else if discard {
            self.file.pending = None;
            self.proceed(ctx, then);
        } else if cancel || response.should_close() {
            self.file.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日時を_読み書きできる() {
        let now = now_local();
        assert!(show_time(&now).is_some(), "{now}");
        assert_eq!(show_time("2026-11-22T00:10:00Z").map(|t| t.len()), Some(16));
        assert_eq!(show_time(""), None);
        assert_eq!(show_time("きのう"), None);
    }

    #[test]
    fn 保存先には_案件の拡張子を付ける() {
        assert_eq!(with_extension(PathBuf::from("箱")), PathBuf::from("箱.pknb"));
        assert_eq!(with_extension(PathBuf::from("箱.pknb")), PathBuf::from("箱.pknb"));
        assert_eq!(with_extension(PathBuf::from("箱.PKNB")), PathBuf::from("箱.PKNB"));
        assert_eq!(with_extension(PathBuf::from("a/箱.v2")), PathBuf::from("a/箱.v2.pknb"));
    }

    #[test]
    fn 新しい案件は_まっさらで_型の工程から始まる() {
        let mut app = App::new(&super::super::Initial { sides: 6, workspace: 1, ..Default::default() });
        app.add_decal(Picture::Logo, [40.0, 40.0]);
        assert!(app.unsaved());
        app.request_new();
        assert_eq!(app.file.pending, Some(Then::New), "保存していない変更があれば、先に確かめる");
        app.file.pending = None;
        app.new_now();
        assert!(app.decals.is_empty());
        assert!(app.assets.iter().all(|a| a.bundled));
        assert_eq!(app.dims.shape().kind, project::Kind::TuckBox);
        assert!(app.workspace == Workspace::Shape);
        assert!(!app.unsaved());
        assert_eq!(app.project_name(), UNTITLED);
        assert!(!app.history.can_undo(), "前の案件の履歴は持ち越さない");
    }

    #[test]
    fn 最近使った案件は_新しい順に10件まで() {
        let dir = std::env::temp_dir().join(format!("pakenoba-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&super::super::Initial::default());
        for i in 0..12 {
            let path = dir.join(format!("箱{i}.pknb"));
            app.file.path = None;
            app.project().unwrap().save(&path).unwrap();
            app.open_path(&path);
        }
        app.open_path(&dir.join("箱3.pknb"));
        let names: Vec<String> = app.settings.recent.iter().map(|p| file_name(p)).collect();
        assert_eq!(names.len(), RECENT);
        assert_eq!(names[0], "箱3.pknb");
        assert_eq!(names[1], "箱11.pknb");
        assert!(!names.contains(&"箱1.pknb".to_owned()), "古いものから消える");
        assert_eq!(names.iter().filter(|n| *n == "箱3.pknb").count(), 1, "同じ案件は 1 つだけ");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
