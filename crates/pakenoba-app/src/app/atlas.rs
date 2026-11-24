//! 組み立て向きの絵。箱の外側の面を、組み立てたときの向き（正面をまん中に、右の側面は右、天面は上）で 1 枚に並べたもの。
//!
//! 展開図では、フタや底の面が逆さや横向きに並んでいるので、そのまま Photoshop や生成 AI で描くと、組み立てたときに
//! 絵や文字が逆さになる。組み立て向きの PSD に描いてから読み戻すと、パケノバが面ごとに展開図の正しい場所へ戻す。
//!
//! 面ごとの選択範囲（アルファチャンネル）も入れる。Photoshop でチャンネルを Ctrl + クリックすると、その面だけを選べる
//! （生成塗りつぶしを面ごとにかけるため）。型紙の PSD にも、同じく面ごとの選択範囲を入れる。

use pakenoba_i18n::{t, tr, trc};
use std::path::{Path, PathBuf};

use pakenoba_core::surface::{Aff2, Decal};
use pakenoba_core::template::{P2, Template, inside};

use super::App;
use crate::art::{BleedZone, DecalLayer, Frame, GUIDE_LAYER_PREFIX, Plate, SheetArt, add_channels, bleed_zone, build_psd, draw_line, make_sheet_art_at};
use crate::paint::{Canvas, Marks, paint};

/// 組み立て向きの PSD の、ガイドのレイヤーの名前に入れる目印。これがあれば、読み込むときに展開図へ戻す。
pub(crate) const ATLAS_TAG: &str = "(PAKENOBA assembled view)";

/// 組み立て向きの並び。面ごとに、並びの座標（mm、上が正）をそのパネルの座標へ写すもの。
pub(crate) struct Atlas {
    pub places: Vec<(usize, Aff2)>,
    /// 並べた面の全体の範囲（並びの座標）。
    pub lo: P2,
    pub hi: P2,
}

impl Atlas {
    /// 決めた細かさと、まわりの余白（mm）での画像。
    pub fn frame(&self, ppm: f32, pad: f32) -> Frame {
        let w = ((self.hi[0] - self.lo[0] + 2.0 * pad) * ppm).ceil().max(2.0) as usize;
        let h = ((self.hi[1] - self.lo[1] + 2.0 * pad) * ppm).ceil().max(2.0) as usize;
        Frame { left: self.lo[0] - pad, top: self.hi[1] + pad, ppm, w, h }
    }

    /// 並びの点が、どの面のどこか（面の番号と、パネルの座標）。
    fn find(&self, tpl: &Template, q: P2) -> Option<(usize, P2)> {
        self.places.iter().find_map(|&(panel, place)| {
            let p = place.point(q);
            inside(&tpl.panels[panel].outline, p).then_some((panel, p))
        })
    }
}

/// `src`（RGBA、`frame` の画像）の、展開図や並びの点 `p` での色。近い 4 画素をなめらかにつなぐ。
fn sample(src: &[u8], frame: &Frame, p: P2) -> [u8; 4] {
    let [fx, fy] = frame.to_px(p);
    let (fx, fy) = ((fx - 0.5).clamp(0.0, (frame.w - 1) as f32), (fy - 0.5).clamp(0.0, (frame.h - 1) as f32));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(frame.w - 1), (y0 + 1).min(frame.h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let px = |x: usize, y: usize| {
        let o = (y * frame.w + x) * 4;
        let a = src[o + 3] as f32;
        [src[o] as f32 * a, src[o + 1] as f32 * a, src[o + 2] as f32 * a, a]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty;
    }
    if out[3] <= 0.0 {
        return [0; 4];
    }
    [(out[0] / out[3]).round() as u8, (out[1] / out[3]).round() as u8, (out[2] / out[3]).round() as u8, out[3].round().min(255.0) as u8]
}

/// 画像の行を、行の束に分けて、別のスレッドで埋める。`f(画素の番号, その画素の 4 バイト)`。
fn fill_rows(out: &mut [u8], w: usize, f: impl Fn(usize, &mut [u8]) + Sync) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    let h = out.len() / (w * 4);
    let band = h.div_ceil(threads).max(1);
    let f = &f;
    std::thread::scope(|scope| {
        for (i, chunk) in out.chunks_mut(band * w * 4).enumerate() {
            scope.spawn(move || {
                for (j, px) in chunk.chunks_exact_mut(4).enumerate() {
                    f(i * band * w + j, px);
                }
            });
        }
    });
}

/// 展開図の画像（`src`、`src_frame`）を、組み立て向きの並びの画像（`frame`）にする。面の外は透明。
pub(crate) fn to_atlas(src: &[u8], src_frame: &Frame, atlas: &Atlas, tpl: &Template, frame: &Frame) -> Vec<u8> {
    let mut out = vec![0u8; frame.w * frame.h * 4];
    fill_rows(&mut out, frame.w, |i, px| {
        if let Some((_, p)) = atlas.find(tpl, frame.at(i % frame.w, i / frame.w)) {
            px.copy_from_slice(&sample(src, src_frame, p));
        }
    });
    out
}

/// 組み立て向きの画像（`img`、`img_frame`）を、展開図の画像（`frame`）へ戻す。並びにない面（フラップなど）は透明。
/// 塗り足しは、いちばん近い面の、並びの上でその外側に描かれた絵を使う（隣の面へ回り込む絵は、そのまま続く）。
pub(crate) fn from_atlas(img: &[u8], img_frame: &Frame, atlas: &Atlas, tpl: &Template, frame: &Frame, zone: Option<&BleedZone>) -> Vec<u8> {
    let mut out = vec![0u8; frame.w * frame.h * 4];
    fill_rows(&mut out, frame.w, |i, px| {
        let p = frame.at(i % frame.w, i / frame.w);
        let owner = atlas.places.iter().find(|&&(panel, _)| inside(&tpl.panels[panel].outline, p)).or_else(|| {
            let &(panel, _) = zone?.map.get(&(i as u32))?;
            atlas.places.iter().find(|&&(q, _)| q == panel as usize)
        });
        if let Some(&(_, place)) = owner {
            px.copy_from_slice(&sample(img, img_frame, place.inverse().point(p)));
        }
    });
    out
}

/// 面ごとの選択範囲（展開図の上）。白が、その面と、その面の塗り足し。のりしろと、閉じると隠れる面は除く。
pub(crate) fn face_selections(tpl: &Template, frame: &Frame, zone: Option<&BleedZone>) -> Vec<Plate> {
    let usable: Vec<usize> = (0..tpl.panels.len()).filter(|&i| tpl.panels[i].takes_art() && !tpl.panels[i].hidden_when_closed()).collect();
    usable
        .into_iter()
        .map(|panel| {
            let outline = &tpl.panels[panel].outline;
            let mut ink = vec![0u8; frame.w * frame.h];
            let grow = zone.map_or(0.0, |z| z.bleed);
            let lo = outline.iter().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
            let hi = outline.iter().fold([f32::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
            let (x0, x1, y0, y1) = frame.span([lo[0] - grow, lo[1] - grow], [hi[0] + grow, hi[1] + grow]);
            for y in y0..y1 {
                for x in x0..x1 {
                    let o = y * frame.w + x;
                    if inside(outline, frame.at(x, y)) || zone.is_some_and(|z| z.map.get(&(o as u32)).is_some_and(|&(owner, _)| owner as usize == panel)) {
                        ink[o] = 255;
                    }
                }
            }
            Plate { name: trc("panel", tpl.panels[panel].name).to_owned(), rgb: [255, 0, 0], ink }
        })
        .collect()
}

/// 組み立て向きの並びの、面ごとの選択範囲。
fn atlas_selections(atlas: &Atlas, tpl: &Template, frame: &Frame) -> Vec<Plate> {
    atlas
        .places
        .iter()
        .map(|&(panel, place)| {
            let mut ink = vec![0u8; frame.w * frame.h];
            let back = place.inverse();
            let shape: Vec<P2> = tpl.panels[panel].outline.iter().map(|&p| back.point(p)).collect();
            let lo = shape.iter().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
            let hi = shape.iter().fold([f32::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
            let (x0, x1, y0, y1) = frame.span(lo, hi);
            for y in y0..y1 {
                for x in x0..x1 {
                    if inside(&shape, frame.at(x, y)) {
                        ink[y * frame.w + x] = 255;
                    }
                }
            }
            Plate { name: trc("panel", tpl.panels[panel].name).to_owned(), rgb: [255, 0, 0], ink }
        })
        .collect()
}

/// 組み立て向きの PSD のファイルの名前（例: 箱_assembled.psd）。
pub(super) fn atlas_name(project: &str) -> String {
    t!("{project}_assembled.psd")
}

/// 組み立て向きの PSD か（ガイドのレイヤーの名前に目印がある）。
pub(crate) fn psd_is_atlas(bytes: &[u8]) -> bool {
    craft_psd::PsdFile::from_bytes(bytes).is_ok_and(|file| file.iter_layers().any(|layer| layer.name().contains(ATLAS_TAG)))
}

impl App {
    /// いまの箱の、組み立て向きの並び。正面をまん中に、正面がまっすぐ立って見える向き。
    pub(crate) fn atlas(&self) -> Atlas {
        let tpl = &self.template;
        let (front, _) = self.dims.front_and_around(tpl);
        let start = Decal { panel: front, at: [0.5, 0.5], size: [1.0, 1.0], angle: self.surface.up(front, false) }.place(tpl);
        let places = self.surface.unfold(tpl, front, start);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for &(panel, place) in &places {
            let back = place.inverse();
            for &p in &tpl.panels[panel].outline {
                let q = back.point(p);
                (lo, hi) = ([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])]);
            }
        }
        Atlas { places, lo, hi }
    }

    /// 組み立て向きの PSD を作る（紙の表の、箱の外側の面）。細かさと、まわりの余白（塗り足しの幅）は、出力の設定。
    /// レイヤーは下から「絵柄」、立体の上で置いた柄（まとめて 1 枚。読み戻すときは除く）、ガイド（面の輪郭）。
    pub(crate) fn atlas_psd(&self) -> Result<Vec<u8>, String> {
        let out = self.out_sheet();
        let (sheet, frame) = (&out.sheet, out.sheet.frame);
        let tpl = &self.template;
        let atlas = self.atlas();
        let a_frame = atlas.frame(frame.ppm, self.output.bleed.max(0.0));
        // 絵柄（読み込んだ展開図の絵。刷った所だけが色を持つ）と、確認用の紙の色。
        let base = out.custom[0].clone().unwrap_or_else(|| vec![0u8; frame.w * frame.h * 4]);
        let base_a = to_atlas(&base, &frame, &atlas, tpl, &a_frame);
        let paper_a = to_atlas(&sheet.art, &frame, &atlas, tpl, &a_frame);
        // 立体の上で置いた柄（塗り足しまで描いたもの）。
        let mut decals = vec![0u8; frame.w * frame.h * 4];
        paint(&self.surface, tpl, &self.decals, Marks::default(), false, Canvas::bled(frame, out.zone.as_ref(), &mut decals));
        let decals_a = to_atlas(&decals, &frame, &atlas, tpl, &a_frame);
        // ガイド: 面の輪郭。
        let mut lines = vec![0u8; a_frame.w * a_frame.h * 4];
        for &(panel, place) in &atlas.places {
            let back = place.inverse();
            let outline: Vec<P2> = tpl.panels[panel].outline.iter().map(|&p| back.point(p)).collect();
            for i in 0..outline.len() {
                let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
                draw_line(&mut lines, a_frame.w, a_frame.h, a_frame.to_px(a), a_frame.to_px(b), 0.4 * a_frame.ppm, [0, 150, 220, 255]);
            }
        }
        let guides = format!("{} {ATLAS_TAG}", tr(GUIDE_LAYER_PREFIX));
        let mut merged = paper_a.clone();
        crate::art::alpha_over(&mut merged, &base_a);
        crate::art::alpha_over(&mut merged, &decals_a);
        let layer = DecalLayer::shown(t!("All artwork placed in 3D").to_owned(), decals_a);
        let bytes = crate::art::build_psd_on_paper(a_frame.w, a_frame.h, frame.dpi_value(), Some(paper_a), (tr("Artwork"), base_a), vec![layer], (guides, lines), Some(merged))?;
        add_channels(bytes, &[], &atlas_selections(&atlas, tpl, &a_frame))
    }

    /// 組み立て向きの PSD に描いた絵を、展開図の PSD（紙の表の、展開図の絵）にする。
    /// 書き出したときの細かさ（PSD の解像度）と、まわりの余白（画素の数から求める）に合わせて戻す。
    pub(crate) fn atlas_to_dieline(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let loaded = crate::art::load_psd(bytes)?;
        let dpi = loaded.dpi.ok_or_else(|| t!("This assembled-view PSD has no resolution, so it cannot be put back on the dieline.").to_owned())?;
        let tpl = &self.template;
        let atlas = self.atlas();
        let ppm = dpi / 25.4;
        let [w, h] = loaded.source_size;
        let pads = [(w as f32 / ppm - (atlas.hi[0] - atlas.lo[0])) * 0.5, (h as f32 / ppm - (atlas.hi[1] - atlas.lo[1])) * 0.5];
        if (pads[0] - pads[1]).abs() > 1.0 || pads[0] < -1.0 {
            return Err(t!("The assembled-view PSD does not match the current box (the shape or the dimensions have changed since it was saved).").to_owned());
        }
        let pad = ((pads[0] + pads[1]) * 0.5).max(0.0);
        let img_frame = Frame { left: atlas.lo[0] - pad, top: atlas.hi[1] + pad, ppm, w, h };
        let frame = Frame::dpi(tpl, dpi, pad);
        let sheet: SheetArt = make_sheet_art_at(tpl, frame);
        let zone = (pad > 0.0).then(|| bleed_zone(tpl, &sheet, pad));
        let art = from_atlas(&loaded.source, &img_frame, &atlas, tpl, &frame, zone.as_ref());
        let guides = t!("{guides} (cut lines, fold lines)", guides = tr(GUIDE_LAYER_PREFIX));
        build_psd(frame.w, frame.h, dpi, (tr("Artwork"), art), Vec::new(), (guides, sheet.lines), None)
    }

    /// 組み立て向きの PSD を読み、展開図の絵（紙の表）にする。元に戻せる。
    pub(super) fn open_atlas(&mut self, path: &Path, bytes: &[u8]) {
        match self.atlas_to_dieline(bytes) {
            Ok(dieline) => {
                let stem = path.file_stem().map_or_else(|| t!("Dieline artwork").to_owned(), |s| s.to_string_lossy().into_owned());
                let name: PathBuf = path.with_file_name(t!("{stem} (from the assembled view).psd"));
                self.load_base(&name, dieline, 0);
            }
            Err(e) => self.say(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    fn black() -> App {
        let mut app = App::new(&Initial::default());
        let (_, text) = super::super::samples::SAMPLES[3];
        let opened = pakenoba_core::project::Project::from_text(text).unwrap();
        assert!(app.apply_project(opened.project).is_empty());
        app.output.dpi = 100;
        app.output.bleed = 3.0;
        app
    }

    #[test]
    fn 組み立て向きに並べて戻すと_展開図の絵がそのまま戻る() {
        let app = black();
        let out = app.out_sheet();
        let frame = out.sheet.frame;
        let tpl = &app.template;
        let mut art = vec![0u8; frame.w * frame.h * 4];
        paint(&app.surface, tpl, &app.decals, Marks::default(), false, Canvas::at(frame, &mut art));
        let atlas = app.atlas();
        assert_eq!(atlas.places.len(), 6);
        let a_frame = atlas.frame(frame.ppm, 3.0);
        let there = to_atlas(&art, &frame, &atlas, tpl, &a_frame);
        let back = from_atlas(&there, &a_frame, &atlas, tpl, &frame, out.zone.as_ref());
        // 並べた面の中ほど（縁から 1 mm より内側）は、同じ色に戻る。
        let mut checked = 0;
        for &(panel, _) in &atlas.places {
            let outline = &tpl.panels[panel].outline;
            let lo = outline.iter().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
            let hi = outline.iter().fold([f32::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
            for i in 1..10 {
                for j in 1..10 {
                    let p = [lo[0] + 1.0 + (hi[0] - lo[0] - 2.0) * i as f32 / 10.0, lo[1] + 1.0 + (hi[1] - lo[1] - 2.0) * j as f32 / 10.0];
                    if !inside(outline, p) {
                        continue;
                    }
                    let [x, y] = frame.to_px(p);
                    let o = (y as usize * frame.w + x as usize) * 4;
                    for c in 0..4 {
                        assert!((art[o + c] as i32 - back[o + c] as i32).abs() <= 3, "{} の {p:?}: {:?} → {:?}", tpl.panels[panel].name, &art[o..o + 4], &back[o..o + 4]);
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 300, "{checked}");
    }

    #[test]
    fn 組み立て向きの_psd_を読み戻すと_表の展開図の絵になり_面ごとの選択範囲がある() {
        let mut app = black();
        let bytes = app.atlas_psd().unwrap();
        assert!(psd_is_atlas(&bytes));
        let file = craft_psd::PsdFile::from_bytes(&bytes).unwrap();
        // 面ごとの選択範囲（6 面）。
        assert!([9, 10].contains(&(file.header.channels as usize)), "RGB（と透明）と、6 面: {}", file.header.channels);
        // Photoshop で描いたつもりの PSD: 並びの上半分（天面のあたり）を赤で塗ったレイヤーを足す。
        let loaded = crate::art::load_psd(&bytes).unwrap();
        let [w, h] = loaded.source_size;
        let atlas = app.atlas();
        let front = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let place = atlas.places.iter().find(|&&(p, _)| p == front).unwrap().1;
        let front_top = app.template.panels[front].outline.iter().map(|&p| place.inverse().point(p)[1]).fold(f32::MIN, f32::max);
        // 解像度は PSD の中で丸められるので、画素の数は 1 つずれることがある。
        let frame = Frame { w, h, ..atlas.frame(loaded.dpi.unwrap() / 25.4, 3.0) };
        let red: Vec<u8> = (0..w * h).flat_map(|i| if frame.at(i % w, i / w)[1] > front_top + 2.0 { [220, 20, 20, 255] } else { [0, 0, 0, 0] }).collect();
        let guides = format!("{} {ATLAS_TAG}", tr(GUIDE_LAYER_PREFIX));
        let painted = build_psd(w, h, loaded.dpi.unwrap(), ("Generative Fill", red), Vec::new(), (guides, vec![0u8; w * h * 4]), None).unwrap();
        let dir = std::env::temp_dir().join(format!("pakenoba-atlas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("box_assembled.psd");
        std::fs::write(&path, &painted).unwrap();
        app.open_psd(&path);
        let base = app.bases[0].as_ref().expect("表の展開図の絵になる");
        assert!(base.file.name.contains("box_assembled"));
        assert!((base.pad - 3.0).abs() < 0.3, "{}", base.pad);
        // 天面（正面の上に並べた面）は赤く、正面は塗られていない。展開図では、天面は正面の上にはない（背面につながっている）。
        let custom = app.custom[0].as_ref().unwrap();
        let screen = Frame::screen(&app.template);
        let center = |name: &str| {
            let panel = app.template.panels.iter().find(|p| p.name == name).unwrap();
            panel.outline.iter().fold([0.0f32; 2], |s, p| [s[0] + p[0] / panel.outline.len() as f32, s[1] + p[1] / panel.outline.len() as f32])
        };
        let px = |p: P2| {
            let [x, y] = screen.to_px(p);
            let o = (y as usize * screen.w + x as usize) * 4;
            [custom[o], custom[o + 1], custom[o + 2], custom[o + 3]]
        };
        assert_eq!(px(center("正面"))[3], 0);
        let tops: Vec<[u8; 4]> = atlas.places.iter().filter(|&&(p, _)| p != front).filter(|&&(p, pl)| {
            let c = app.template.panels[p].outline.iter().fold([0.0f32; 2], |s, &q| { let q = pl.inverse().point(q); [s[0] + q[0], s[1] + q[1]] });
            c[1] / app.template.panels[p].outline.len() as f32 > front_top + 2.0
        }).map(|&(p, _)| px(center(app.template.panels[p].name))).collect();
        assert!(!tops.is_empty() && tops.iter().all(|c| c[0] > 200 && c[3] == 255), "{tops:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn どの型でも_外側の面を重ならずに並べる() {
        for sides in [0usize, 2, 4, 6, 100] {
            let app = App::new(&Initial { sides, ..Default::default() });
            let atlas = app.atlas();
            assert!(atlas.places.len() >= 3, "型 {sides}: {}", atlas.places.len());
            let frame = atlas.frame(2.0, 0.0);
            let selections = atlas_selections(&atlas, &app.template, &frame);
            // 2 つの面に同時に入る画素は、ほとんどない（辺の上だけ）。
            let both = (0..frame.w * frame.h).filter(|&i| selections.iter().filter(|s| s.ink[i] > 0).count() > 1).count();
            assert!(both * 100 < frame.w * frame.h, "型 {sides}: {both}");
        }
    }
}
