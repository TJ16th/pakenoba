//! 詳しい編集: 柄の素材を、外のアプリ（PhotoCraft、Photoshop など）で開いて直す。
//!
//! 柄をダブルクリック（または Ctrl+E、右の欄の「外のアプリで編集」）すると、素材の中身を作業用のファイルに書き出して、
//! 外のアプリで開く。外のアプリで保存すると、パケノバが気づいて、その素材を使っている柄をすべて新しい絵にする（元に戻せる）。
//! 作業用のファイルは、使う人のデータのフォルダの `edit` に置く（案件のフォルダは汚さない）。
//! 文字の柄は、右の欄の文字の入力へ移る。付属の見本は直せない。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui;

use super::App;
use super::assets::Replace;
use crate::paint::Picture;

/// 外のアプリで開いている素材 1 つ。
pub(super) struct EditJob {
    /// 素材の欄での番号。
    pub asset: usize,
    pub path: PathBuf,
    /// 最後に取り込んだときの、ファイルの更新日時と大きさ。
    seen: Option<(SystemTime, u64)>,
    /// 変わったのに気づいたときの値。次に見たときも同じなら、書き終わったとみなして取り込む。
    pending: Option<(SystemTime, u64)>,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// 作業用のファイルを置くフォルダ。
pub(super) fn edit_dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pakenoba-edit-test-{}", std::process::id()));
    }
    super::settings::data_dir().unwrap_or_else(std::env::temp_dir).join("edit")
}

/// ファイルを、決めたアプリ（なければ、OS でその種類に決まっているアプリ）で開く。
fn open_with(app: Option<&Path>, file: &Path) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    match app {
        // OS でその種類に決まっているアプリで開く（Windows はエクスプローラー、macOS は open、ほかは xdg-open に頼む）。
        None => {
            let launcher = if cfg!(windows) {
                "explorer"
            } else if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            std::process::Command::new(launcher).arg(file).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        Some(app) if cfg!(target_os = "macos") && app.extension().is_some_and(|e| e == "app") => {
            std::process::Command::new("open").arg("-a").arg(app).arg(file).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        Some(app) => std::process::Command::new(app).arg(file).spawn().map(|_| ()).map_err(|e| e.to_string()),
    }
}

impl App {
    /// 選んでいる柄を、詳しく編集する。
    pub(super) fn edit_selected(&mut self) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else { return };
        match &self.decals[index].picture {
            Picture::Text(_) => {
                // 文字は、右の欄で打つ。
                self.focus_text = true;
            }
            Picture::Shape(_) => self.say(t!("Change a shape's form and colors in the right panel, or in the options bar with the Shape tool.")),
            Picture::Logo | Picture::Band => self.say(t!("Built-in samples cannot be edited in another app. Use \"Replace this artwork only\" to switch to another image.")),
            Picture::Image { .. } => match self.asset_of(&self.decals[index].picture.clone()) {
                Some(asset) => self.start_edit(asset),
                None => self.say(t!("The asset for this artwork was not found.")),
            },
            Picture::Group { .. } | Picture::Adjustment => {}
        }
    }

    /// 素材を直すアプリ。ベクター（SVG）と画像で、別々に選べる。
    fn editor_for(&self, picture: &Picture) -> Option<&Path> {
        if matches!(picture, Picture::Image { vector: true, .. }) { self.settings.vector_editor.as_deref() } else { self.settings.editor.as_deref() }
    }

    /// 素材を外のアプリで開く。すでに開いていれば、もう一度開くだけ。
    pub(super) fn start_edit(&mut self, asset: usize) {
        let Some(stored) = self.assets.get(asset).and_then(|a| a.stored.clone()) else { return self.say(t!("This asset cannot be edited in another app.")) };
        let editor = self.editor_for(&self.assets[asset].picture).map(Path::to_path_buf);
        if let Some(job) = self.edits.iter().find(|j| j.asset == asset) {
            let path = job.path.clone();
            if let Err(e) = open_with(editor.as_deref(), &path) {
                self.say(t!("Could not open it in the other app: {e}"));
            }
            return;
        }
        let dir = edit_dir();
        // 名前は「中身の印の頭 8 文字-素材の名前」（同じ名前の素材があっても、ぶつからないように）。
        let path = dir.join(format!("{}-{}", &stored.id()[..8], self.assets[asset].name));
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, stored.bytes().as_slice()));
        if let Err(e) = written {
            return self.say(t!("Could not write the working file: {e}"));
        }
        match open_with(editor.as_deref(), &path) {
            Ok(()) => {
                self.edits.push(EditJob { asset, seen: stamp(&path), pending: None, path });
                self.say(t!("Opened \"{}\" in another app. Saving there updates the box.", self.assets[asset].name));
            }
            Err(e) => self.say(t!("Could not open it in the other app: {e}")),
        }
    }

    /// その素材を、外のアプリで開いているか。
    pub(super) fn is_editing(&self, picture: &Picture) -> bool {
        self.asset_of(picture).is_some_and(|asset| self.edits.iter().any(|j| j.asset == asset))
    }

    /// 外のアプリでの編集をやめる（作業用のファイルは消す。箱の絵はそのまま）。
    pub(super) fn stop_edit(&mut self, asset: usize) {
        if let Some(position) = self.edits.iter().position(|j| j.asset == asset) {
            let job = self.edits.remove(position);
            let _ = std::fs::remove_file(&job.path);
        }
    }

    /// 外のアプリで保存されたかを見る（0.5 秒ごと）。書き終わったものを取り込む。
    pub(super) fn poll_edits(&mut self, ctx: &egui::Context) {
        if self.edits.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let now = ctx.input(|i| i.time);
        if now - self.edit_polled < 0.5 {
            return;
        }
        self.edit_polled = now;
        self.check_edits();
    }

    /// 作業用のファイルが変わっていたら取り込む。変わったのに気づいた回は待ち、次も同じなら取り込む（書きかけを読まないため）。
    pub(super) fn check_edits(&mut self) {
        for i in 0..self.edits.len() {
            let Some(now) = stamp(&self.edits[i].path) else { continue };
            let job = &mut self.edits[i];
            if Some(now) == job.seen {
                job.pending = None;
                continue;
            }
            if job.pending != Some(now) {
                job.pending = Some(now);
                continue;
            }
            // 書き終わった。読めたら、素材を差し替える（名前は元のまま）。
            let (asset, path) = (job.asset, job.path.clone());
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if super::decode_image(&bytes).is_err() {
                // まだ書いている途中か、読めない形。次の回にもう一度見る。
                job.pending = None;
                continue;
            }
            job.seen = Some(now);
            job.pending = None;
            let Some(name) = self.assets.get(asset).map(|a| a.name.clone()) else { continue };
            self.replace_with(Replace::Asset(asset), &path);
            if let Some(a) = self.assets.get_mut(asset) {
                a.name = name.clone();
                if let Some(stored) = &mut a.stored {
                    stored.name = name.clone();
                }
            }
            self.say(t!("Updated the box with \"{name}\" as edited in the other app. Press Ctrl+Z to undo."));
        }
    }

    /// 環境設定の、外のアプリで編集するときに開くアプリ（画像とベクターで別々）。
    pub(super) fn editor_apps_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(t!("Apps for editing"), |ui| {
            ui.set_min_width(300.0);
            for vector in [false, true] {
                self.editor_app_row(ui, vector);
            }
        });
    }

    fn editor_app_row(&mut self, ui: &mut egui::Ui, vector: bool) {
        ui.label(egui::RichText::new(if vector { t!("Vectors (SVG)") } else { t!("Images") }).strong());
        let (tip, title) = if vector { (t!("VectorCraft, Illustrator, Affinity and others"), t!("Choose the app for editing vectors (SVG)")) } else { (t!("PhotoCraft, Photoshop, Affinity and others"), t!("Choose the app for editing images")) };
        let current = if vector { self.settings.vector_editor.clone() } else { self.settings.editor.clone() };
        let app = current.as_ref().and_then(|p| p.file_stem()).map_or_else(|| t!("the system default app").to_owned(), |n| n.to_string_lossy().into_owned());
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            ui.small(t!("Opens with: {app}"));
            if ui.small_button(t!("Choose…")).on_hover_text(tip).clicked()
                && let Some(path) = rfd::FileDialog::new().set_title(title).pick_file()
            {
                chosen = Some(Some(path));
            }
            if current.is_some() && ui.small_button(t!("Reset")).on_hover_text(t!("Open with the system default app")).clicked() {
                chosen = Some(None);
            }
        });
        if let Some(path) = chosen {
            if vector {
                self.settings.vector_editor = path;
            } else {
                self.settings.editor = path;
            }
            if let Err(e) = self.settings.save() {
                self.say(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&color.repeat((w * h) as usize)).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn 外のアプリで保存すると_書き終わってから取り込み_名前は保つ() {
        let dir = std::env::temp_dir().join(format!("pakenoba-edit-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ロゴ.png"), png(4, 2, [255, 0, 0, 255])).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_image(&dir.join("ロゴ.png"));
        let index = app.decals.len() - 1;
        app.select_only(Some(index));
        app.edit_selected();
        assert_eq!(app.edits.len(), 1);
        let path = app.edits[0].path.clone();
        assert!(path.starts_with(edit_dir()));
        assert!(app.is_editing(&app.decals[index].picture));
        // 変わっていなければ、何もしない。
        app.check_edits();
        let before = app.decals[index].size;
        // 外のアプリが保存した（縦横の比が変わった）。1 回目は待ち、2 回目で取り込む。
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, png(4, 4, [0, 0, 255, 255])).unwrap();
        app.check_edits();
        assert_eq!(app.decals[index].size, before, "気づいた回は、まだ取り込まない");
        app.check_edits();
        let Picture::Image { w, h, .. } = &app.decals[index].picture else { panic!() };
        assert_eq!([*w, *h], [4, 4]);
        assert_eq!(app.assets.iter().filter(|a| !a.bundled).map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["ロゴ.png"], "名前は元のまま");
        assert!(app.is_editing(&app.decals[index].picture), "差し替えたあとも、編集は続く");
        // 元に戻せる。
        app.undo();
        let Picture::Image { w, h, .. } = &app.decals[index].picture else { panic!() };
        assert_eq!([*w, *h], [4, 2]);
        // 編集をやめると、作業用のファイルは消える。
        let asset = app.edits[0].asset;
        app.stop_edit(asset);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(edit_dir());
    }
}
