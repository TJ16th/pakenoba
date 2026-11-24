//! 書き出し。いまは、型紙の PSD（紙の表と、裏に柄があれば紙の裏）。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};

use super::App;
use crate::paint::Picture;
use crate::art::{BleedZone, DecalLayer, Frame, Plate, SheetArt, add_spot_channels, alpha_over, bleed_zone, draw_bleed_line, extend_edges, make_sheet_art_at, mirror_gray, save_back_psd, save_psd};
use crate::paint::{Canvas, Marks, paint};

/// 別のスレッドで動かす書き出しの種類。
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ExportKind {
    Psd,
    Ready,
}

/// 書き出しに要る、案件の写し（別のスレッドへ渡す）。
pub(super) struct ExportCopy {
    dims: super::Dims,
    decals: Vec<crate::paint::Placed>,
    finishes: Vec<pakenoba_core::material::Finish>,
    bases: [Option<super::base::BaseArt>; 2],
    output: super::outputs::OutputSettings,
    project: Option<PathBuf>,
}

/// 動いている書き出し。
pub(super) struct ExportJob {
    kind: ExportKind,
    path: PathBuf,
    started: std::time::Instant,
    handle: std::thread::JoinHandle<Result<String, String>>,
}

/// 書き出しの細かさ（dpi）の選択肢。100 は画面と同じ（1 mm 4 画素）。
pub(super) const EXPORT_DPI: [u32; 4] = [100, 300, 350, 600];

/// 書き出し用の、細かさを決めた展開図（紙、ガイド線、読み込んだ展開図の絵）。
pub(super) struct OutSheet {
    pub sheet: SheetArt,
    pub custom: [Option<Vec<u8>>; 2],
    /// 塗り足し（幅が 0 なら None）。
    pub zone: Option<BleedZone>,
}

impl OutSheet {
    pub fn custom(&self) -> [Option<&[u8]>; 2] {
        [self.custom[0].as_deref(), self.custom[1].as_deref()]
    }
}

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
    /// 書き出しの細かさ（出力の設定の dpi）。
    /// 塗り足しの幅（mm）のぶん、まわりに余白を足す。
    pub(super) fn export_frame(&self) -> Frame {
        let pad = self.output.bleed.max(0.0);
        match self.output.dpi {
            0..=100 => Frame::of(&self.template, crate::art::PX_PER_MM, pad),
            dpi => Frame::dpi(&self.template, dpi as f32, pad),
        }
    }

    /// 書き出しの細かさで、展開図を作り直す（画面の細かさなら、そのまま写す）。
    pub(super) fn out_sheet(&self) -> OutSheet {
        let frame = self.export_frame();
        let mut sheet = make_sheet_art_at(&self.template, frame);
        let custom = [0, 1].map(|side| self.bases[side].as_ref().map(|base| base.fitted(&self.template, frame)));
        let zone = (self.output.bleed > 0.0).then(|| bleed_zone(&self.template, &sheet, self.output.bleed));
        if let Some(zone) = &zone {
            // 型紙のガイドに、塗り足しの線（赤）を足す。
            let paper = std::mem::take(&mut sheet.art);
            draw_bleed_line(&mut sheet.lines, &paper, frame, zone);
            draw_bleed_line(&mut sheet.guides, &paper, frame, zone);
            sheet.art = paper;
        }
        OutSheet { sheet, custom, zone }
    }

    /// 型紙の PSD の 1 面ぶん（柄のレイヤーと、統合した絵）。塗り足しで、柄が届かない所は、端を伸ばした絵を
    /// いちばん下の柄のレイヤー「塗り足し（端を伸ばした所）」にする。
    pub(super) fn psd_side(&self, out: &OutSheet, back: bool) -> (Vec<DecalLayer>, Vec<u8>) {
        let (sheet, custom, zone) = (&out.sheet, out.custom(), out.zone.as_ref());
        let mut layers = self.decal_layers_in(sheet, custom, zone, back);
        let mut merged = self.painted_in(sheet, custom, zone, back);
        if let Some(zone) = zone {
            let frame = sheet.frame;
            let mut printed = custom[usize::from(back)].map_or_else(|| vec![0u8; frame.w * frame.h * 4], <[u8]>::to_vec);
            paint(&self.surface, &self.template, &self.decals, Marks::default(), back, Canvas::bled(frame, Some(zone), &mut printed));
            let mut ext = vec![0u8; printed.len()];
            extend_edges(&mut printed, frame, zone, Some(&mut ext));
            if ext.chunks_exact(4).any(|p| p[3] > 0) {
                for (m, e) in merged.chunks_exact_mut(4).zip(ext.chunks_exact(4)) {
                    if e[3] > 0 {
                        let mut under = [e[0], e[1], e[2], e[3]];
                        alpha_over(&mut under, m);
                        m.copy_from_slice(&under);
                    }
                }
                layers.insert(0, DecalLayer::shown(t!("Bleed (extended edges)").to_owned(), ext));
            }
        }
        (layers, merged)
    }

    /// 書き出しの細かさが文字の絵より細かければ、文字を描き直してから `f` を呼び、あとで元に戻す。
    /// 描き直せなかった文字は、そのまま（画面の細かさ）で書き出す。
    pub(super) fn with_sharp_text<R>(&mut self, f: impl FnOnce(&Self) -> R) -> R {
        let ppm = self.export_frame().ppm;
        if ppm <= crate::text::SCREEN_TEXT_PPM * 1.05 {
            return f(self);
        }
        let mut kept = Vec::new();
        for index in 0..self.decals.len() {
            if let Picture::Text(art) = &self.decals[index].picture
                && let Ok(sharp) = self.fonts.render_at(&art.spec, ppm, crate::text::EXPORT_MAX_PIXELS)
            {
                let old = std::mem::replace(&mut self.decals[index].picture, Picture::Text(std::sync::Arc::new(sharp)));
                kept.push((index, old));
            }
        }
        let result = f(self);
        for (index, old) in kept {
            self.decals[index].picture = old;
        }
        result
    }

    /// 書き出しの細かさに対して、置いた画像の細かさが足りないもの（名前と、実寸での dpi）。
    pub(super) fn coarse_images(&self) -> Vec<(String, u32)> {
        let dpi = self.export_frame().dpi_value();
        let need = dpi.min(300.0) * 0.8;
        let mut out = Vec::new();
        for (index, decal) in self.decals.iter().enumerate() {
            if let Picture::Image { w, h, vector: false, .. } = &decal.picture
                && !decal.layer.hidden
            {
                let real = (*w as f32 / (decal.size[0] / 25.4)).min(*h as f32 / (decal.size[1] / 25.4));
                if real < need {
                    out.push((self.layer_label(index), real.round() as u32));
                }
            }
        }
        out
    }

    /// 細かさの足りない画像があれば、知らせる文（先頭に改行）。なければ空。
    pub(super) fn coarse_note(&self) -> String {
        let coarse = self.coarse_images();
        if coarse.is_empty() {
            return String::new();
        }
        let list: Vec<String> = coarse.iter().map(|(name, dpi)| format!("{name} ({dpi} dpi)")).collect();
        format!("\n{}", t!("These images are coarse at actual size: {}", list.join(", ")))
    }

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
    /// 書き出しは、別のスレッドで動かす（画面を止めない）。
    pub(super) fn export_psd(&mut self) {
        let Some(path) = self.ask_export() else { return };
        self.start_export(ExportKind::Psd, path);
    }

    /// 書き出しを、別のスレッドで始める。案件の写し（文字は書き出しの細かさで描き直したもの）を渡す。
    pub(super) fn start_export(&mut self, kind: ExportKind, path: PathBuf) {
        if self.export_job.is_some() {
            self.say(t!("Another export is running. Wait until it has finished."));
            return;
        }
        let copy = ExportCopy { dims: self.dims, decals: self.with_sharp_text(|app| app.decals.clone()), finishes: self.finishes.clone(), bases: self.bases.clone(), output: self.output.clone(), project: self.file.path.clone() };
        let target = path.clone();
        let handle = std::thread::spawn(move || {
            let app = App::headless(copy);
            match kind {
                ExportKind::Psd => app.psd_message(&target),
                ExportKind::Ready => app.ready_message(&target),
            }
        });
        self.export_job = Some(ExportJob { kind, path, started: std::time::Instant::now(), handle });
    }

    /// 書き出しが終わっていたら、結果を知らせる。動いている間は、画面を時々描き直す（かかった時間を出すため）。
    pub(super) fn poll_export(&mut self, ctx: &eframe::egui::Context) {
        let Some(job) = &self.export_job else { return };
        if !job.handle.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        let job = self.export_job.take().expect("export job present");
        match job.handle.join() {
            Ok(Ok(message)) | Ok(Err(message)) => self.say(message),
            Err(_) => self.say(t!("The export stopped unexpectedly: {}", job.path.display())),
        }
    }

    /// 書き出しが動いていれば、そう出して真を返す（保存のボタンの代わり）。
    pub(super) fn export_busy(&self, ui: &mut eframe::egui::Ui) -> bool {
        let Some(job) = &self.export_job else { return false };
        ui.horizontal(|ui| {
            ui.spinner();
            let what = match job.kind {
                ExportKind::Psd => t!("Saving the dieline PSD"),
                ExportKind::Ready => t!("Saving the print-ready PDF"),
            };
            ui.label(t!("{what}… {} s", job.started.elapsed().as_secs()));
        });
        true
    }

    /// 書き出し専用の、画面を出さない作業用の App（別のスレッドで作る）。
    fn headless(copy: ExportCopy) -> App {
        let mut app = App::new(&super::Initial { quiet: true, ..Default::default() });
        let (template, surface, _) = copy.dims.build();
        (app.dims, app.built_dims, app.template, app.surface) = (copy.dims, Some(copy.dims), template, surface);
        app.sheet = crate::art::make_sheet_art(&app.template);
        (app.decals, app.finishes, app.bases, app.output) = (copy.decals, copy.finishes, copy.bases, copy.output);
        app.file.path = copy.project;
        app.refit_bases();
        app
    }

    /// 型紙の PSD を書き、知らせる文を返す。
    pub(super) fn psd_message(&self, path: &Path) -> Result<String, String> {
        let files = self.psd_files(path)?;
        let others: Vec<String> = files[1..].iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
        let mut message = if others.is_empty() { t!("Saved: {}", path.display()) } else { t!("Saved: {} (and {})", path.display(), others.join(", ")) };
        message.push_str(&self.coarse_note());
        Ok(message)
    }

    /// 型紙の PSD を、この場で書く（試験用）。
    #[cfg(test)]
    pub(super) fn write_psds(&mut self, path: &Path) {
        match self.with_sharp_text(|app| app.psd_message(path)) {
            Ok(message) | Err(message) => self.say(message),
        }
    }

    /// 型紙の PSD を書き、書いたファイルを返す（表と、裏に柄があれば裏）。
    /// 加工があれば、版をスポットカラーチャンネルとして入れ、指示書（`名前_finishes.txt`）も横に置く。
    pub(super) fn psd_files(&self, path: &Path) -> Result<Vec<PathBuf>, String> {
        let out = self.out_sheet();
        let (sheet, custom) = (&out.sheet, out.custom());
        let front = self.finish_plates(false, sheet.frame);
        let (layers, merged) = self.psd_side(&out, false);
        save_psd(sheet, custom[0], &layers, Some(&merged))
            .and_then(|bytes| add_spot_channels(bytes, &front))
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))
            .map_err(|e| t!("Could not save: {e}"))?;
        let mut files = vec![path.to_owned()];
        if self.has_back_decals() {
            let back = back_path(path);
            let plates: Vec<Plate> = self.finish_plates(true, sheet.frame).into_iter().map(|p| Plate { ink: mirror_gray(&p.ink, sheet.w), ..p }).collect();
            let (layers, merged) = self.psd_side(&out, true);
            save_back_psd(sheet, custom[1], &layers, Some(&merged))
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
        app.output.dpi = 100;
        app.output.bleed = 0.0;
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

    #[test]
    fn 書き出しは_選んだ解像度で_実寸が合う() {
        let dir = std::env::temp_dir().join(format!("pakenoba-dpi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_decal(Picture::Logo, [46.0, 46.0]);
        app.output.dpi = 300;
        app.output.bleed = 0.0;
        let path = dir.join("hi.psd");
        app.write_psds(&path);
        let file = craft_psd::PsdFile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        let frame = app.export_frame();
        assert_eq!((file.header.width as usize, file.header.height as usize), (frame.w, frame.h));
        // 展開図の幅（mm）と、画素の数と解像度から求めた幅が合う。
        let mm = file.header.width as f64 / (file.resolution().unwrap().h_res_fixed as f64 / 65536.0) * 25.4;
        let width = (app.template.max[0] - app.template.min[0]) as f64;
        assert!((mm - width).abs() < 0.2, "{mm} mm / {width} mm");
        assert!(frame.w > app.sheet.w * 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 塗り足しつきの型紙に描いた絵は_塗り足しごと読み戻す() {
        let dir = std::env::temp_dir().join(format!("pakenoba-bleed-psd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        app.output.dpi = 300;
        app.output.bleed = 3.0;
        let path = dir.join("bleed.psd");
        app.write_psds(&path);
        let frame = app.export_frame();
        let file = craft_psd::PsdFile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((file.header.width as usize, file.header.height as usize), (frame.w, frame.h));
        // 読み込むと、余白を 3 mm と見て、画面では切り落とす。書き出すときは、塗り足しのぶんも入る。
        app.open_psd(&path);
        let base = app.bases[0].as_ref().unwrap();
        assert!((base.pad - 3.0).abs() < 0.2, "{}", base.pad);
        assert_eq!(app.custom[0].as_ref().map(Vec::len), Some(app.sheet.w * app.sheet.h * 4));
        let out = app.out_sheet();
        assert_eq!(out.custom[0].as_ref().map(Vec::len), Some(frame.w * frame.h * 4));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 書き出しは_別のスレッドで動き_終わると知らせる() {
        let dir = std::env::temp_dir().join(format!("pakenoba-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_decal(Picture::Logo, [46.0, 46.0]);
        app.output.dpi = 100;
        let ctx = eframe::egui::Context::default();
        let path = dir.join("job.psd");
        app.start_export(ExportKind::Psd, path.clone());
        // 動いている間は、もう 1 つ始めない。
        app.start_export(ExportKind::Ready, dir.join("job.pdf"));
        assert!(!dir.join("job.pdf").exists());
        for _ in 0..600 {
            app.poll_export(&ctx);
            if app.export_job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(app.export_job.is_none(), "終わらない");
        assert!(path.exists());
        let file = craft_psd::PsdFile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert!(file.iter_layers().any(|l| l.name().contains("Logo")));
        app.start_export(ExportKind::Ready, dir.join("job.pdf"));
        while app.export_job.is_some() {
            app.poll_export(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(dir.join("job.pdf").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
