//! 「素材」の画面。素材を探して、この案件に取り込むための、大きな画面。
//!
//! 左に場所（この案件の素材、登録したフォルダ、付属の見本）、中央に大きな絵の一覧、右に選んだものの詳しい情報。
//! 立体は出さない。素材を箱に置くのは、デザインの工程で（左の素材の欄からドラッグ）。
//!
//! 登録したフォルダ（コレクション）の中は、案件には入れない。取り込んだときに、中身を案件に写す（案件のファイルだけで開けるように）。
//! 一覧の絵は、別のスレッドで作る（大きな画像がたくさんあっても、画面が止まらないように）。

use pakenoba_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use eframe::egui::{self, Color32, Rect, Sense, vec2};

use pakenoba_core::project;

use super::assets::{Replace, thumbnail};
use super::{App, Workspace, decode_image, file, is_image};
use crate::paint::Picture;

/// フォルダの中を見る深さと、数の上限（大きすぎるフォルダを登録しても、止まらないように）。
const SCAN_DEPTH: usize = 4;
const SCAN_LIMIT: usize = 5000;
/// 一覧の絵と、右の大きな絵の、もとの大きさ（px）。
const TILE_PX: usize = 160;
const PREVIEW_PX: usize = 512;
/// 覚えておく絵の数。超えたら、しばらく見ていないものから捨てる。
const KEEP: usize = 600;

/// 左の場所。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Place {
    Project,
    /// 登録したフォルダ（設定での番号）。
    Collection(usize),
    Bundled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sort {
    Name,
    Newer,
}

/// 選んでいるもの。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    /// 素材の欄の番号（この案件の素材か、付属の見本）。
    Asset(usize),
    /// 登録したフォルダの中の画像。
    File(PathBuf),
}

/// フォルダの中の画像 1 つ。
pub(super) struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// 登録したフォルダから見た、入っているフォルダ（すぐ下なら空）。
    pub folder: String,
    pub modified: Option<SystemTime>,
    pub bytes: u64,
}

/// 登録したフォルダを見た結果。
pub(super) struct Scan {
    pub entries: Vec<Entry>,
    /// フォルダが見つからない（外付けのドライブを外した、名前を変えた、など）。
    pub missing: bool,
    /// 数の上限で、途中までしか見ていない。
    pub capped: bool,
}

/// フォルダの中の画像（PNG、JPEG、PSD、SVG）を探す。隠しフォルダは見ない。
pub(super) fn scan(root: &Path) -> Scan {
    if !root.is_dir() {
        return Scan { entries: Vec::new(), missing: true, capped: false };
    }
    let mut entries = Vec::new();
    let mut capped = false;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut children: Vec<_> = read.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = child.file_type() else { continue };
            let path = child.path();
            if kind.is_dir() {
                if depth + 1 < SCAN_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if is_image(&path) {
                if entries.len() >= SCAN_LIMIT {
                    capped = true;
                    break;
                }
                let meta = child.metadata().ok();
                let folder = path.parent().and_then(|p| p.strip_prefix(root).ok()).map(|p| p.display().to_string()).unwrap_or_default();
                entries.push(Entry { name, folder, modified: meta.as_ref().and_then(|m| m.modified().ok()), bytes: meta.map_or(0, |m| m.len()), path });
            }
        }
        if capped {
            break;
        }
    }
    Scan { entries, missing: false, capped }
}

/// 一覧の絵 1 つ。
pub(super) enum Thumb {
    Waiting,
    Ready {
        texture: egui::TextureHandle,
        /// 画像の大きさ（px）。
        size: [usize; 2],
        /// 中身の印（取り込み済みかを見るため）。
        id: String,
    },
    Broken(String),
}

struct Made {
    rgba: Vec<u8>,
    size: [usize; 2],
    id: String,
}

struct Done {
    path: PathBuf,
    px: usize,
    result: Result<Made, String>,
}

/// 一覧の絵を、別のスレッドで作って、覚えておく。
pub(super) struct Thumbs {
    send: Option<mpsc::Sender<(PathBuf, usize)>>,
    done_send: mpsc::Sender<Done>,
    done: mpsc::Receiver<Done>,
    cache: HashMap<(PathBuf, usize), (Thumb, u64)>,
    frame: u64,
}

/// 画像のファイルから、一覧の絵を作る。
fn make(path: &Path, px: usize) -> Result<Made, String> {
    let bytes = std::fs::read(path).map_err(|e| t!("Could not open: {e}"))?;
    let picture = decode_image(&bytes)?;
    let Picture::Image { w, h, .. } = &picture else { return Err(t!("Not an image").to_owned()) };
    let size = [*w, *h];
    let rgba = thumbnail(&picture, [50.0, 50.0 * *h as f32 / *w as f32], px);
    let id = project::Asset::new("", bytes, [0.0, 0.0], "").id().to_owned();
    Ok(Made { rgba, size, id })
}

impl Thumbs {
    fn new() -> Thumbs {
        let (done_send, done) = mpsc::channel();
        Thumbs { send: None, done_send, done, cache: HashMap::new(), frame: 0 }
    }

    /// その画像の絵。まだなければ、作るように頼む。
    fn get(&mut self, ctx: &egui::Context, path: &Path, px: usize) -> &Thumb {
        let key = (path.to_path_buf(), px);
        if !self.cache.contains_key(&key) {
            let send = self.send.get_or_insert_with(|| {
                let (send, wanted) = mpsc::channel::<(PathBuf, usize)>();
                let done = self.done_send.clone();
                let ctx = ctx.clone();
                std::thread::Builder::new()
                    .name("pakenoba-thumbs".to_owned())
                    .spawn(move || {
                        // 新しく頼まれたものから作る（流し見たあとで、いま見ている絵が後回しにならないように）。
                        let mut queue = Vec::new();
                        loop {
                            if queue.is_empty() {
                                match wanted.recv() {
                                    Ok(job) => queue.push(job),
                                    Err(_) => return,
                                }
                            }
                            queue.extend(wanted.try_iter());
                            let Some((path, px)) = queue.pop() else { continue };
                            let result = make(&path, px);
                            if done.send(Done { path, px, result }).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    })
                    .expect("could not start the thumbnail thread");
                send
            });
            let _ = send.send(key.clone());
            self.cache.insert(key.clone(), (Thumb::Waiting, self.frame));
        }
        let entry = self.cache.get_mut(&key).expect("just inserted");
        entry.1 = self.frame;
        &entry.0
    }

    /// できた絵を受け取る。多すぎたら、しばらく見ていないものから捨てる。
    fn receive(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        for done in self.done.try_iter() {
            let thumb = match done.result {
                Ok(made) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([done.px, done.px], &made.rgba);
                    let texture = ctx.load_texture(format!("library-{}-{}", done.px, done.path.display()), image, egui::TextureOptions::LINEAR);
                    Thumb::Ready { texture, size: made.size, id: made.id }
                }
                Err(e) => Thumb::Broken(e),
            };
            // 頼んだあとで忘れたもの（読み直した、など）は、受け取らない。
            if let Some(entry) = self.cache.get_mut(&(done.path, done.px)) {
                entry.0 = thumb;
            }
        }
        if self.cache.len() > KEEP {
            let mut old: Vec<_> = self.cache.iter().filter(|(_, (t, _))| !matches!(t, Thumb::Waiting)).map(|(k, (_, used))| (*used, k.clone())).collect();
            old.sort_by_key(|(used, _)| *used);
            for (_, key) in old.into_iter().take(self.cache.len() - KEEP + KEEP / 4) {
                self.cache.remove(&key);
            }
        }
    }

    /// そのフォルダの中の絵を忘れる（読み直すとき）。
    fn forget(&mut self, root: &Path) {
        self.cache.retain(|(path, _), _| !path.starts_with(root));
    }
}

/// 「素材」の画面の状態。行き来しても、選んだものと表示は保つ。
pub(super) struct Library {
    pub open: bool,
    pub place: Place,
    pub search: String,
    pub sort: Sort,
    pub tile: f32,
    /// 場所ごとに、選んでいるもの。
    picks: HashMap<Place, Pick>,
    scans: HashMap<PathBuf, Scan>,
    thumbs: Thumbs,
    /// テスト用: フォルダを選ぶダイアログの代わりの答え。
    pub answer_folder: Option<PathBuf>,
}

impl Library {
    pub fn new(open: bool, answer_folder: Option<PathBuf>) -> Library {
        Library { open, place: Place::Project, search: String::new(), sort: Sort::Name, tile: 140.0, picks: HashMap::new(), scans: HashMap::new(), thumbs: Thumbs::new(), answer_folder }
    }

    fn pick(&self) -> Option<&Pick> {
        self.picks.get(&self.place)
    }
}

/// 中央の一覧の 1 つ。
#[derive(Clone, Copy)]
enum Item {
    Asset(usize),
    /// 登録したフォルダの中の画像（見た結果での番号）。
    File(usize),
}

/// 透けているところが分かるように、市松模様を敷く（黒い絵も白い絵も見えるように）。
fn checker(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 6.0, Color32::from_gray(58));
    let step = 10.0;
    let painter = painter.with_clip_rect(rect.shrink(1.0).intersect(painter.clip_rect()));
    let (columns, rows) = ((rect.width() / step).ceil() as usize, (rect.height() / step).ceil() as usize);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + vec2(column as f32 * step, row as f32 * step);
            painter.rect_filled(Rect::from_min_size(min, vec2(step, step)), 0.0, Color32::from_gray(74));
        }
    }
}

fn show_modified(time: Option<SystemTime>) -> Option<String> {
    Some(chrono::DateTime::<chrono::Local>::from(time?).format("%Y-%m-%d %H:%M").to_string())
}

fn show_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

impl App {
    /// 「素材」の画面を開く（閉じる）。開くときは、その場所を見ておく。
    pub(super) fn toggle_library(&mut self) {
        self.library.open = !self.library.open;
        if self.library.open {
            self.go_place(self.library.place);
        }
    }

    fn go_place(&mut self, place: Place) {
        let place = match place {
            Place::Collection(i) if i >= self.settings.collections.len() => Place::Project,
            other => other,
        };
        self.library.place = place;
        if let Place::Collection(i) = place {
            let root = self.settings.collections[i].path.clone();
            if !self.library.scans.contains_key(&root) {
                self.library.scans.insert(root.clone(), scan(&root));
            }
        }
    }

    fn rescan(&mut self, index: usize) {
        let Some(root) = self.settings.collections.get(index).map(|c| c.path.clone()) else { return };
        self.library.thumbs.forget(&root);
        self.library.scans.insert(root.clone(), scan(&root));
        if let Some(Pick::File(path)) = self.library.picks.get(&Place::Collection(index))
            && !path.exists()
        {
            self.library.picks.remove(&Place::Collection(index));
        }
    }

    /// 登録したフォルダ `index` を、見直してから開く。
    pub(super) fn open_collection(&mut self, index: usize) {
        self.rescan(index);
        self.go_place(Place::Collection(index));
        self.library.open = true;
    }

    /// フォルダを選んでもらって、コレクションに登録する。
    fn add_collection(&mut self) {
        let answer = match self.library.answer_folder.take() {
            Some(answer) => Some(answer),
            None => rfd::FileDialog::new().set_title(t!("Register an asset folder")).pick_folder(),
        };
        let Some(folder) = answer else { return };
        if !self.settings.add(folder.clone()) {
            self.say(t!("{} is already registered.", folder.display()));
        } else if let Err(e) = self.settings.save() {
            self.say(e);
        } else {
            self.say(t!("Registered {} as an asset folder.", folder.display()));
        }
        let index = self.settings.collections.iter().position(|c| c.path == folder).unwrap_or(self.settings.collections.len().saturating_sub(1));
        self.rescan(index);
        self.go_place(Place::Collection(index));
    }

    /// 登録を外す（フォルダの中身は、そのまま）。
    fn remove_collection(&mut self, index: usize) {
        if index >= self.settings.collections.len() {
            return;
        }
        let removed = self.settings.collections.remove(index);
        if let Err(e) = self.settings.save() {
            self.say(e);
        } else {
            self.say(t!("Unregistered {} (the folder's contents are unchanged).", removed.path.display()));
        }
        self.library.scans.remove(&removed.path);
        self.library.thumbs.forget(&removed.path);
        // 番号がずれるので、フォルダで選んでいたものは忘れる。
        self.library.picks.retain(|place, _| !matches!(place, Place::Collection(_)));
        if matches!(self.library.place, Place::Collection(_)) {
            self.library.place = Place::Project;
        }
    }

    /// 登録したフォルダの、見た結果。
    fn scan_of(&self, index: usize) -> Option<&Scan> {
        self.library.scans.get(&self.settings.collections.get(index)?.path)
    }

    /// 中央に並べるもの（探す言葉で絞り、並べ替えたもの）。
    fn library_items(&self) -> Vec<Item> {
        let words = self.library.search.trim().to_lowercase();
        let hit = |name: &str| words.is_empty() || name.to_lowercase().contains(&words);
        let mut items: Vec<Item> = match self.library.place {
            Place::Project | Place::Bundled => {
                let bundled = self.library.place == Place::Bundled;
                (0..self.assets.len()).filter(|&i| self.assets[i].bundled == bundled && hit(&self.assets[i].name)).map(Item::Asset).collect()
            }
            Place::Collection(c) => self.scan_of(c).map(|scan| (0..scan.entries.len()).filter(|&i| hit(&scan.entries[i].name) || hit(&scan.entries[i].folder)).map(Item::File).collect()).unwrap_or_default(),
        };
        let entries = match self.library.place {
            Place::Collection(c) => self.scan_of(c).map(|s| &s.entries[..]),
            _ => None,
        };
        let name = |item: &Item| match *item {
            Item::Asset(i) => self.assets[i].name.to_lowercase(),
            Item::File(i) => entries.map_or_else(String::new, |e| format!("{}/{}", e[i].folder, e[i].name).to_lowercase()),
        };
        match self.library.sort {
            Sort::Name => items.sort_by_cached_key(name),
            Sort::Newer => {
                // 取り込んだ日時か、ファイルを書き換えた日時の、新しい順。
                let when = |item: &Item| match *item {
                    Item::Asset(i) => self.assets[i].stored.as_ref().map(|s| s.imported.clone()).unwrap_or_default(),
                    Item::File(i) => entries.and_then(|e| e[i].modified).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()).unwrap_or_default(),
                };
                items.sort_by_cached_key(|item| std::cmp::Reverse(when(item)));
            }
        }
        items
    }

    /// この案件に取り込んだ素材の、中身の印。
    fn project_ids(&self) -> HashMap<String, usize> {
        self.assets.iter().enumerate().filter_map(|(i, a)| Some((a.stored.as_ref()?.id().to_owned(), i))).collect()
    }

    /// 画面全体。
    pub(super) fn library_screen(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.library.thumbs.receive(&ctx);
        if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.library.open = false;
            return;
        }
        egui::Panel::left("library-places").resizable(true).default_size(220.0).size_range(170.0..=360.0).frame(super::theme::side_frame()).show(ui, |ui| super::panels::boxed(ui, t!("Assets"), true, |ui| self.library_places(ui)));
        egui::Panel::right("library-detail").resizable(true).default_size(300.0).size_range(220.0..=480.0).frame(super::theme::side_frame()).show(ui, |ui| super::panels::boxed(ui, t!("Preview"), true, |ui| self.library_detail(ui)));
        egui::CentralPanel::default().show(ui, |ui| self.library_list(ui));
    }

    /// 左: 場所。
    fn library_places(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        let mine = self.assets.iter().filter(|a| !a.bundled).count();
        let name = self.project_name();
        if ui.selectable_label(self.library.place == Place::Project, t!("Assets in {name} ({mine})")).on_hover_text(t!("The assets saved in this project file. They are the ones you can place on the box")).clicked() {
            go = Some(Place::Project);
        }
        ui.add_space(14.0);
        ui.label(egui::RichText::new(t!("Import from")).color(super::theme::FAINT).small());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(t!("Collections"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(t!("+ Register…")).on_hover_text(t!("Register a folder that holds assets. It can be used from any project")).clicked() {
                    self.add_collection();
                }
            });
        });
        if self.settings.collections.is_empty() {
            ui.small(t!("Register folders that hold assets and they are listed here. Their contents are never moved."));
        }
        let mut rescan = None;
        let mut remove = None;
        for (i, collection) in self.settings.collections.iter().enumerate() {
            let scan = self.library.scans.get(&collection.path);
            let label = match scan {
                Some(scan) if scan.missing => t!("{} (not found)", collection.name()),
                Some(scan) => t!("{} ({}{})", collection.name(), scan.entries.len(), if scan.capped { "+" } else { "" }),
                None => collection.name(),
            };
            let response = ui.selectable_label(self.library.place == Place::Collection(i), label).on_hover_text(collection.path.display().to_string());
            if response.clicked() {
                go = Some(Place::Collection(i));
            }
            response.context_menu(|ui| {
                if ui.button(t!("Rescan")).on_hover_text(t!("Lists images added to or changed in the folder again")).clicked() {
                    rescan = Some(i);
                    ui.close();
                }
                if ui.button(t!("Unregister")).on_hover_text(t!("The folder's contents are unchanged")).clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        ui.add_space(10.0);
        if ui.selectable_label(self.library.place == Place::Bundled, t!("Built-in samples")).clicked() {
            go = Some(Place::Bundled);
        }
        // 見本パック（別に配る素材の詰め合わせ）。入れると、コレクションに並ぶ。
        if !self.pack_installed() {
            ui.add_space(10.0);
            ui.label(t!("Sample pack"));
            ui.small(t!("Fictional logos and seamless patterns for trying things out (about 40 MB)."));
            ui.horizontal_wrapped(|ui| {
                if super::pack::PACK_URL.is_some() && ui.add_enabled(self.pack_job.is_none(), egui::Button::new(t!("Download"))).clicked() {
                    self.download_pack();
                }
                if ui.button(t!("Install from a ZIP…")).clicked() {
                    self.install_pack_file();
                }
            });
            if self.pack_job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small(t!("Downloading the sample pack…"));
                });
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            let back = match self.workspace {
                Workspace::Shape => t!("Structure"),
                Workspace::Design => t!("Design"),
                Workspace::Output => t!("Output"),
            };
            if ui.button(t!("Back to {back}")).on_hover_text("Esc").clicked() {
                self.library.open = false;
            }
        });
        if let Some(i) = rescan {
            self.rescan(i);
            go = Some(Place::Collection(i));
        }
        if let Some(i) = remove {
            self.remove_collection(i);
        } else if let Some(place) = go {
            self.go_place(place);
        }
    }

    /// 中央: 探す、並べ替える、大きな絵の一覧。
    fn library_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.library.search).hint_text(t!("Search by name")).desired_width(220.0));
            if !self.library.search.is_empty() && ui.small_button("×").on_hover_text(t!("Clear the search")).clicked() {
                self.library.search.clear();
            }
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("library-sort").selected_text(if self.library.sort == Sort::Name { t!("By name") } else { t!("Newest first") }).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.library.sort, Sort::Name, t!("By name"));
                ui.selectable_value(&mut self.library.sort, Sort::Newer, t!("Newest first"));
            });
            ui.add_space(8.0);
            ui.label(t!("Thumbnail size"));
            ui.add(egui::Slider::new(&mut self.library.tile, 80.0..=260.0).show_value(false));
            if let Place::Collection(i) = self.library.place {
                ui.add_space(8.0);
                if ui.button(t!("Rescan")).on_hover_text(t!("Lists images added to or changed in the folder again")).clicked() {
                    self.rescan(i);
                }
            }
        });
        ui.separator();
        // 場所ごとの、空のときの案内。
        match self.library.place {
            Place::Collection(i) => match self.scan_of(i) {
                Some(scan) if scan.missing => {
                    ui.label(t!("Folder not found: {}", self.settings.collections[i].path.display()));
                    ui.small(t!("For an external drive or a network folder, connect it and press \"Rescan\"."));
                    if ui.button(t!("Unregister")).clicked() {
                        self.remove_collection(i);
                    }
                    return;
                }
                Some(scan) if scan.capped => {
                    ui.small(t!("There are many images, so only the first {SCAN_LIMIT} are shown. Register a smaller folder to see them all."));
                }
                _ => {}
            },
            Place::Project if !self.assets.iter().any(|a| !a.bundled) => {
                ui.label(t!("This project has no assets yet."));
                ui.small(t!("Drop images (PNG, JPEG, PSD, SVG) on the window, or import them from a folder registered under \"Collections\" on the left."));
                return;
            }
            _ => {}
        }
        let items = self.library_items();
        if items.is_empty() {
            ui.label(if self.library.search.trim().is_empty() { t!("There are no images (PNG, JPEG, PSD, SVG).") } else { t!("No assets match the search.") });
            return;
        }
        let imported = self.project_ids();
        let tile = self.library.tile;
        let gap = 10.0;
        let cell = vec2(tile, tile + 20.0);
        let columns = (((ui.available_width() + gap) / (tile + gap)).floor() as usize).max(1);
        let rows = items.len().div_ceil(columns);
        let mut import = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, cell.y + gap, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(gap, gap);
                    for &item in items.iter().skip(row * columns).take(columns) {
                        if let Some(path) = self.library_tile(ui, item, cell, &imported) {
                            import = Some(path);
                        }
                    }
                });
            }
        });
        if let Some(path) = import {
            self.import_from_library(&path);
        }
    }

    /// 一覧の 1 マス。ダブルクリックしたフォルダの画像を返す（取り込むため）。
    fn library_tile(&mut self, ui: &mut egui::Ui, item: Item, cell: egui::Vec2, imported: &HashMap<String, usize>) -> Option<PathBuf> {
        let (rect, response) = ui.allocate_exact_size(cell, Sense::click());
        let image_rect = Rect::from_min_size(rect.min, vec2(cell.x, cell.x));
        let (name, pick, file) = match item {
            Item::Asset(i) => (self.assets[i].name.clone(), Pick::Asset(i), None),
            Item::File(i) => {
                let Place::Collection(c) = self.library.place else { return None };
                let entry = &self.scan_of(c)?.entries[i];
                (entry.name.clone(), Pick::File(entry.path.clone()), Some(entry.path.clone()))
            }
        };
        let selected = self.library.pick() == Some(&pick);
        let visible = ui.is_rect_visible(rect);
        // 絵（見えているときだけ作る）。
        let mut done = false;
        let mut texture = None;
        let mut broken = false;
        if visible {
            match (&item, &file) {
                (Item::Asset(i), _) => texture = Some(self.thumb(ui.ctx(), *i)),
                (_, Some(path)) => match self.library.thumbs.get(ui.ctx(), path, TILE_PX) {
                    Thumb::Ready { texture: t, id, .. } => {
                        texture = Some(t.id());
                        done = imported.contains_key(id);
                    }
                    Thumb::Broken(_) => broken = true,
                    Thumb::Waiting => {}
                },
                _ => {}
            }
        }
        if visible {
            let visuals = ui.visuals();
            let painter = ui.painter();
            checker(painter, image_rect);
            if response.hovered() {
                painter.rect_stroke(image_rect, 6.0, egui::Stroke::new(1.0, visuals.widgets.hovered.fg_stroke.color), egui::StrokeKind::Inside);
            }
            match texture {
                Some(texture) => {
                    painter.image(texture, image_rect.shrink(8.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    let text = if broken { t!("Unreadable") } else { "…" };
                    painter.text(image_rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), visuals.weak_text_color());
                }
            }
            if selected {
                painter.rect_stroke(image_rect, 6.0, egui::Stroke::new(2.5, visuals.selection.bg_fill), egui::StrokeKind::Inside);
            }
            let used = match item {
                Item::Asset(i) => self.uses(i) > 0,
                Item::File(_) => done,
            };
            if used {
                // 箱で使っている素材か、この案件に取り込み済みの画像の印。
                painter.circle_filled(image_rect.right_bottom() - vec2(10.0, 10.0), 5.0, visuals.selection.bg_fill);
            }
            let label_rect = Rect::from_min_max(egui::pos2(rect.min.x, image_rect.max.y + 2.0), rect.max);
            let galley = ui.painter().layout(name.clone(), egui::FontId::proportional(12.0), visuals.text_color(), f32::INFINITY);
            let painter = ui.painter().with_clip_rect(label_rect);
            let x = if galley.size().x < label_rect.width() { label_rect.center().x - galley.size().x * 0.5 } else { label_rect.min.x };
            painter.galley(egui::pos2(x, label_rect.min.y), galley, Color32::WHITE);
        }
        let tip = match (&item, done) {
            (Item::File(_), true) => t!("{name}\nAlready imported into this project"),
            (Item::File(_), false) => t!("{name}\nDouble-click to import into this project"),
            _ => name,
        };
        let response = response.on_hover_text(tip);
        if response.clicked() || response.secondary_clicked() {
            self.library.picks.insert(self.library.place, pick);
        }
        let mut import = None;
        if response.double_clicked() && !done {
            import = file.clone();
        }
        if let Some(path) = file {
            response.context_menu(|ui| {
                if ui.add_enabled(!done, egui::Button::new(t!("Import into this project"))).clicked() {
                    import = Some(path.clone());
                    ui.close();
                }
            });
        } else if let Item::Asset(i) = item
            && !self.assets[i].bundled
        {
            response.context_menu(|ui| {
                if ui.button(t!("Replace asset…")).on_hover_text(t!("Every piece of artwork that uses this asset gets the new picture")).clicked() {
                    self.replace_request = Some(Replace::Asset(i));
                    ui.close();
                }
            });
        }
        import
    }

    /// フォルダの画像を、この案件に取り込む（箱には置かない）。元に戻せる。
    fn import_from_library(&mut self, path: &Path) {
        if let Some(index) = self.import_image(path, true) {
            self.say(t!("Imported {} into this project's assets. In the Design step, drag it from the assets panel onto the box.", self.assets[index].name));
        }
    }

    /// 右: 選んだものの、大きな絵と情報。
    fn library_detail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let Some(pick) = self.library.pick().cloned() else {
            ui.weak(t!("Select an asset to see it large here."));
            return;
        };
        let side = ui.available_width().min(460.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::hover());
        let image = Rect::from_center_size(rect.center(), vec2(side, side));
        checker(ui.painter(), rect);
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.add_space(6.0);
        match pick {
            Pick::Asset(index) if index < self.assets.len() => {
                let texture = self.thumb(ui.ctx(), index);
                ui.painter().image(texture, image.shrink(10.0), uv, Color32::WHITE);
                let asset = &self.assets[index];
                ui.strong(&asset.name);
                if let Picture::Image { w, h, .. } = &asset.picture {
                    ui.label(t!("Resolution: {w} × {h} px"));
                }
                if let Some(when) = asset.stored.as_ref().and_then(|s| file::show_time(&s.imported)) {
                    ui.label(t!("Imported: {when}"));
                }
                let uses = self.uses(index);
                ui.label(if uses > 0 { t!("Used in {uses} place(s) on the box") } else { t!("Not placed on the box yet").to_owned() });
                ui.add_space(6.0);
                if self.assets[index].bundled {
                    ui.weak(t!("A built-in sample. In the Design step, drag it from the assets panel onto the box."));
                } else if ui.button(t!("Replace…")).on_hover_text(t!("Every piece of artwork that uses this asset gets the new picture")).clicked() {
                    self.replace_request = Some(Replace::Asset(index));
                }
            }
            Pick::File(path) => {
                let ctx = ui.ctx().clone();
                let (texture, size, id, broken) = match self.library.thumbs.get(&ctx, &path, PREVIEW_PX) {
                    Thumb::Ready { texture, size, id } => (Some(texture.id()), Some(*size), Some(id.clone()), None),
                    Thumb::Broken(e) => (None, None, None, Some(e.clone())),
                    Thumb::Waiting => (None, None, None, None),
                };
                match texture {
                    Some(texture) => {
                        ui.painter().image(texture, image.shrink(10.0), uv, Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(image.center(), egui::Align2::CENTER_CENTER, if broken.is_some() { t!("Unreadable") } else { "…" }, egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
                    }
                }
                let entry = match self.library.place {
                    Place::Collection(c) => self.scan_of(c).and_then(|s| s.entries.iter().find(|e| e.path == path)),
                    _ => None,
                };
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ui.strong(&name);
                if let Some(entry) = entry
                    && !entry.folder.is_empty()
                {
                    ui.label(t!("Folder: {}", entry.folder));
                }
                if let Some([w, h]) = size {
                    ui.label(t!("Resolution: {w} × {h} px"));
                }
                if let Some(entry) = entry {
                    if let Some(when) = show_modified(entry.modified) {
                        ui.label(t!("Modified: {when}"));
                    }
                    ui.label(t!("File size: {}", show_bytes(entry.bytes)));
                }
                if let Some(e) = &broken {
                    ui.colored_label(ui.visuals().warn_fg_color, t!("This image cannot be read: {e}"));
                }
                ui.small(path.display().to_string()).on_hover_text(t!("File location"));
                ui.add_space(6.0);
                let ids = self.project_ids();
                match id.as_ref().and_then(|id| ids.get(id)) {
                    Some(&index) => {
                        let uses = self.uses(index);
                        ui.label(if uses > 0 { t!("Already imported into this project (used in {uses} place(s) on the box)") } else { t!("Already imported into this project (not placed on the box yet)").to_owned() });
                        if ui.button(t!("Show in this project's assets")).clicked() {
                            self.library.picks.insert(Place::Project, Pick::Asset(index));
                            self.go_place(Place::Project);
                        }
                    }
                    None => {
                        let ready = id.is_some();
                        if ui.add_enabled(ready, egui::Button::new(t!("Import into this project"))).on_hover_text(t!("Copies the image into the project. Moving or deleting the original file does not affect the project")).clicked() {
                            self.import_from_library(&path);
                        }
                    }
                }
            }
            Pick::Asset(_) => {}
        }
    }
}

impl App {
    /// 素材の欄の「最近使ったもの」。最近取り込んだ画像のうち、この案件にまだないもの（見つからないファイルは出さない）。
    /// クリックかドラッグで、この案件に取り込む（ドラッグなら、そのまま箱へ運べる）。
    pub(super) fn recent_assets(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.library.thumbs.receive(&ctx);
        let imported = self.project_ids();
        let side = self.thumb_size;
        let mut shown = Vec::new();
        for path in self.settings.recent_assets.clone() {
            if !path.is_file() {
                continue;
            }
            match self.library.thumbs.get(&ctx, &path, TILE_PX) {
                Thumb::Ready { id, .. } if imported.contains_key(id) => continue,
                Thumb::Broken(_) => continue,
                _ => shown.push(path),
            }
        }
        if shown.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(t!("Recently used"));
        let mut take = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for path in shown {
                let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
                checker(ui.painter(), rect);
                if let Thumb::Ready { texture, .. } = self.library.thumbs.get(&ctx, &path, TILE_PX) {
                    ui.painter().image(texture.id(), rect.shrink(4.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                if response.hovered() {
                    ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(1.0, ui.visuals().widgets.hovered.fg_stroke.color), egui::StrokeKind::Inside);
                }
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let response = response.on_hover_text(t!("{name}\nClick or drag to import into this project\n{}", path.display()));
                if response.drag_started() {
                    take = Some((path, true));
                } else if response.clicked() {
                    take = Some((path, false));
                }
            }
        });
        if let Some((path, carry)) = take
            && let Some(index) = self.import_image(&path, true)
        {
            self.asset_selected = Some(index);
            if carry {
                self.carry = Some(index);
            } else {
                self.say(t!("Imported {} into this project's assets.", self.assets[index].name));
            }
        }
    }
}

/// 中身の印の集まり（テスト用）。
#[cfg(test)]
fn ids(app: &App) -> std::collections::HashSet<String> {
    app.project_ids().into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Initial;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pakenoba-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path, w: u32, h: u32, color: [u8; 4]) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&color.repeat((w * h) as usize)).unwrap();
    }

    #[test]
    fn フォルダの中の_png_だけを_深さを決めて探す() {
        let dir = temp_dir("scan");
        std::fs::create_dir_all(dir.join("ロゴ/白")).unwrap();
        std::fs::create_dir_all(dir.join(".隠し")).unwrap();
        std::fs::create_dir_all(dir.join("a/b/c/d")).unwrap();
        write_png(&dir.join("帯.png"), 4, 2, [255, 0, 0, 255]);
        write_png(&dir.join("ロゴ/白/ロゴ.PNG"), 2, 2, [255, 255, 255, 255]);
        write_png(&dir.join(".隠し/秘密.png"), 2, 2, [0, 0, 0, 255]);
        write_png(&dir.join("a/b/c/d/深すぎる.png"), 2, 2, [0, 0, 0, 255]);
        std::fs::write(dir.join("メモ.txt"), "png ではない").unwrap();
        let found = scan(&dir);
        assert!(!found.missing && !found.capped);
        let mut names: Vec<_> = found.entries.iter().map(|e| (e.folder.replace('\\', "/"), e.name.clone())).collect();
        names.sort();
        assert_eq!(names, vec![(String::new(), "帯.png".to_owned()), ("ロゴ/白".to_owned(), "ロゴ.PNG".to_owned())]);
        assert!(scan(&dir.join("ない")).missing);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 一覧の絵は_別のスレッドで作り_読めない画像はそう言う() {
        let dir = temp_dir("thumbs");
        write_png(&dir.join("赤.png"), 8, 4, [255, 0, 0, 255]);
        std::fs::write(dir.join("壊れた.png"), "PNG ではない").unwrap();
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs::new();
        assert!(matches!(thumbs.get(&ctx, &dir.join("赤.png"), 32), Thumb::Waiting));
        assert!(matches!(thumbs.get(&ctx, &dir.join("壊れた.png"), 32), Thumb::Waiting));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            thumbs.receive(&ctx);
            let red = thumbs.get(&ctx, &dir.join("赤.png"), 32);
            let ready = matches!(red, Thumb::Ready { .. });
            let broken = matches!(thumbs.get(&ctx, &dir.join("壊れた.png"), 32), Thumb::Broken(_));
            if ready && broken {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "絵ができない");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let Thumb::Ready { size, id, .. } = thumbs.get(&ctx, &dir.join("赤.png"), 32) else { unreachable!() };
        assert_eq!(*size, [8, 4]);
        assert_eq!(id.len(), 64);
        thumbs.forget(&dir);
        assert!(thumbs.cache.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 登録したフォルダから取り込むと_案件の素材になり_元に戻せる() {
        let dir = temp_dir("import");
        write_png(&dir.join("青.png"), 6, 3, [0, 0, 255, 255]);
        let mut app = App::new(&Initial::default());
        app.library.answer_folder = Some(dir.clone());
        app.add_collection();
        assert_eq!(app.settings.collections.len(), 1);
        assert_eq!(app.library.place, Place::Collection(0));
        assert_eq!(app.library_items().len(), 1);
        let before = app.assets.len();
        app.import_from_library(&dir.join("青.png"));
        assert_eq!(app.assets.len(), before + 1);
        assert!(app.decals.iter().all(|d| !matches!(d.picture, Picture::Image { .. })), "取り込むだけで、箱には置かない");
        assert_eq!(ids(&app).len(), 1);
        // 同じ画像を、もう一度取り込んでも、増えない。
        app.import_from_library(&dir.join("青.png"));
        assert_eq!(app.assets.len(), before + 1);
        app.undo();
        assert_eq!(app.assets.len(), before, "取り込みは、元に戻せる");
        // 探す言葉で絞る。
        app.library.search = "赤".to_owned();
        assert!(app.library_items().is_empty());
        // 取り込んだ画像は、最近使ったものに入り、新しい案件にしても残る。
        assert_eq!(app.settings.recent_assets.first().and_then(|p| p.file_name()), Some(std::ffi::OsStr::new("青.png")));
        app.new_now();
        assert_eq!(app.settings.recent_assets.len(), 1);
        app.remove_collection(0);
        assert!(app.settings.collections.is_empty());
        assert_eq!(app.library.place, Place::Project);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
