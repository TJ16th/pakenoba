//! 書き出し。いまは、型紙の PSD（紙の表と、裏に柄があれば紙の裏）。

use std::path::{Path, PathBuf};

use super::App;
use crate::art::{save_back_psd, save_psd};

/// 裏の型紙のファイルの名前。表の名前に「_裏」を足す（例: 箱_型紙.psd → 箱_型紙_裏.psd）。
pub(super) fn back_path(front: &Path) -> PathBuf {
    let stem = front.file_stem().map_or_else(|| "型紙".to_owned(), |s| s.to_string_lossy().into_owned());
    front.with_file_name(format!("{stem}_裏.psd"))
}

/// 名前に `.psd` が付いていなければ、足す。
fn with_psd(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd")) {
        return path;
    }
    let mut name = path.file_name().map(|n| n.to_owned()).unwrap_or_default();
    name.push(".psd");
    path.with_file_name(name)
}

impl App {
    /// 裏に置いた柄があるか。
    pub(super) fn has_back_decals(&self) -> bool {
        self.decals.iter().any(|decal| decal.back) || self.bases[1].is_some()
    }

    /// 保存先を選んでもらう。はじめの名前は「案件の名前_型紙.psd」。
    fn ask_export(&mut self) -> Option<PathBuf> {
        if let Some(answer) = self.answer_export.take() {
            return Some(with_psd(answer));
        }
        let project = self.file.path.as_deref().and_then(Path::file_stem).map_or_else(|| "パケノバ".to_owned(), |s| s.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new().set_title("型紙を PSD で保存").add_filter("Photoshop（PSD）", &["psd"]).set_file_name(format!("{project}_型紙.psd"));
        if let Some(dir) = self.file.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(with_psd)
    }

    /// 型紙を PSD で保存する。裏に柄があれば、裏の型紙も、同じ場所に「_裏」を付けて保存する。
    pub(super) fn export_psd(&mut self) {
        let Some(path) = self.ask_export() else { return };
        self.write_psds(&path);
    }

    /// 型紙の PSD を書く（ダイアログのあと）。
    pub(super) fn write_psds(&mut self, path: &Path) {
        let front = save_psd(&self.sheet, self.custom[0].as_deref(), &self.decal_layers(false)).and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()));
        if let Err(e) = front {
            return self.say(format!("保存できませんでした: {e}"));
        }
        if !self.has_back_decals() {
            return self.say(format!("保存しました: {}", path.display()));
        }
        let back = back_path(path);
        match save_back_psd(&self.sheet, self.custom[1].as_deref(), &self.decal_layers(true)).and_then(|bytes| std::fs::write(&back, bytes).map_err(|e| e.to_string())) {
            Ok(()) => self.say(format!("表と裏の 2 つを保存しました: {}、{}", path.display(), back.file_name().unwrap_or_default().to_string_lossy())),
            Err(e) => self.say(format!("表は保存しましたが、裏を保存できませんでした: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Picture;

    #[test]
    fn 裏の名前は_表の名前に裏を足す() {
        assert_eq!(back_path(Path::new("a/箱_型紙.psd")), PathBuf::from("a/箱_型紙_裏.psd"));
        assert_eq!(with_psd(PathBuf::from("箱")), PathBuf::from("箱.psd"));
        assert_eq!(with_psd(PathBuf::from("箱.PSD")), PathBuf::from("箱.PSD"));
    }

    #[test]
    fn 裏に柄があれば_裏の型紙も保存する() {
        let dir = std::env::temp_dir().join(format!("pakenoba-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_decal(Picture::Logo, [46.0, 46.0]);
        let path = dir.join("箱_型紙.psd");
        app.write_psds(&path);
        assert!(path.exists() && !back_path(&path).exists());
        app.decals[0].back = true;
        app.add_decal(Picture::Band, [120.0, 14.0]);
        app.write_psds(&path);
        let layers = |p: &Path| craft_psd::PsdFile::from_bytes(&std::fs::read(p).unwrap()).unwrap().iter_layers().map(|l| l.name()).collect::<Vec<_>>();
        // 表には帯（柄 2）だけ、裏にはロゴ（柄 1）だけ。
        assert_eq!(layers(&path), ["絵柄", "立体の柄 2（帯）", "ガイド（切り線、折り線、のりしろ）"]);
        assert_eq!(layers(&back_path(&path)), ["紙の裏", "立体の柄 1（ロゴ）", "ガイド（切り線、折り線）"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
