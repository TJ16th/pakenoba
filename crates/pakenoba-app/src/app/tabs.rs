//! 1 つのウィンドウで、いくつもの案件をタブで開く（Photoshop と同じ形）。
//!
//! いま見ている案件の中身は、これまでどおり `App` の項目に置く。ほかのタブの案件は、案件ごとの項目だけを
//! `Doc` に預けておき、タブを切り替えるときに `App` と入れ替える（入れ替えるだけなので、すぐ切り替わる）。
//! 元に戻すための記録、選んでいる柄、視点、表示の出し方、落ちたときの控えも、案件ごとに持つ。
//!
//! 素材の画面、設定、のぞき込み、文字や図形の書式、描く部品（GPU）は、どのタブでも共通。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui;

use super::{App, Dims, FlatCam, Shown, Snapshot, Workspace, assets, base, edit, file, history, recovery};
use crate::art::{SheetArt, make_sheet_art};
use crate::paint::Placed;
use pakenoba_core::surface::Surface;
use pakenoba_core::template::Template;

/// 預けている案件の項目を、`App` と入れ替える。`App` の項目と同じ名前、同じ型。
macro_rules! doc {
    ($($field:ident: $ty:ty),* $(,)?) => {
        /// 預けているタブの案件。
        pub(super) struct Doc {
            $($field: $ty,)*
            /// タブに出す名前と、保存していない変更があるか（預けたときのもの。預けている間は変わらない）。
            label: String,
            dirty: bool,
        }

        fn swap(app: &mut App, doc: &mut Doc) {
            $(std::mem::swap(&mut app.$field, &mut doc.$field);)*
        }
    };
}

doc! {
    dims: Dims,
    built_dims: Option<Dims>,
    template: Rc<Template>,
    surface: Surface,
    problems: Vec<(bool, String)>,
    workspace: Workspace,
    shown: Shown,
    flat: [FlatCam; 2],
    output_kind: usize,
    assets: Vec<assets::Asset>,
    history: history::History<Snapshot>,
    asset_selected: Option<usize>,
    cams: [(f32, f32, f32); 3],
    cam_owner: Workspace,
    decals: Vec<Placed>,
    log: Vec<pakenoba_core::project::LogEntry>,
    versions: Vec<pakenoba_core::project::Version>,
    selected: Option<usize>,
    extra: Vec<usize>,
    sheet: SheetArt,
    bases: [Option<base::BaseArt>; 2],
    custom: [Option<Vec<u8>>; 2],
    file: file::FileState,
    recovery: recovery::Recovery,
    edits: Vec<edit::EditJob>,
    art_dirty: bool,
    front_rgba: Vec<u8>,
    front_tex: Option<egui::TextureHandle>,
    front_tex_dirty: bool,
    back_rgba: Vec<u8>,
    back_tex: Option<egui::TextureHandle>,
    back_tex_dirty: bool,
    back_dirty: bool,
    back_has_decals: bool,
    fold: f32,
    animate_to: Option<f32>,
    view_to: Option<(f32, f32)>,
    view_before_flat: Option<(f32, f32)>,
    open: [f32; 2],
    open_to: [Option<f32>; 2],
    yaw: f32,
    pitch: f32,
    zoom: f32,
    ortho: bool,
}

impl Doc {
    /// まっさらな案件（差し込み箱、柄も素材もなし）。中身は、入れ替えたあとに `new_now` で整える。
    fn blank(app: &App) -> Doc {
        let dims = Dims::for_shape(0, app.dims.t);
        let (template, surface, problems) = dims.build();
        let sheet = make_sheet_art(&template);
        Doc {
            dims,
            built_dims: Some(dims),
            template,
            surface,
            problems,
            workspace: Workspace::Shape,
            shown: Shown::SOLID_ONLY,
            flat: [FlatCam::default(); 2],
            output_kind: 0,
            assets: Vec::new(),
            history: history::History::default(),
            asset_selected: None,
            cams: app.cams,
            cam_owner: Workspace::Shape,
            decals: Vec::new(),
            log: Vec::new(),
            versions: Vec::new(),
            selected: None,
            extra: Vec::new(),
            sheet,
            bases: [None, None],
            custom: [None, None],
            file: file::FileState::new(None, None),
            recovery: app.recovery.sibling(),
            edits: Vec::new(),
            art_dirty: true,
            front_rgba: Vec::new(),
            front_tex: None,
            front_tex_dirty: true,
            back_rgba: Vec::new(),
            back_tex: None,
            back_tex_dirty: true,
            back_dirty: true,
            back_has_decals: false,
            fold: 1.0,
            animate_to: None,
            view_to: None,
            view_before_flat: None,
            open: [0.0; 2],
            open_to: [None; 2],
            yaw: app.yaw,
            pitch: app.pitch,
            zoom: 1.0,
            ortho: app.ortho,
            label: String::new(),
            dirty: false,
        }
    }
}

/// タブの並び。いま見ているタブの場所は `None`（中身は `App` にある）。
#[derive(Default)]
pub(super) struct Tabs {
    slots: Vec<Option<Doc>>,
    active: usize,
    /// 名前なしの案件に付けた番号（いちばん大きいもの）。
    untitled: usize,
}

impl Tabs {
    pub fn new() -> Tabs {
        Tabs { slots: vec![None], active: 0, untitled: 1 }
    }

    /// 新しい名前なしの案件の番号。タブが 1 つだけなら 1（番号を付けない）。
    pub fn next_untitled(&mut self) -> usize {
        if self.slots.len() <= 1 {
            return 1;
        }
        self.untitled += 1;
        self.untitled
    }

    /// 預けているタブで、保存していない変更があるものの番号。
    fn dirty_parked(&self) -> Option<usize> {
        self.slots.iter().position(|slot| slot.as_ref().is_some_and(|doc| doc.dirty))
    }
}

impl App {
    /// いまの案件が、まっさら（名前なしで、柄も素材もなく、保存していない変更もない）か。
    /// まっさらなら、新しい案件や開いた案件は、新しいタブではなく、このタブに入れる。
    pub(super) fn pristine(&self) -> bool {
        self.file.path.is_none() && !self.unsaved() && self.decals.is_empty() && self.assets.iter().all(|a| a.bundled) && self.bases.iter().all(Option::is_none)
    }

    /// 切り替えられない途中の仕事（保存、動画の書き出し、ベクター化）があれば、その知らせ。
    fn busy(&self) -> Option<&'static str> {
        if self.is_saving() {
            Some(t!("Saving. Switch tabs when it has finished."))
        } else if self.movie.is_some() {
            Some(t!("Exporting a video. Switch tabs when it has finished."))
        } else if self.vectorizing.is_some() {
            Some(t!("Vectorizing. Switch tabs when it has finished."))
        } else {
            None
        }
    }

    /// タブ `index` へ切り替える。
    pub(super) fn switch_tab(&mut self, index: usize) -> bool {
        if index == self.tabs.active || index >= self.tabs.slots.len() {
            return true;
        }
        if let Some(why) = self.busy() {
            self.say(why);
            return false;
        }
        let Some(mut doc) = self.tabs.slots[index].take() else { return false };
        let (label, dirty) = (self.project_name(), self.unsaved());
        swap(self, &mut doc);
        (doc.label, doc.dirty) = (label, dirty);
        self.tabs.slots[self.tabs.active] = Some(doc);
        self.tabs.active = index;
        self.after_switch();
        true
    }

    /// 切り替えたあと、つかんでいるものや、仮の姿など、前のタブの途中の操作を消す。
    fn after_switch(&mut self) {
        self.grab = None;
        self.transforming = None;
        self.carry = None;
        self.preview = None;
        self.hover_decal = None;
        self.hover_now = None;
        self.renaming = None;
        self.layer_drag = None;
        self.editing = false;
        self.pending_shape = None;
        self.replace_request = None;
        self.home = false;
        self.library.open = false;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 開いているタブの数。
    #[cfg(test)]
    pub(super) fn tabs_count(&self) -> usize {
        self.tabs.slots.len()
    }

    /// 新しいタブを開いて、そこへ移る（中身はまっさら。呼んだ側で `new_now` か、案件を開く）。
    pub(super) fn open_tab(&mut self) -> bool {
        if let Some(why) = self.busy() {
            self.say(why);
            return false;
        }
        let mut doc = Doc::blank(self);
        let (label, dirty) = (self.project_name(), self.unsaved());
        swap(self, &mut doc);
        (doc.label, doc.dirty) = (label, dirty);
        self.tabs.slots[self.tabs.active] = Some(doc);
        self.tabs.slots.push(None);
        self.tabs.active = self.tabs.slots.len() - 1;
        self.after_switch();
        true
    }

    /// その案件のファイルを開いているタブ。
    pub(super) fn tab_with(&self, path: &Path) -> Option<usize> {
        let same = |p: &Path| std::fs::canonicalize(p).ok() == std::fs::canonicalize(path).ok();
        if self.file.path.as_deref().is_some_and(same) {
            return Some(self.tabs.active);
        }
        self.tabs.slots.iter().position(|slot| slot.as_ref().and_then(|doc| doc.file.path.as_deref()).is_some_and(same))
    }

    /// タブを閉じる。保存していない変更があれば、先に確かめる。
    pub(super) fn request_close_tab(&mut self, index: usize) {
        if !self.switch_tab(index) {
            return;
        }
        if self.unsaved() {
            self.file.pending = Some(file::Then::CloseTab);
        } else {
            self.close_tab_now();
        }
    }

    /// いまのタブを閉じる。最後の 1 つなら、まっさらな案件にして、ホームを出す。
    pub(super) fn close_tab_now(&mut self) {
        if self.tabs.slots.len() <= 1 {
            self.recovery.finish();
            self.recovery = self.recovery.sibling();
            self.new_now();
            self.home = true;
            return;
        }
        let closing = self.tabs.active;
        // となり（右、なければ左）のタブへ移ってから、閉じるタブを外す。
        let next = if closing + 1 < self.tabs.slots.len() { closing + 1 } else { closing - 1 };
        if !self.switch_tab(next) {
            return;
        }
        if let Some(mut doc) = self.tabs.slots.remove(closing) {
            doc.recovery.finish();
        }
        if self.tabs.active > closing {
            self.tabs.active -= 1;
        }
    }

    /// ウィンドウを閉じる前に: ほかのタブで保存していない変更があれば、そのタブへ移って確かめる。
    /// 確かめることがなければ偽。
    pub(super) fn ask_next_unsaved_tab(&mut self) -> bool {
        let Some(index) = self.tabs.dirty_parked() else { return false };
        if self.switch_tab(index) {
            self.file.pending = Some(file::Then::Close);
            true
        } else {
            false
        }
    }

    /// 終わるとき: どのタブの控えも消す。
    pub(super) fn finish_all_tabs(&mut self) {
        self.recovery.finish();
        for doc in self.tabs.slots.iter_mut().flatten() {
            doc.recovery.finish();
        }
    }

    /// 案件の名前の帯に並べるタブ。押すと切り替え、× で閉じる。右端の ＋ で新しい案件。
    pub(super) fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let mut switch = None;
        let mut close = None;
        let active = self.tabs.active;
        let (name, dirty) = (self.project_name(), self.unsaved());
        let entries: Vec<(String, bool, Option<PathBuf>)> = self.tabs.slots.iter().enumerate().map(|(i, slot)| match slot {
            Some(doc) => (doc.label.clone(), doc.dirty, doc.file.path.clone()),
            None => (name.clone(), dirty, if i == active { self.file.path.clone() } else { None }),
        }).collect();
        for (i, (label, dirty, path)) in entries.into_iter().enumerate() {
            let text = format!("{label}{}", if dirty { " *" } else { "" });
            let tip = match (&path, dirty) {
                (Some(path), true) => format!("{}\n{}", path.display(), t!("There are unsaved changes (Ctrl+S to save)")),
                (Some(path), false) => path.display().to_string(),
                (None, true) => t!("There are unsaved changes (Ctrl+S to save)").to_owned(),
                (None, false) => String::new(),
            };
            let (response, closed) = super::icons::doc_tab(ui, &text, i == active && !self.home, true, (!tip.is_empty()).then_some(tip.as_str()));
            if closed {
                close = Some(i);
            } else if response.clicked() {
                switch = Some(i);
            }
            if response.middle_clicked() {
                close = Some(i);
            }
        }
        ui.add_space(4.0);
        if ui.add(egui::Button::new("+").frame(false).min_size(egui::vec2(24.0, 24.0))).on_hover_text(t!("New project in a new tab (Ctrl+N)")).clicked() {
            self.request_new();
        }
        if let Some(i) = switch {
            self.switch_tab(i);
            self.home = false;
        }
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    /// Ctrl+Tab と Ctrl+Shift+Tab で、となりのタブへ。Ctrl+W でいまのタブを閉じる。
    pub(super) fn tab_shortcuts(&mut self, ctx: &egui::Context) {
        let count = self.tabs.slots.len();
        let (next, prev, close) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Tab),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Tab),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
            )
        });
        if count > 1 && next {
            self.switch_tab((self.tabs.active + 1) % count);
        }
        if count > 1 && prev {
            self.switch_tab((self.tabs.active + count - 1) % count);
        }
        if close {
            self.request_close_tab(self.tabs.active);
        }
    }
}
