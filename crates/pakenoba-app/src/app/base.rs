//! 展開図まるごとの絵（外のアプリで、型紙の PSD に描いた絵）。紙の表と裏に、1 枚ずつ敷ける。
//!
//! 表の絵は、「出力」で保存した型紙の PSD に描いたもの。裏の絵は、裏の型紙の PSD（裏から見た向き）に描いたもの。
//! 読み込む、差し替える、外す、のどれも、元に戻せる。案件のファイルには、元の PSD のまま入る。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;

use pakenoba_core::project;

use super::{App, file};
use crate::art::{GuideHandling, fit, load_psd, mirror};

/// 展開図まるごとの絵 1 枚。元に戻すための記録にも入れるので、重いものは共有する。
#[derive(Clone)]
pub(super) struct BaseArt {
    /// 元のファイル（中身、名前、取り込んだ日時）。
    pub file: project::Asset,
    /// 読んだ絵（表の展開図の向き。裏の絵も、ここでは表の向きに直して持つ）。
    pub source: Arc<Vec<u8>>,
    pub size: [usize; 2],
    /// 読んだときの説明（大きさ、レイヤー、注意）。
    pub info: String,
}

impl BaseArt {
    /// PSD を読む。`back` が真なら、裏の型紙（裏から見た向き）として読む。
    pub fn read(file: project::Asset, back: bool, sheet: [usize; 2]) -> Result<BaseArt, String> {
        let loaded = load_psd(file.bytes())?;
        let merged = if loaded.has_real_merged == Some(false) { "\n注意: この PSD には統合画像が入っていません。" } else { "" };
        let ratio_src = loaded.source_size[0] as f32 / loaded.source_size[1] as f32;
        let ratio_sheet = sheet[0] as f32 / sheet[1] as f32;
        let stretch = if (ratio_src / ratio_sheet - 1.0).abs() > 0.01 { "\n注意: 縦横比が今の展開図と違うので、引き伸ばして貼っています。" } else { "" };
        let guides = match loaded.guides {
            GuideHandling::NotPresent => "",
            GuideHandling::Removed => "\nガイドのレイヤー（と、立体の上で置いた柄のレイヤー）は除いて読み込みました。",
            GuideHandling::Baked => "\n注意: ガイドのレイヤーを除けませんでした（グループ、マスク、通常以外の合成モードがあります）。ガイドを非表示にしてから書き出してください。",
        };
        let side = if back { "紙の裏" } else { "紙の表" };
        let layers = if loaded.layer_names.is_empty() { "なし".to_owned() } else { loaded.layer_names.join("、") };
        let info = format!("{}（{side}）\n大きさ: {} x {} px\nレイヤー: {layers}{merged}{stretch}{guides}", file.name, loaded.source_size[0], loaded.source_size[1]);
        let [w, h] = loaded.source_size;
        // 裏の型紙は裏から見た向きなので、表の向きに直す（柄と同じ座標で重ねるため）。
        let source = if back { mirror(&loaded.source, w, h) } else { loaded.source };
        Ok(BaseArt { file, source: Arc::new(source), size: [w, h], info })
    }

    /// 展開図の大きさに合わせた絵。
    pub fn fitted(&self, sheet: [usize; 2]) -> Vec<u8> {
        fit(&self.source, self.size, sheet[0], sheet[1])
    }
}

/// 同じ絵か（中身の印で見る）。
pub(super) fn same_base(a: &Option<BaseArt>, b: &Option<BaseArt>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.file.id() == b.file.id() && a.file.name == b.file.name,
        _ => false,
    }
}

impl App {
    /// 展開図の大きさに合わせ直す（寸法を変えたとき、絵を替えたとき）。
    pub(super) fn refit_bases(&mut self) {
        let sheet = [self.sheet.w, self.sheet.h];
        self.custom = [0, 1].map(|side| self.bases[side].as_ref().map(|base| base.fitted(sheet)));
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 表（0）か裏（1）の絵を、替える（None なら外す）。`record` が真なら、元に戻せるようにする。
    pub(super) fn set_base(&mut self, side: usize, base: Option<BaseArt>, record: bool) {
        if record {
            self.checkpoint();
        }
        self.psd_info = base.as_ref().map_or_else(String::new, |b| b.info.clone());
        self.bases[side] = base;
        self.refit_bases();
    }

    /// PSD のファイルを読んで、展開図の絵にする。パケノバの裏の型紙（「紙の裏」のレイヤーがある）なら裏、それ以外は表。
    pub(super) fn open_psd(&mut self, path: &Path) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.psd_info = format!("{} を読めませんでした: {e}", path.display());
                return self.say(self.psd_info.clone());
            }
        };
        let back = crate::art::psd_is_back(&bytes);
        self.load_base(path, bytes, if back { 1 } else { 0 });
    }

    /// 決めた側に、PSD を読み込む。
    pub(super) fn load_base(&mut self, path: &Path, bytes: Vec<u8>, side: usize) {
        let name = path.file_name().map_or_else(|| "展開図の絵".to_owned(), |n| n.to_string_lossy().into_owned());
        let file = project::Asset::new(name.clone(), bytes, [0.0, 0.0], file::now_local());
        match BaseArt::read(file, side == 1, [self.sheet.w, self.sheet.h]) {
            Ok(base) => {
                let replaced = self.bases[side].is_some();
                self.set_base(side, Some(base), true);
                let where_ = if side == 1 { "紙の裏" } else { "紙の表" };
                self.say(if replaced { format!("{where_}の展開図の絵を、{name} に差し替えました。Ctrl+Z で戻せます。") } else { format!("{name} を、{where_}の展開図の絵にしました。") });
            }
            Err(e) => {
                self.psd_info = format!("{}\n{e}", path.display());
                self.say(format!("{name} を読めませんでした: {e}"));
            }
        }
    }

    /// 差し替える PSD を選んでもらう。
    pub(super) fn ask_base(&mut self, side: usize) -> Option<PathBuf> {
        if let Some(answer) = self.answer_replace.take() {
            return Some(answer);
        }
        let title = if side == 1 { "紙の裏の展開図の絵（PSD）を選ぶ" } else { "展開図の絵（PSD）を選ぶ" };
        rfd::FileDialog::new().set_title(title).add_filter("PSD", &["psd", "PSD"]).pick_file()
    }

    /// レイヤーの欄の、展開図の絵の行（差し替える、外す）。
    pub(super) fn base_layer(&mut self, ui: &mut egui::Ui, side: usize) {
        let Some(base) = &self.bases[side] else { return };
        let name = base.file.name.clone();
        let mut replace = false;
        let mut remove = false;
        let info = base.info.clone();
        // 欄の幅を広げないよう、名前は切り詰めて出し、ボタンは次の行へ回せるようにする。
        ui.add(egui::Label::new(format!("展開図の絵: {name}")).truncate()).on_hover_text(info);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(12.0);
            if ui.small_button("差し替える…").clicked() {
                replace = true;
            }
            if ui.small_button("外す").on_hover_text("Ctrl+Z で戻せます").clicked() {
                remove = true;
            }
        });
        if remove {
            self.set_base(side, None, true);
            self.say(format!("展開図の絵（{name}）を外しました。Ctrl+Z で戻せます。"));
        } else if replace && let Some(path) = self.ask_base(side) {
            match std::fs::read(&path) {
                Ok(bytes) => self.load_base(&path, bytes, side),
                Err(e) => self.say(format!("{} を読めませんでした: {e}", path.display())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn 展開図の絵は_表と裏に読み込めて_差し替えと外すを元に戻せ_案件に入る() {
        let dir = std::env::temp_dir().join(format!("pakenoba-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        let (w, h) = (app.sheet.w, app.sheet.h);
        // 表の型紙と、裏の型紙を書き出して、読み込む。
        let front = crate::art::save_psd(&app.sheet, None, &[]).unwrap();
        let back = crate::art::save_back_psd(&app.sheet, None, &[]).unwrap();
        assert!(!crate::art::psd_is_back(&front));
        assert!(crate::art::psd_is_back(&back));
        std::fs::write(dir.join("表.psd"), &front).unwrap();
        std::fs::write(dir.join("裏.psd"), &back).unwrap();
        app.open_psd(&dir.join("表.psd"));
        app.open_psd(&dir.join("裏.psd"));
        assert!(app.bases[0].is_some() && app.bases[1].is_some());
        assert_eq!(app.custom[1].as_ref().map(Vec::len), Some(w * h * 4));
        // 裏の絵は、表の向きに直して持つ（紙の形が、表の絵と同じ場所にある）。
        let solid = |rgba: &[u8]| (0..w * h).filter(|&i| rgba[i * 4 + 3] > 0).count();
        let (a, b) = (app.custom[0].as_ref().unwrap(), app.custom[1].as_ref().unwrap());
        let agree = (0..w * h).filter(|&i| (a[i * 4 + 3] > 0) == (b[i * 4 + 3] > 0)).count();
        assert!(agree as f32 > 0.99 * (w * h) as f32, "裏の絵の向きが違う: {agree} / {}（表 {}、裏 {}）", w * h, solid(a), solid(b));
        // 外すと、元に戻せる。
        app.set_base(1, None, true);
        assert!(app.bases[1].is_none());
        app.undo();
        assert!(app.bases[1].is_some(), "外したのを戻せる");
        // 差し替えも、元に戻せる。
        let other = crate::art::tests_psd(w, h, false);
        std::fs::write(dir.join("別の絵.psd"), &other).unwrap();
        app.load_base(&dir.join("別の絵.psd"), other, 0);
        assert_eq!(app.bases[0].as_ref().unwrap().file.name, "別の絵.psd");
        app.undo();
        assert_eq!(app.bases[0].as_ref().unwrap().file.name, "表.psd");
        // 保存して開くと、表と裏の絵が戻る。
        let project = app.project().unwrap();
        assert_eq!(project.back_base.as_ref().map(|b| b.name.as_str()), Some("裏.psd"));
        let mut again = App::new(&Initial::default());
        again.apply_project(project);
        assert_eq!(again.bases.each_ref().map(|b| b.as_ref().map(|b| b.file.name.clone())), [Some("表.psd".to_owned()), Some("裏.psd".to_owned())]);
        assert_eq!(again.custom[1], app.custom[1]);
        // 裏の型紙を書き出すと、紙の裏のレイヤーに、読み込んだ裏の絵が入る（裏から見た向きに戻して）。
        let written = crate::art::save_back_psd(&again.sheet, again.custom[1].as_deref(), &[]).unwrap();
        let reread = crate::art::load_psd(&written).unwrap();
        assert_eq!(mirror(&reread.source, w, h).len(), again.custom[1].as_ref().unwrap().len());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
