//! 書き出し。いまは、型紙の PSD（紙の表と、裏に柄があれば紙の裏）。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};

use super::App;
use crate::art::{Plate, add_spot_channels, mirror_gray, save_back_psd, save_psd};

/// 裏の型紙のファイルの名前。表の名前に「_裏」を足す（例: 箱_型紙.psd → 箱_型紙_裏.psd）。
pub(super) fn back_path(front: &Path) -> PathBuf {
    let stem = front.file_stem().map_or_else(|| t!("Dieline").to_owned(), |s| s.to_string_lossy().into_owned());
    front.with_file_name(t!("{stem}_back.psd"))
}

/// 加工の指示書のファイルの名前（例: 箱_型紙.psd → 箱_型紙_finishes.txt）。
pub(super) fn notes_path(front: &Path) -> PathBuf {
    let stem = front.file_stem().map_or_else(|| t!("Dieline").to_owned(), |s| s.to_string_lossy().into_owned());
    front.with_file_name(format!("{stem}_finishes.txt"))
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
        let project = self.file.path.as_deref().and_then(Path::file_stem).map_or_else(|| t!("PAKENOBA").to_owned(), |s| s.to_string_lossy().into_owned());
        let mut dialog = rfd::FileDialog::new().set_title(t!("Save the dieline as PSD")).add_filter(t!("Photoshop (PSD)"), &["psd"]).set_file_name(t!("{project}_dieline.psd"));
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
        match self.psd_files(path) {
            Ok(files) => {
                let others: Vec<String> = files[1..].iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
                if others.is_empty() {
                    self.say(t!("Saved: {}", path.display()));
                } else {
                    self.say(t!("Saved: {} (and {})", path.display(), others.join(", ")));
                }
            }
            Err(e) => self.say(e),
        }
    }

    /// 型紙の PSD を書き、書いたファイルを返す（表と、裏に柄があれば裏）。
    /// 加工があれば、版をスポットカラーチャンネルとして入れ、指示書（`名前_finishes.txt`）も横に置く。
    pub(super) fn psd_files(&self, path: &Path) -> Result<Vec<PathBuf>, String> {
        let front = self.finish_plates(false);
        save_psd(&self.sheet, self.custom[0].as_deref(), &self.decal_layers(false), Some(&self.painted(false)))
            .and_then(|bytes| add_spot_channels(bytes, &front))
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))
            .map_err(|e| t!("Could not save: {e}"))?;
        let mut files = vec![path.to_owned()];
        if self.has_back_decals() {
            let back = back_path(path);
            let plates: Vec<Plate> = self.finish_plates(true).into_iter().map(|p| Plate { ink: mirror_gray(&p.ink, self.sheet.w), ..p }).collect();
            save_back_psd(&self.sheet, self.custom[1].as_deref(), &self.decal_layers(true), Some(&self.painted(true)))
                .and_then(|bytes| add_spot_channels(bytes, &plates))
                .and_then(|bytes| std::fs::write(&back, bytes).map_err(|e| e.to_string()))
                .map_err(|e| t!("Saved the front, but could not save the back: {e}"))?;
            files.push(back);
        }
        if self.finishes.iter().any(|f| !f.hidden) {
            let notes = notes_path(path);
            std::fs::write(&notes, self.finish_notes()).map_err(|e| t!("Saved the PSD, but could not save the finish notes: {e}"))?;
            files.push(notes);
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Picture;

    #[test]
    fn 裏の名前は_表の名前に裏を足す() {
        assert_eq!(back_path(Path::new("a/箱_型紙.psd")), PathBuf::from("a/箱_型紙_back.psd"));
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
        assert_eq!(layers(&path), ["Artwork", "3D artwork 2 (Band)", "Guides (cut lines, fold lines, glue flaps)"]);
        assert_eq!(layers(&back_path(&path)), ["Back of sheet", "3D artwork 1 (Logo)", "Guides (cut lines, fold lines)"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 加工は_スポットカラーチャンネルと指示書で渡す() {
        use pakenoba_core::material::{Finish, Foil};
        let dir = std::env::temp_dir().join(format!("pakenoba-plates-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_decal(Picture::Logo, [46.0, 46.0]);
        let link = app.decals[0].layer.id;
        app.finishes = vec![Finish::foil(Foil::Gold, link), Finish::emboss(link, false), Finish::foil(Foil::Gold, link)];
        app.ensure_finish_ids();
        let path = dir.join("箱_型紙.psd");
        let files = app.psd_files(&path).unwrap();
        assert_eq!(files, [path.clone(), notes_path(&path)]);
        let mut file = craft_psd::PsdFile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        let base = file.header.channels as usize - 2;
        let names: Vec<String> = file.resources.iter().find(|r| r.id == 1006).map(|r| {
            let mut out = Vec::new();
            let mut d = &r.data[..];
            while let Some((&n, rest)) = d.split_first() {
                out.push(String::from_utf8_lossy(&rest[..n as usize]).into_owned());
                d = &rest[n as usize..];
            }
            out
        }).unwrap();
        // 同じ金箔は 1 枚にまとめ、エンボスは別の版。
        assert_eq!(names[names.len() - 2..], ["Foil Gold", "Emboss"]);
        let kinds: Vec<u8> = craft_psd::channel_resources::display_info(&file.resources.iter().find(|r| r.id == 1077).unwrap().data, true).map(|d| d.kind).collect();
        assert_eq!(kinds.len(), names.len());
        assert!(kinds[kinds.len() - 2..].iter().all(|&k| k == 2));
        // 版の中身は、Photoshop と同じく、黒（0）が加工の所。柄の外は白。
        file.image_data.load().unwrap();
        let planes = file.image_data.decode(&file.header).unwrap();
        let n = app.sheet.w * app.sheet.h;
        let foil = &planes[base * n..(base + 1) * n];
        assert!(foil.contains(&0) && foil.contains(&255));
        assert!(std::fs::read_to_string(notes_path(&path)).unwrap().contains("Emboss"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
