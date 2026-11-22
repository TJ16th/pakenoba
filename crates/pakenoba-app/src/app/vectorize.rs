//! 画像の柄のベクター化。VectorCraft の画像トレースで SVG にし、その柄だけを、SVG の新しい素材に差し替える。
//!
//! トレースは、写真だと数秒かかることがあるので、別のスレッドで行う。その間も画面は動く。
//! 終わったときに、柄が別の絵に変わっていたら（消した、差し替えた、別の案件を開いた）、結果は使わない。
//! 元の素材は変えない。Ctrl+Z で戻せる。

use pakenoba_i18n::t;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use pakenoba_core::project;

use super::assets::{Asset, same_picture};
use super::{App, file};
use crate::paint::Picture;
use crate::vector::{PRESETS, TraceSettings};

/// トレースに渡す絵の、長いほうの画素数の上限。これより大きい画像は、縮めてからトレースする（時間とメモリを抑える）。
const TRACE_LONG: usize = 2500;

/// 別のスレッドでトレースしている、柄 1 つぶん。
pub(super) struct VectorizeJob {
    /// 柄の番号と、始めたときの絵（終わったときに、同じ柄かを確かめる）。
    index: usize,
    source: Picture,
    /// 新しい素材の名前。
    name: String,
    result: Receiver<Result<Done, String>>,
}

/// トレースの結果。
struct Done {
    svg: String,
    paths: usize,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

/// 長いほうが `long` px 以下になるように、画素を平均して縮める。
fn shrink(w: usize, h: usize, rgba: &[u8], long: usize) -> (usize, usize, Vec<u8>) {
    let step = w.max(h).div_ceil(long).max(1);
    if step == 1 {
        return (w, h, rgba.to_vec());
    }
    let (nw, nh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = Vec::with_capacity(nw * nh * 4);
    for ny in 0..nh {
        for nx in 0..nw {
            // アルファで重みをつけて平均する（透明なところの色が、縁ににじまないように）。
            let (mut sum, mut alpha, mut count) = ([0u32; 3], 0u32, 0u32);
            for y in ny * step..((ny + 1) * step).min(h) {
                for x in nx * step..((nx + 1) * step).min(w) {
                    let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    for k in 0..3 {
                        sum[k] += p[k] as u32 * p[3] as u32;
                    }
                    alpha += p[3] as u32;
                    count += 1;
                }
            }
            let a = alpha.max(1);
            out.extend_from_slice(&[(sum[0] / a) as u8, (sum[1] / a) as u8, (sum[2] / a) as u8, (alpha / count.max(1)) as u8]);
        }
    }
    (nw, nh, out)
}

impl App {
    /// 画像の柄 `index` のベクター化を始める。
    pub(super) fn start_vectorize(&mut self, index: usize) {
        if self.vectorizing.is_some() {
            return self.say(t!("Other artwork is being vectorized. Press again when it has finished."));
        }
        let Some(placed) = self.decals.get(index) else { return };
        let Picture::Image { w, h, rgba, vector: false } = &placed.picture else { return };
        let (w, h, rgba, size) = (*w, *h, rgba.clone(), placed.decal.size);
        let base = self.asset_of(&placed.picture).map_or_else(|| t!("Image").to_owned(), |i| self.assets[i].name.clone());
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(stem, _)| stem);
        let name = t!("{stem} (vector).svg");
        let settings = self.trace;
        let (send, result) = std::sync::mpsc::channel();
        let work = move || {
            let (tw, th, small) = shrink(w, h, &rgba, TRACE_LONG);
            let done = crate::vector::trace(tw, th, &small, settings, size).and_then(|traced| {
                let (w, h, rgba) = crate::vector::decode_svg(traced.svg.as_bytes())?;
                Ok(Done { svg: traced.svg, paths: traced.paths, w, h, rgba })
            });
            let _ = send.send(done);
        };
        if let Err(e) = std::thread::Builder::new().name("vectorize".to_owned()).spawn(work) {
            return self.say(t!("Could not start vectorizing: {e}"));
        }
        self.vectorizing = Some(VectorizeJob { index, source: placed.picture.clone(), name, result });
        self.say(t!("Vectorizing… (VectorCraft Image Trace)"));
    }

    /// ベクター化が終わったかを見る。終わっていたら、柄を差し替える。
    pub(super) fn poll_vectorize(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.vectorizing else { return };
        match job.result.try_recv() {
            Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.vectorizing = None;
                self.say(t!("Vectorizing stopped partway."));
            }
            Ok(done) => {
                let Some(job) = self.vectorizing.take() else { return };
                match done {
                    Ok(done) => self.finish_vectorize(job, done),
                    Err(why) => self.say(why),
                }
            }
        }
    }

    /// トレースの結果を、新しい素材にして、その柄だけを差し替える。
    fn finish_vectorize(&mut self, job: VectorizeJob, done: Done) {
        let still = self.decals.get(job.index).is_some_and(|d| same_picture(&d.picture, &job.source));
        if !still {
            return self.say(t!("The artwork changed while it was being vectorized, so the result was not used."));
        }
        let size = self.decals[job.index].decal.size;
        let stored = project::Asset::new(job.name.clone(), done.svg.into_bytes(), size, file::now_local());
        self.checkpoint();
        let known = self.assets.iter().find(|a| a.stored.as_ref().is_some_and(|s| s.id() == stored.id())).map(|a| a.picture.clone());
        let picture = known.unwrap_or_else(|| {
            let picture = Picture::Image { w: done.w, h: done.h, rgba: Arc::new(done.rgba), vector: true };
            self.assets.push(Asset { name: job.name.clone(), picture: picture.clone(), size: stored.size, thumb: None, bundled: false, stored: Some(stored) });
            picture
        });
        self.decals[job.index].picture = picture;
        (self.art_dirty, self.back_dirty) = (true, true);
        self.say(t!("Vectorized ({} shapes). It is a new asset, \"{}\", and the original asset is unchanged. Press Ctrl+Z to undo.", done.paths, job.name));
    }

    /// 画像の調整の欄の、ベクター化の行。
    pub(super) fn vectorize_controls(&mut self, ui: &mut egui::Ui, index: usize, enabled: bool) {
        ui.separator();
        let running = self.vectorizing.as_ref().map(|job| job.index);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(enabled && running.is_none(), |ui| {
                if ui.button(t!("Vectorize")).on_hover_text(t!("Turns it into line art (SVG) with VectorCraft Image Trace. It stays sharp when enlarged and can be edited in VectorCraft")).clicked() {
                    self.start_vectorize(index);
                }
            });
            if running == Some(index) {
                ui.spinner();
                ui.weak(t!("Tracing…"));
            }
        });
        ui.add_enabled_ui(enabled, |ui| {
            egui::Grid::new("trace-settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label(t!("Preset"));
                let current = PRESETS.get(self.trace.preset).map_or("", |p| pakenoba_i18n::tr(p.1));
                egui::ComboBox::from_id_salt("trace-preset").selected_text(current).show_ui(ui, |ui| {
                    for (i, (_, label)) in PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.trace.preset, i, pakenoba_i18n::tr(label));
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.trace.drop_background, t!("Leave out the background")).on_hover_text(t!("Does not make shapes from the area connected to the edges in the most common edge color"));
                ui.end_row();
            });
        });
    }
}

/// はじめの設定。
pub(super) fn default_settings() -> TraceSettings {
    TraceSettings::default()
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Placed;
    use pakenoba_core::surface::Decal;

    /// 白地に、赤い丸。
    fn red_dot(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                if (x - w as f32 / 2.0).hypot(y - h as f32 / 2.0) < h as f32 / 3.0 { [210, 30, 40, 255] } else { [250, 250, 250, 255] }
            })
            .collect()
    }

    /// 終わるまで待つ。
    fn wait(app: &mut App) {
        let ctx = egui::Context::default();
        for _ in 0..600 {
            app.poll_vectorize(&ctx);
            if app.vectorizing.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("ベクター化が終わらない");
    }

    #[test]
    fn ベクター化すると_svg_の素材になり_保存して戻り_元に戻せる() {
        let mut app = App::new(&Initial::default());
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let picture = Picture::Image { w: 80, h: 60, rgba: Arc::new(red_dot(80, 60)), vector: false };
        let decal = Decal { panel, at: [0.5, 0.5], size: [40.0, 30.0], angle: 0.0 };
        app.decals.push(Placed { decal, picture: picture.clone(), back: false, layer: Default::default() });
        let index = app.decals.len() - 1;
        let assets = app.assets.len();
        app.start_vectorize(index);
        wait(&mut app);
        assert!(matches!(app.decals[index].picture, Picture::Image { vector: true, .. }), "{:?}", app.message);
        assert_eq!(app.decals[index].decal.size, [40.0, 30.0], "大きさは変わらない");
        assert_eq!(app.assets.len(), assets + 1);
        let asset = app.assets.last().unwrap();
        assert!(asset.name.ends_with(" (vector).svg"));
        assert!(crate::vector::is_svg(asset.stored.as_ref().unwrap().bytes()));
        // 保存して開き直すと、SVG から描き直して、同じくベクターの柄。
        let project = app.project().unwrap();
        let mut again = App::new(&Initial::default());
        let problems = again.apply_project(project);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(matches!(again.decals[index].picture, Picture::Image { vector: true, .. }));
        // 元に戻すと、元の画像。
        app.undo();
        assert!(same_picture(&app.decals[index].picture, &picture));
        assert_eq!(app.assets.len(), assets);
    }

    #[test]
    fn 途中で柄を消すと_結果は使わない() {
        let mut app = App::new(&Initial::default());
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let picture = Picture::Image { w: 80, h: 60, rgba: Arc::new(red_dot(80, 60)), vector: false };
        let decal = Decal { panel, at: [0.5, 0.5], size: [40.0, 30.0], angle: 0.0 };
        app.decals.push(Placed { decal, picture, back: false, layer: Default::default() });
        let index = app.decals.len() - 1;
        let assets = app.assets.len();
        app.start_vectorize(index);
        app.decals.remove(index);
        wait(&mut app);
        assert_eq!(app.assets.len(), assets);
    }

    #[test]
    fn 大きい画像は縮めてからトレースする() {
        let (w, h, rgba) = shrink(5000, 100, &vec![255u8; 5000 * 100 * 4], TRACE_LONG);
        assert_eq!((w, h), (2500, 50));
        assert_eq!(rgba.len(), w * h * 4);
        assert_eq!(shrink(100, 50, &vec![0u8; 100 * 50 * 4], TRACE_LONG).0, 100);
    }
}
