//! レイヤーの欄と、レイヤーの操作（Photoshop と PhotoCraft に寄せた形）。
//!
//! - 行ごとに、目（見せる）、鍵（ロック）、名前。グループの中身は字下げし、グループは開け閉めできる。
//! - 柄だけに効く調整は、柄の行の下に字下げして並ぶ（スマートフィルターと同じ見せ方）。目で切り替え、押すとプロパティで数値を直せる。
//! - 欄のいちばん下に、マスク、調整、グループ、削除のボタン。
//! - グループは 1 段だけ（グループの中にグループは作らない）。中身は、並びの中で、見出しのすぐ前に続けて置く。

use pakenoba_i18n::t;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use pakenoba_core::project::{AdjustKind, Adjustment, Blend, Layer, Mask};
use pakenoba_core::surface::Decal;
use pakenoba_core::template::P2;

use super::{App, icons};
use crate::paint::{Picture, Placed, group_of, is_artwork, members_of};

/// マスクを筆で描くときの設定。
pub(in crate::app) struct Brush {
    /// 筆の太さ（直径、mm）。
    pub size: f32,
    /// 筆の硬さ（0〜1。1 なら縁がくっきり）。
    pub hardness: f32,
    /// 隠す（黒で描く）。偽なら見せる（白で描く）。
    pub hide: bool,
    /// ひと続きの線の、前の点（マスクの画素）。
    last: Option<[f32; 2]>,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { size: 8.0, hardness: 0.6, hide: true, last: None }
    }
}

/// 別のスレッドで作っているマスク。
pub(in crate::app) struct MaskJob {
    index: usize,
    /// 作り始めたときの柄の絵（その間に柄が入れ替わっていたら、使わない）。
    picture: Picture,
    rx: Receiver<Result<Mask, String>>,
}

/// マスクの作り方。
#[derive(Clone, Copy, PartialEq)]
pub(in crate::app) enum MaskFrom {
    RevealAll,
    Transparency,
    Subject,
}

/// 画素ごとに決まる調整（調整レイヤーにもできるもの）。並びは Photoshop の「新規調整レイヤー」と同じ。`None` は区切り線。
pub(in crate::app) const PIXEL_ADJUST: [Option<AdjustKind>; 16] = [
    Some(AdjustKind::BrightnessContrast { brightness: 0, contrast: 0 }),
    Some(AdjustKind::Levels { black: 0, white: 255, gamma: 100 }),
    Some(AdjustKind::Curves { points: pakenoba_core::project::CURVE_STRAIGHT, count: 2 }),
    Some(AdjustKind::Exposure { exposure: 0, offset: 0, gamma: 100 }),
    None,
    Some(AdjustKind::Vibrance { vibrance: 0, saturation: 0 }),
    Some(AdjustKind::HueSaturation { hue: 0, saturation: 0, lightness: 0 }),
    Some(AdjustKind::ColorBalance { shadows: [0; 3], midtones: [0; 3], highlights: [0; 3], preserve: true }),
    Some(AdjustKind::BlackWhite { weights: pakenoba_core::project::BLACK_WHITE_DEFAULT }),
    Some(AdjustKind::PhotoFilter { color: [236, 138, 0], density: 25, preserve: true }),
    None,
    Some(AdjustKind::Invert),
    Some(AdjustKind::Posterize { levels: 4 }),
    Some(AdjustKind::Threshold { level: 128 }),
    Some(AdjustKind::GradientMap { from: [0, 0, 0], to: [255, 255, 255], reverse: false }),
    None,
];

/// 柄だけに効く、画像全体を見て決める調整。
pub(in crate::app) const IMAGE_ADJUST: [AdjustKind; 4] = [AdjustKind::Subject { fine: false }, AdjustKind::ClearBackground { tolerance: 40 }, AdjustKind::FlattenShading, AdjustKind::ReduceColors { colors: 6 }];

pub(in crate::app) fn adjust_label(kind: AdjustKind) -> &'static str {
    match kind {
        AdjustKind::BrightnessContrast { .. } => t!("Brightness/Contrast"),
        AdjustKind::HueSaturation { .. } => t!("Hue/Saturation"),
        AdjustKind::Levels { .. } => t!("Levels"),
        AdjustKind::Invert => t!("Invert"),
        AdjustKind::ClearBackground { .. } => t!("Clear background"),
        AdjustKind::FlattenShading => t!("Even out shading"),
        AdjustKind::ReduceColors { .. } => t!("Reduce colors"),
        AdjustKind::Subject { .. } => t!("Select subject"),
        AdjustKind::Curves { .. } => t!("Curves"),
        AdjustKind::Exposure { .. } => t!("Exposure"),
        AdjustKind::Vibrance { .. } => t!("Vibrance"),
        AdjustKind::ColorBalance { .. } => t!("Color Balance"),
        AdjustKind::BlackWhite { .. } => t!("Black & White"),
        AdjustKind::PhotoFilter { .. } => t!("Photo Filter"),
        AdjustKind::Posterize { .. } => t!("Posterize"),
        AdjustKind::Threshold { .. } => t!("Threshold"),
        AdjustKind::GradientMap { .. } => t!("Gradient Map"),
    }
}

/// 調整を選ぶ一覧（画素ごとの調整を区切りつきで）。押したものを返す。
fn pixel_adjust_menu(ui: &mut egui::Ui) -> Option<AdjustKind> {
    let mut picked = None;
    for entry in PIXEL_ADJUST {
        match entry {
            Some(kind) => {
                if ui.button(adjust_label(kind)).clicked() {
                    picked = Some(kind);
                }
            }
            None => {
                ui.separator();
            }
        }
    }
    picked
}

/// グループの見出しと調整レイヤーの、置き場所（使わない）。
const NOWHERE: Decal = Decal { panel: 0, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 };

/// マスクの大きさ（柄の絵の大きさ。長い辺は 2048 px まで）。
fn mask_size(w: usize, h: usize) -> (usize, usize) {
    let scale = (2048.0 / w.max(h) as f32).min(1.0);
    (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1))
}

/// RGBA の絵を、`(mw, mh)` へ縮める（近い画素を取る）。
fn shrink(w: usize, h: usize, rgba: &[u8], mw: usize, mh: usize) -> Vec<u8> {
    if (mw, mh) == (w, h) {
        return rgba.to_vec();
    }
    let mut out = vec![0u8; mw * mh * 4];
    for y in 0..mh {
        let sy = (y * h / mh).min(h - 1);
        for x in 0..mw {
            let sx = (x * w / mw).min(w - 1);
            out[(y * mw + x) * 4..(y * mw + x) * 4 + 4].copy_from_slice(&rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4]);
        }
    }
    out
}

/// 小さな絵の、画素の数と、画面での大きさ。
const THUMB_PX: usize = 48;
const THUMB_PT: f32 = 26.0;

/// 小さな絵の中で、柄を置く四角（縦横比を保って収める）。左、上、幅、高さ（画素）。
fn fit(size: [f32; 2]) -> [f32; 4] {
    let s = THUMB_PX as f32;
    let aspect = size[0] / size[1].max(1e-3);
    let (w, h) = if aspect >= 1.0 { (s, s / aspect) } else { (s * aspect, s) };
    [(s - w) * 0.5, (s - h) * 0.5, w, h]
}

/// 柄の小さな絵（透明な所は市松模様。柄だけの調整を効かせた絵）。
fn picture_thumb(placed: &Placed) -> Vec<u8> {
    let picture = crate::effects::shown(placed, false);
    let size = placed.size;
    let [x0, y0, w, h] = fit(size);
    let mut out = vec![0u8; THUMB_PX * THUMB_PX * 4];
    for y in 0..THUMB_PX {
        for x in 0..THUMB_PX {
            let checker = if ((x / 6) + (y / 6)) % 2 == 0 { 255.0 } else { 204.0 };
            let (fx, fy) = ((x as f32 + 0.5 - x0) / w, (y as f32 + 0.5 - y0) / h);
            let c = if (0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy) { picture.sample([(fx - 0.5) * size[0], (0.5 - fy) * size[1]], size) } else { [0.0; 4] };
            let o = (y * THUMB_PX + x) * 4;
            for i in 0..3 {
                out[o + i] = (c[i] * c[3] + checker * (1.0 - c[3])).round() as u8;
            }
            out[o + 3] = 255;
        }
    }
    out
}

/// マスクの小さな絵（白が見せる所。柄の外は暗い灰色）。
fn mask_thumb_pixels(placed: &Placed) -> Vec<u8> {
    let mut out = vec![0u8; THUMB_PX * THUMB_PX * 4];
    let Some(mask) = placed.layer.mask.as_ref() else { return out };
    let size = placed.size;
    let [x0, y0, w, h] = fit(size);
    for y in 0..THUMB_PX {
        for x in 0..THUMB_PX {
            let (fx, fy) = ((x as f32 + 0.5 - x0) / w, (y as f32 + 0.5 - y0) / h);
            let v = if (0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy) { (crate::paint::mask_at(mask, [(fx - 0.5) * size[0], (0.5 - fy) * size[1]], size) * 255.0).round() as u8 } else { 60 };
            let o = (y * THUMB_PX + x) * 4;
            out[o..o + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    out
}

impl App {
    /// 毎回の描画のはじめに: 別のスレッドで調整した絵やマスクができていれば、取り込む。
    pub(super) fn layers_frame(&mut self) {
        let generation = crate::effects::generation();
        if generation != self.effects_seen {
            self.effects_seen = generation;
            (self.art_dirty, self.back_dirty) = (true, true);
        }
        if let Some(note) = crate::effects::take_notes().pop() {
            self.say(note);
        }
        match crate::models::take_finished() {
            Some(Ok(())) => {
                crate::effects::forget_all();
                (self.art_dirty, self.back_dirty) = (true, true);
                self.say(t!("The high-quality subject model is ready."));
            }
            Some(Err(why)) => self.say(why),
            None => {}
        }
        if let Some(job) = &self.mask_job {
            match job.rx.try_recv() {
                Ok(result) => {
                    let job = self.mask_job.take().unwrap();
                    let same = self.decals.get(job.index).is_some_and(|d| super::assets::same_picture(&d.picture, &job.picture));
                    match result {
                        Ok(mask) if same => {
                            self.checkpoint();
                            self.decals[job.index].layer.mask = Some(mask);
                            (self.art_dirty, self.back_dirty) = (true, true);
                            self.say(t!("Made a mask from the subject. Paint on it with the brush to fix the edges."));
                        }
                        Ok(_) => {}
                        Err(why) => self.say(why),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.mask_job = None,
            }
        }
        if self.brush.is_some() && self.brush_target().is_none() {
            self.brush = None;
        }
    }

    /// グループの入れ子とクリップの決まりに合わない印を直す。
    pub(super) fn tidy_groups(&mut self) {
        let groups: Vec<bool> = self.decals.iter().map(|d| matches!(d.picture, Picture::Group { .. })).collect();
        let backs: Vec<bool> = self.decals.iter().map(|d| d.back).collect();
        let mut depth: Vec<u8> = self.decals.iter().map(|d| d.layer.depth).collect();
        let mut clipped: Vec<bool> = self.decals.iter().map(|d| d.layer.clipped).collect();
        pakenoba_core::project::nest_rule(&groups, &backs, &mut depth, &mut clipped);
        for ((d, depth), clipped) in self.decals.iter_mut().zip(depth).zip(clipped) {
            (d.layer.depth, d.layer.clipped) = (depth, clipped);
        }
    }

    /// 右の欄の、レイヤー。
    pub(super) fn layers(&mut self, ui: &mut egui::Ui) {
        ui.strong(t!("Layers"));
        self.blend_controls(ui);
        self.lock_controls(ui);
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            if ui.checkbox(&mut self.show_guides, t!("Guides (cut lines, fold lines, glue flaps)")).changed() {
                self.art_dirty = true;
                self.back_dirty = true;
            }
            // 紙の表と裏に分けて、上に重なっているものから並べる。
            let mut rows_at: Vec<(usize, egui::Rect)> = Vec::new();
            for back in [false, true] {
                ui.label(if back { t!("Back (inside of the box)") } else { t!("Front (outside of the box)") });
                let rows: Vec<usize> = (0..self.decals.len()).rev().filter(|&i| self.decals[i].back == back).collect();
                if rows.is_empty() && self.bases[back as usize].is_none() {
                    ui.small(t!("　No artwork"));
                }
                for index in rows {
                    // 閉じたグループの中身は出さない。
                    if crate::paint::ancestors(&self.decals, index).iter().any(|&g| matches!(self.decals[g].picture, Picture::Group { open: false })) {
                        continue;
                    }
                    if let Some(rect) = self.layer_row(ui, index) {
                        rows_at.push((index, rect));
                    }
                    self.adjust_rows(ui, index);
                }
                self.base_layer(ui, back as usize);
            }
            self.drag_layer(ui, &rows_at);
        });
        ui.add_space(2.0);
        self.layer_buttons(ui);
        ui.add_space(6.0);
        self.psd_loader(ui);
    }

    /// レイヤーの行 1 つ。行の場所を返す（名前を変えている間は返さない）。
    /// 並びは Photoshop と同じ: 目、（字下げ）、（グループなら開け閉め）、小さな絵、マスクの小さな絵、名前、fx、ロックの印。
    fn layer_row(&mut self, ui: &mut egui::Ui, index: usize) -> Option<egui::Rect> {
        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let layer = self.decals[index].layer.clone();
            if icons::eye_button(ui, !layer.hidden).clicked() {
                self.checkpoint();
                self.decals[index].layer.hidden = !layer.hidden;
                (self.art_dirty, self.back_dirty) = (true, true);
            }
            if layer.depth > 0 {
                ui.add_space(16.0 * layer.depth as f32);
            }
            if layer.clipped {
                // クリップしているものは、少し字下げして、下向きの曲がった矢印（Photoshop と同じ）。
                ui.add_space(4.0);
                let base = crate::paint::clip_base(&self.decals, index).map(|b| self.layer_label(b)).unwrap_or_default();
                ui.weak("↳").on_hover_text(t!("Clipped to \"{base}\" (shows only where it is visible)"));
            }
            if let Picture::Group { open } = self.decals[index].picture {
                let arrow = if open { "▾" } else { "▸" };
                if ui.add(egui::Button::new(arrow).small().frame(false)).on_hover_text(if open { t!("Close the group") } else { t!("Open the group") }).clicked() {
                    // 開け閉めは、元に戻す記録に入れない（見た目だけ）。保存はする。
                    self.decals[index].picture = Picture::Group { open: !open };
                }
            }
            let thumb = self.layer_thumb(ui, index);
            let mut mask_click = None;
            if layer.mask.is_some() {
                let r = self.mask_thumb(ui, index);
                if r.clicked() {
                    mask_click = Some(ui.input(|i| i.modifiers.shift));
                }
                r.context_menu(|ui| self.mask_menu(ui, index));
            }
            if let Some(shift) = mask_click {
                if shift {
                    // Shift+クリックで、マスクを使う・使わない（Photoshop と同じ）。
                    if !layer.sealed() {
                        self.checkpoint();
                        if let Some(m) = &mut self.decals[index].layer.mask {
                            m.enabled = !m.enabled;
                        }
                        (self.art_dirty, self.back_dirty) = (true, true);
                    }
                } else {
                    self.select_only(Some(index));
                    self.toggle_brush();
                }
            }
            ui.add_space(2.0);
            if let Some((renamed, text)) = &mut self.renaming
                && *renamed == index
            {
                let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if cancel || edit.lost_focus() {
                    let text = text.trim().to_owned();
                    self.renaming = None;
                    if !cancel {
                        let name = (!text.is_empty()).then_some(text);
                        if name != self.decals[index].layer.name {
                            self.checkpoint();
                            self.decals[index].layer.name = name;
                        }
                    }
                }
                return None;
            }
            // 右の端に、ロックの印と、調整の印（fx）。
            let right = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if layer.locked {
                    icons::lock_mark(ui, true, t!("All locked"));
                } else if layer.lock_pixels || layer.lock_position {
                    let tip = match (layer.lock_pixels, layer.lock_position) {
                        (true, true) => t!("Image and position locked"),
                        (true, false) => t!("Image locked"),
                        _ => t!("Position locked"),
                    };
                    icons::lock_mark(ui, false, tip);
                }
                if !layer.adjust.is_empty() && is_artwork(&self.decals[index].picture) {
                    ui.weak("fx").on_hover_text(t!("Has adjustments for this artwork only"));
                }
                let mut label = self.layer_label(index);
                if self.is_editing(&self.decals[index].picture) {
                    label.push_str(t!(" (being edited elsewhere)"));
                }
                let text = if layer.hidden { egui::RichText::new(label).weak() } else { egui::RichText::new(label) };
                let text = if matches!(self.decals[index].picture, Picture::Group { .. }) { text.strong() } else { text };
                let selected = self.is_selected(index);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| ui.selectable_label(selected, text)).inner.interact(egui::Sense::click_and_drag())
            });
            // 小さな絵を押しても、行を押したのと同じ。
            Some(if thumb.clicked() { thumb.union(right.inner) } else { right.inner })
        });
        let response = row.inner?;
        if response.hovered() {
            self.hover_now = Some(index);
        }
        if response.double_clicked() {
            self.renaming = Some((index, self.decals[index].layer.name.clone().unwrap_or_default()));
        } else if response.clicked() {
            let shift = ui.input(|i| i.modifiers.shift);
            self.select_click(Some(index), shift);
            self.adjust_focus = None;
            // グループを押したら、中身もいっしょに選ぶ（まとめて動かせるように）。
            if !shift && matches!(self.decals[index].picture, Picture::Group { .. }) {
                self.extra = members_of(&self.decals, index).into_iter().filter(|&i| is_artwork(&self.decals[i].picture)).collect();
            }
        }
        if response.secondary_clicked() && !self.is_selected(index) {
            self.select_only(Some(index));
        }
        if response.drag_started() {
            self.layer_drag = Some(index);
        }
        let response = response.on_hover_text(t!("Double-click to rename. Drag to change the stacking order"));
        response.context_menu(|ui| {
            if ui.button(t!("Rename")).clicked() {
                self.renaming = Some((index, self.decals[index].layer.name.clone().unwrap_or_default()));
                ui.close();
            }
            if !matches!(self.decals[index].picture, Picture::Group { .. }) {
                let label = if self.decals[index].layer.clipped { t!("Release clipping mask") } else { t!("Create clipping mask") };
                if ui.button(label).on_hover_text(t!("Show this layer only where the layer below it is visible (Ctrl+Alt+G)")).clicked() {
                    self.toggle_clip(index);
                    ui.close();
                }
            }
            match self.decals[index].picture {
                Picture::Group { .. } => {
                    if ui.button(t!("Ungroup")).clicked() {
                        self.ungroup(index);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(t!("Delete")).clicked() {
                        self.delete_selected();
                        ui.close();
                    }
                }
                Picture::Adjustment => {
                    ui.separator();
                    if ui.button(t!("Delete")).clicked() {
                        self.delete_selected();
                        ui.close();
                    }
                }
                _ => {
                    if self.decals[index].layer.depth > 0 && ui.button(t!("Take out of the group")).clicked() {
                        self.take_out_of_group(index);
                        ui.close();
                    }
                    ui.separator();
                    self.decal_menu(ui);
                }
            }
        });
        Some(row.response.rect)
    }

    /// 柄の行の下に、柄だけに効く調整を、上から（あとで効くものから）並べる。
    fn adjust_rows(&mut self, ui: &mut egui::Ui, index: usize) {
        if !is_artwork(&self.decals[index].picture) || self.decals[index].layer.adjust.is_empty() {
            return;
        }
        let indent = 20.0 + 16.0 * self.decals[index].layer.depth as f32;
        for k in (0..self.decals[index].layer.adjust.len()).rev() {
            let a = self.decals[index].layer.adjust[k].clone();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(indent);
                if icons::eye_button(ui, a.on).clicked() {
                    self.checkpoint();
                    self.decals[index].layer.adjust[k].on = !a.on;
                    (self.art_dirty, self.back_dirty) = (true, true);
                }
                let focused = self.adjust_focus == Some((index, k));
                let text = egui::RichText::new(adjust_label(a.kind)).small();
                let text = if a.on { text } else { text.weak() };
                if ui.selectable_label(focused, text).on_hover_text(t!("Click to change its settings in Properties")).clicked() {
                    self.select_only(Some(index));
                    self.adjust_focus = Some((index, k));
                }
            });
        }
    }

    /// 欄のいちばん下の、アイコンのボタン（Photoshop と同じ並び: マスク、調整、グループ、削除）。
    fn layer_buttons(&mut self, ui: &mut egui::Ui) {
        let index = self.selected.filter(|&i| i < self.decals.len());
        let artwork = index.filter(|&i| is_artwork(&self.decals[i].picture) && !self.decals[i].layer.sealed());
        let size = egui::vec2(30.0, 24.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let deletable = index.is_some_and(|i| !self.decals[i].layer.locked);
                if icons::small_icon_button(ui, size, false, deletable, t!("Delete the selected layers"), icons::draw_trash).clicked() {
                    self.delete_selected();
                }
                if icons::small_icon_button(ui, size, false, true, t!("Put the selected layers in a group (Ctrl+G)"), icons::draw_folder).clicked() {
                    self.group_selected();
                }
                let adjust = icons::small_icon_button(ui, size, false, true, t!("Add an adjustment. You can change it, switch it off or remove it later"), icons::draw_adjust);
                egui::Popup::menu(&adjust).show(|ui| {
                    ui.set_min_width(240.0);
                    ui.weak(t!("Adjustment layer (everything below)"));
                    if let Some(kind) = pixel_adjust_menu(ui) {
                        self.add_adjustment_layer(kind);
                        ui.close();
                    }
                    ui.add_enabled_ui(artwork.is_some(), |ui| {
                        ui.menu_button(t!("This artwork only"), |ui| {
                            ui.set_min_width(220.0);
                            if let Some(kind) = pixel_adjust_menu(ui) {
                                self.add_adjustment(kind);
                                ui.close();
                            }
                            for kind in IMAGE_ADJUST {
                                if ui.button(adjust_label(kind)).clicked() {
                                    self.add_adjustment(kind);
                                    ui.close();
                                }
                            }
                        });
                    });
                });
                let can_mask = artwork.is_some_and(|i| self.decals[i].layer.mask.is_none());
                let mask = icons::small_icon_button(ui, size, false, can_mask, t!("Add a layer mask to the selected artwork"), icons::draw_mask);
                egui::Popup::menu(&mask).show(|ui| {
                    ui.set_min_width(220.0);
                    if ui.button(t!("Reveal all")).on_hover_text(t!("A white mask that shows everything. Paint black with the brush to hide parts")).clicked() {
                        self.add_mask(MaskFrom::RevealAll);
                        ui.close();
                    }
                    if ui.button(t!("From the subject")).on_hover_text(t!("Finds the main subject with AI on this PC and hides the rest")).clicked() {
                        self.add_mask(MaskFrom::Subject);
                        ui.close();
                    }
                    if ui.button(t!("From transparency")).on_hover_text(t!("Uses the transparent parts of the picture")).clicked() {
                        self.add_mask(MaskFrom::Transparency);
                        ui.close();
                    }
                });
            });
        });
    }

    /// ロックの段（Photoshop の 2 段目と同じ）。画像、位置、すべて。選んでいるレイヤーすべてに効く。
    fn lock_controls(&mut self, ui: &mut egui::Ui) {
        let mut targets: Vec<usize> = self.selection();
        if let Some(header) = self.selected.filter(|&i| matches!(self.decals.get(i).map(|d| &d.picture), Some(Picture::Group { .. }))) {
            targets.retain(|&i| i == header);
        }
        let first = targets.first().map(|&i| self.decals[i].layer.clone());
        let enabled = first.is_some();
        let layer = first.unwrap_or_default();
        let mut change: Option<u8> = None;
        ui.horizontal(|ui| {
            ui.label(t!("Lock:"));
            ui.spacing_mut().item_spacing.x = 1.0;
            let size = egui::vec2(24.0, 20.0);
            let all = layer.locked;
            if icons::small_icon_button(ui, size, layer.lock_pixels || all, enabled && !all, t!("Lock image: adjustments and the mask cannot be changed"), icons::draw_brush).clicked() {
                change = Some(0);
            }
            if icons::small_icon_button(ui, size, layer.lock_position || all, enabled && !all, t!("Lock position: cannot be moved, resized or rotated"), icons::draw_move).clicked() {
                change = Some(1);
            }
            if icons::small_icon_button(ui, size, all, enabled, t!("Lock all: cannot be grabbed, changed or deleted"), icons::draw_lock).clicked() {
                change = Some(2);
            }
        });
        let Some(which) = change else { return };
        self.checkpoint();
        for i in targets {
            let l = &mut self.decals[i].layer;
            match which {
                0 => l.lock_pixels = !layer.lock_pixels,
                1 => l.lock_position = !layer.lock_position,
                _ => l.locked = !layer.locked,
            }
        }
        self.grab = None;
        if self.brush_target().is_none() {
            self.brush = None;
        }
    }

    /// 行の小さな絵（柄の絵。グループと調整レイヤーは印）。
    fn layer_thumb(&mut self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let side = egui::vec2(THUMB_PT, THUMB_PT);
        match self.decals[index].picture {
            Picture::Group { .. } => {
                let r = ui.allocate_exact_size(side, egui::Sense::click()).1;
                icons::draw_folder(ui.painter(), r.rect.center(), egui::Stroke::new(1.3, ui.visuals().text_color()));
                r
            }
            Picture::Adjustment => {
                let r = ui.allocate_exact_size(side, egui::Sense::click()).1;
                ui.painter().rect_stroke(r.rect.shrink(1.0), 2.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
                icons::draw_adjust(ui.painter(), r.rect.center(), egui::Stroke::new(1.2, ui.visuals().text_color()));
                r
            }
            _ => {
                let pixels = picture_thumb(&self.decals[index]);
                let id = self.thumb_texture(ui.ctx(), (index, 0), pixels);
                let r = ui.add(egui::Image::new((id, side)).sense(egui::Sense::click()));
                ui.painter().rect_stroke(r.rect, 1.0, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Outside);
                r
            }
        }
    }

    /// 行のマスクの小さな絵。筆で描いている間は、白い枠で囲む（Photoshop と同じく、描く相手がマスクだと分かるように）。
    fn mask_thumb(&mut self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let side = egui::vec2(THUMB_PT, THUMB_PT);
        let pixels = mask_thumb_pixels(&self.decals[index]);
        let id = self.thumb_texture(ui.ctx(), (index, 1), pixels);
        let r = ui.add(egui::Image::new((id, side)).sense(egui::Sense::click()));
        let painting = self.brush.is_some() && self.selected == Some(index);
        let frame = if painting { egui::Stroke::new(2.0, egui::Color32::WHITE) } else { egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color) };
        ui.painter().rect_stroke(r.rect, 1.0, frame, egui::StrokeKind::Outside);
        if self.decals[index].layer.mask.as_ref().is_some_and(|m| !m.enabled) {
            // 使っていないマスクには、赤いばつ（Photoshop と同じ）。
            let red = egui::Stroke::new(1.6, egui::Color32::from_rgb(220, 40, 40));
            ui.painter().line_segment([r.rect.left_top(), r.rect.right_bottom()], red);
            ui.painter().line_segment([r.rect.right_top(), r.rect.left_bottom()], red);
        }
        r.on_hover_text(t!("Layer mask. Click to paint on it with the brush, Shift+click to switch it off or on. Right-click for more"))
    }

    /// 小さな絵のテクスチャ。中身が変わったときだけ作り直す。
    fn thumb_texture(&mut self, ctx: &egui::Context, key: (usize, u8), pixels: Vec<u8>) -> egui::TextureId {
        let image = || egui::ColorImage::from_rgba_unmultiplied([THUMB_PX, THUMB_PX], &pixels);
        match self.thumbs.get_mut(&key) {
            Some((old, tex)) => {
                if *old != pixels {
                    tex.set(image(), egui::TextureOptions::LINEAR);
                    *old = pixels.clone();
                }
                tex.id()
            }
            None => {
                let tex = ctx.load_texture(format!("layer-thumb-{}-{}", key.0, key.1), image(), egui::TextureOptions::LINEAR);
                let id = tex.id();
                self.thumbs.insert(key, (pixels, tex));
                id
            }
        }
    }

    /// レイヤーの行のドラッグ。運んでいる間は、落とす場所に線を出し、放したら、同じ面の中で重なり順を変える。
    /// 線の字下げが、入る深さ（グループの見出しのすぐ下に落とすと、そのグループのいちばん上の中身になる）。
    fn drag_layer(&mut self, ui: &mut egui::Ui, rows: &[(usize, egui::Rect)]) {
        let Some(dragged) = self.layer_drag.filter(|&i| i < self.decals.len()) else {
            self.layer_drag = None;
            return;
        };
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else { return };
        let back = self.decals[dragged].back;
        let target = rows
            .iter()
            .filter(|(i, _)| self.decals[*i].back == back)
            .min_by(|a, b| (a.1.center().y - pointer.y).abs().total_cmp(&(b.1.center().y - pointer.y).abs()))
            .map(|&(i, rect)| (i, rect, pointer.y < rect.center().y));
        let Some((over, rect, above)) = target else { return };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let y = if above { rect.top() } else { rect.bottom() };
        let (_, depth) = self.drop_place(over, above);
        let x = rect.left() + 20.0 + 16.0 * depth as f32;
        ui.painter().hline(x..=rect.right(), y, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
        if ui.input(|i| i.pointer.any_released()) {
            self.layer_drag = None;
            self.move_layer(dragged, over, above);
        }
    }

    /// レイヤーの塊（グループなら、中身すべてと見出し）。並びの中の番号（下から）。
    fn block(&self, index: usize) -> Vec<usize> {
        match self.decals[index].picture {
            Picture::Group { .. } => {
                let mut v = members_of(&self.decals, index);
                v.push(index);
                v
            }
            _ => vec![index],
        }
    }

    /// 行 `over` のすぐ上（`above`）か、すぐ下へ落としたときの、入る場所（並びの番号。いまの並びで、その前に入る）と深さ。
    fn drop_place(&self, over: usize, above: bool) -> (usize, u8) {
        let d = self.decals[over].layer.depth;
        if matches!(self.decals[over].picture, Picture::Group { .. }) {
            if above {
                (over + 1, d)
            } else {
                // 見出しのすぐ下: そのグループのいちばん上の中身。
                (over, d + 1)
            }
        } else if above {
            (over + 1, d)
        } else {
            (over, d)
        }
    }

    /// レイヤー `dragged`（グループなら中身ごと）を、並びの番号 `at` の前へ、深さ `depth` で入れる。
    fn restack(&mut self, dragged: usize, at: usize, depth: u8) {
        let block = self.block(dragged);
        if (block[0]..=dragged).contains(&at) && block[0] != at && at != dragged + 1 {
            return;
        }
        let head = self.decals[dragged].layer.depth;
        let deepest = block.iter().map(|&i| self.decals[i].layer.depth).max().unwrap_or(head);
        if depth as i32 + (deepest as i32 - head as i32) >= pakenoba_core::project::MAX_DEPTH as i32 {
            self.say(t!("Groups can be nested up to {} levels.", pakenoba_core::project::MAX_DEPTH));
            return;
        }
        self.checkpoint();
        let mut moving: Vec<Placed> = block.iter().map(|&i| self.decals[i].clone()).collect();
        for item in &mut moving {
            item.layer.depth = (item.layer.depth as i32 - head as i32 + depth as i32) as u8;
        }
        let at = at - block.iter().filter(|&&i| i < at).count();
        for &i in block.iter().rev() {
            self.decals.remove(i);
        }
        let count = moving.len();
        // クリップしているものの間に入れたら、いっしょにクリップする。外に出したら、クリップを外す。
        let back = moving[0].back;
        let between = (at..self.decals.len()).find(|&i| self.decals[i].back == back).is_some_and(|i| self.decals[i].layer.clipped && self.decals[i].layer.depth == depth);
        if let Some(top) = moving.last_mut()
            && !matches!(top.picture, Picture::Group { .. })
        {
            top.layer.clipped = between;
        }
        for (k, item) in moving.into_iter().enumerate() {
            self.decals.insert(at + k, item);
        }
        self.tidy_groups();
        self.select_only(Some(at + count - 1));
        self.grab = None;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// レイヤー `dragged` を、行 `over` のすぐ上（`above`）か、すぐ下へ動かす。
    pub(super) fn move_layer(&mut self, dragged: usize, over: usize, above: bool) {
        if dragged >= self.decals.len() || over >= self.decals.len() || dragged == over || self.block(dragged).contains(&over) {
            return;
        }
        let (at, depth) = self.drop_place(over, above);
        self.restack(dragged, at, depth);
    }

    /// 選んでいるレイヤーを、同じ面の中で、重なりの上（+1）か下（-1）へ 1 つ動かす。グループの端では、グループを出入りする（Photoshop と同じ）。
    pub(super) fn reorder_selected(&mut self, step: isize) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else { return };
        let back = self.decals[index].back;
        let d = self.decals[index].layer.depth;
        let is_header = |s: &Self, i: usize| matches!(s.decals[i].picture, Picture::Group { .. });
        if step > 0 {
            let Some(x) = (index + 1..self.decals.len()).find(|&i| self.decals[i].back == back) else { return };
            let xd = self.decals[x].layer.depth;
            if xd < d {
                // グループのいちばん上から出る: 見出しの上へ。
                self.restack(index, x + 1, xd);
            } else if xd > d {
                // 上のグループに、いちばん下の中身として入る。
                self.restack(index, index + 1, d + 1);
            } else {
                // となりの柄か、となりのグループの見出しの上へ。
                self.restack(index, x + 1, d);
            }
        } else {
            let start = self.block(index)[0];
            if d > 0 && group_of(&self.decals, index).is_some_and(|g| members_of(&self.decals, g)[0] == start) {
                // グループのいちばん下から出る: その場で、グループの下になる。
                self.restack(index, start, d - 1);
                return;
            }
            let Some(y) = (0..start).rev().find(|&i| self.decals[i].back == back) else { return };
            let yd = self.decals[y].layer.depth;
            if yd < d {
                // グループのいちばん下から出る: その場で、グループの下になる。
                self.restack(index, start, d - 1);
            } else if is_header(self, y) && yd == d {
                // 下のグループに、いちばん上の中身として入る。
                self.restack(index, y, d + 1);
            } else {
                let below = self.block(y)[0];
                let below = if yd > d { members_of(&self.decals, group_of(&self.decals, y).unwrap_or(y)).first().copied().unwrap_or(below) } else { below };
                self.restack(index, below, d);
            }
        }
    }

    /// クリッピングマスクを作る・外す（すぐ下のレイヤーでクリップする）。
    pub(super) fn toggle_clip(&mut self, index: usize) {
        if index >= self.decals.len() || matches!(self.decals[index].picture, Picture::Group { .. }) {
            return;
        }
        self.checkpoint();
        let layer = &mut self.decals[index].layer;
        layer.clipped = !layer.clipped;
        let wanted = layer.clipped;
        self.tidy_groups();
        if wanted && !self.decals[index].layer.clipped {
            self.say(t!("There is no layer just below it in the same group to clip to."));
        }
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// グループを作り、選んでいるレイヤーを入れる（グループを選んでいれば、グループごと入れ子にする）。
    /// 何も選んでいなければ、空のグループを、表のいちばん上に作る。
    pub(super) fn group_selected(&mut self) {
        let back = self.selected.and_then(|i| self.decals.get(i)).is_some_and(|d| d.back);
        let chosen: Vec<usize> = self.selection().into_iter().filter(|&i| self.decals[i].back == back).collect();
        // 選んだものの塊。選んだグループの中のものは、グループごと動くので除く。
        let mut heads: Vec<usize> = chosen.iter().copied().filter(|&i| !crate::paint::ancestors(&self.decals, i).iter().any(|g| chosen.contains(g))).collect();
        heads.sort_unstable();
        let number = self.decals.iter().filter(|d| matches!(d.picture, Picture::Group { .. })).count() + 1;
        let header = |depth: u8| Placed { decal: NOWHERE, picture: Picture::Group { open: true }, back, layer: Layer { name: Some(t!("Group {number}")), blend: Blend::PassThrough, depth, ..Layer::default() } };
        if heads.is_empty() {
            self.checkpoint();
            self.decals.push(header(0));
            self.select_only(Some(self.decals.len() - 1));
            (self.art_dirty, self.back_dirty) = (true, true);
            return;
        }
        let top = *heads.last().unwrap();
        let depth = self.decals[top].layer.depth;
        let blocks: Vec<Vec<usize>> = heads.iter().map(|&h| self.block(h)).collect();
        let deepest = blocks.iter().flatten().map(|&i| self.decals[i].layer.depth as i32 - self.decals[*blocks.iter().find(|b| b.contains(&i)).unwrap().last().unwrap()].layer.depth as i32).max().unwrap_or(0);
        if depth as i32 + 1 + deepest >= pakenoba_core::project::MAX_DEPTH as i32 {
            self.say(t!("Groups can be nested up to {} levels.", pakenoba_core::project::MAX_DEPTH));
            return;
        }
        self.checkpoint();
        let mut all: Vec<usize> = blocks.iter().flatten().copied().collect();
        all.sort_unstable();
        let mut moving: Vec<Placed> = Vec::new();
        for block in &blocks {
            let head = self.decals[*block.last().unwrap()].layer.depth;
            for &i in block {
                let mut item = self.decals[i].clone();
                item.layer.depth = (item.layer.depth as i32 - head as i32 + depth as i32 + 1) as u8;
                moving.push(item);
            }
        }
        let at = top + 1 - all.iter().filter(|&&i| i <= top).count();
        for &i in all.iter().rev() {
            self.decals.remove(i);
        }
        let count = moving.len();
        if let Some(first) = moving.first_mut() {
            first.layer.clipped = false;
        }
        for (k, item) in moving.into_iter().enumerate() {
            self.decals.insert(at + k, item);
        }
        self.decals.insert(at + count, header(depth));
        self.tidy_groups();
        self.select_only(Some(at + count));
        self.extra = (at..at + count).collect();
        (self.art_dirty, self.back_dirty) = (true, true);
        self.say(t!("Grouped {count} layer(s). Press Ctrl+Z to undo.", count = heads.len()));
    }

    /// グループを解く（中身は、そのまま 1 段浅くして残す）。
    pub(super) fn ungroup(&mut self, header: usize) {
        if !matches!(self.decals.get(header).map(|d| &d.picture), Some(Picture::Group { .. })) {
            return;
        }
        self.checkpoint();
        for i in members_of(&self.decals, header) {
            self.decals[i].layer.depth = self.decals[i].layer.depth.saturating_sub(1);
        }
        self.decals.remove(header);
        self.select_only(None);
        self.tidy_groups();
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// レイヤーを、入っているグループの外（グループのすぐ上）へ出す。
    fn take_out_of_group(&mut self, index: usize) {
        let Some(g) = group_of(&self.decals, index) else { return };
        let depth = self.decals[g].layer.depth;
        self.restack(index, g + 1, depth);
    }

    /// 選んでいる柄に、柄だけに効く調整を足す。
    pub(super) fn add_adjustment(&mut self, kind: AdjustKind) {
        let targets: Vec<usize> = self.selection().into_iter().filter(|&i| is_artwork(&self.decals[i].picture) && !self.decals[i].layer.sealed()).collect();
        if targets.is_empty() {
            return;
        }
        if matches!(kind, AdjustKind::Subject { fine: true }) && !crate::models::fine_model_ready() {
            crate::models::start();
        }
        self.checkpoint();
        for &i in &targets {
            self.decals[i].layer.adjust.push(Adjustment::new(kind));
        }
        let first = targets[0];
        self.adjust_focus = Some((first, self.decals[first].layer.adjust.len() - 1));
        (self.art_dirty, self.back_dirty) = (true, true);
        if matches!(kind, AdjustKind::Subject { .. }) {
            self.say(t!("Finding the subject on this PC. The picture is not sent anywhere."));
        }
    }

    /// 調整レイヤーを、選んでいるレイヤーのすぐ上に足す（何も選んでいなければ、表のいちばん上）。
    pub(super) fn add_adjustment_layer(&mut self, kind: AdjustKind) {
        let index = self.selected.filter(|&i| i < self.decals.len());
        let back = index.is_some_and(|i| self.decals[i].back);
        // 選んでいるのがグループの中身なら、同じグループの中に。グループの見出しなら、グループの上に。
        let depth = index.map_or(0, |i| self.decals[i].layer.depth);
        let at = index.map_or(self.decals.len(), |i| i + 1);
        self.checkpoint();
        let layer = Layer { adjust: vec![Adjustment::new(kind)], depth, ..Layer::default() };
        self.decals.insert(at, Placed { decal: NOWHERE, picture: Picture::Adjustment, back, layer });
        self.tidy_groups();
        self.select_only(Some(at));
        self.adjust_focus = Some((at, 0));
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// プロパティの欄の、調整の一覧と数値。`index` は柄か、調整レイヤー。
    pub(super) fn adjust_properties(&mut self, ui: &mut egui::Ui, index: usize, locked: bool) {
        let locked = locked || self.decals[index].layer.sealed();
        let layer_only = matches!(self.decals[index].picture, Picture::Adjustment);
        let mut list = self.decals[index].layer.adjust.clone();
        if list.is_empty() && !layer_only {
            return;
        }
        let before = list.clone();
        let mut remove = None;
        let mut swap = None;
        let focus = self.adjust_focus.filter(|(i, _)| *i == index).map(|(_, k)| k);
        let header = if layer_only { t!("Adjustments") } else { t!("Adjustments (this artwork only)") };
        egui::CollapsingHeader::new(header).id_salt(("adjust-list", index)).default_open(true).show(ui, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                let n = list.len();
                for k in (0..n).rev() {
                    let a = &mut list[k];
                    let open = focus.is_none_or(|f| f == k);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut a.on, "").on_hover_text(t!("Switch this adjustment on or off"));
                        let label = egui::RichText::new(adjust_label(a.kind)).strong();
                        ui.label(if a.on { label } else { label.weak() });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text(t!("Remove this adjustment")).clicked() {
                                remove = Some(k);
                            }
                            if ui.add_enabled(k > 0, egui::Button::new("▼").small()).on_hover_text(t!("Apply earlier")).clicked() {
                                swap = Some((k, k - 1));
                            }
                            if ui.add_enabled(k + 1 < n, egui::Button::new("▲").small()).on_hover_text(t!("Apply later")).clicked() {
                                swap = Some((k, k + 1));
                            }
                        });
                    });
                    if open {
                        adjust_numbers(ui, &mut a.kind);
                    }
                    ui.add_space(2.0);
                }
                if n > 1 {
                    ui.weak(t!("Applied from the bottom up, like in Photoshop."));
                }
            });
        });
        if let Some(k) = remove {
            list.remove(k);
            self.adjust_focus = None;
        }
        if let Some((a, b)) = swap {
            list.swap(a, b);
            self.adjust_focus = Some((index, b));
        }
        if list != before {
            let earlier = self.snapshot();
            let structural = list.len() != before.len() || list.iter().zip(&before).any(|(a, b)| a.on != b.on || std::mem::discriminant(&a.kind) != std::mem::discriminant(&b.kind));
            for (a, b) in list.iter().zip(&before) {
                if let (AdjustKind::Subject { fine: true }, AdjustKind::Subject { fine: false }) = (a.kind, b.kind)
                    && !crate::models::fine_model_ready()
                {
                    crate::models::start();
                }
            }
            self.decals[index].layer.adjust = list;
            (self.art_dirty, self.back_dirty) = (true, true);
            // スライダーのひと続きは、1 回ぶんにまとめる。
            if !self.editing || structural {
                self.history.record(earlier);
            }
            self.editing = !structural;
        }
        if let Some(fetch) = Some(crate::models::state()).filter(|s| matches!(s, crate::models::Fetch::Running(..))) {
            let crate::models::Fetch::Running(done, total) = fetch else { unreachable!() };
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(t!("Downloading the high-quality model… {} / {} MB", done >> 20, total >> 20)));
        }
        if crate::effects::busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(t!("Applying adjustments…"));
            });
        }
    }

    /// プロパティの欄: グループの見出しや、調整レイヤーを選んでいるとき。
    pub(super) fn layer_properties(&mut self, ui: &mut egui::Ui, index: usize) {
        let locked = self.decals[index].layer.locked;
        egui::Grid::new("layer-properties").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(t!("Type"));
            ui.label(self.decals[index].picture.label());
            ui.end_row();
            ui.label(t!("Side of sheet"));
            ui.label(if self.decals[index].back { t!("Back") } else { t!("Front") });
            ui.end_row();
        });
        match self.decals[index].picture {
            Picture::Group { .. } => {
                let count = members_of(&self.decals, index).len();
                ui.small(t!("{count} layer(s) in this group. Drag layers onto the group to add them."));
                ui.small(t!("\"Pass Through\" mixes the contents with what is below as if there were no group. Other blend modes mix the group as one picture."));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!locked, egui::Button::new(t!("Ungroup"))).clicked() {
                        self.ungroup(index);
                    }
                    if ui.add_enabled(!locked, egui::Button::new(t!("Delete group and contents"))).clicked() {
                        self.delete_selected();
                    }
                });
            }
            Picture::Adjustment => {
                ui.small(t!("Applies to all layers below it. Inside a group that is not \"Pass Through\", only to the layers below it in the group."));
                self.adjust_properties(ui, index, locked);
                ui.add_enabled_ui(!locked, |ui| {
                    ui.menu_button(t!("Add to this adjustment layer"), |ui| {
                        if let Some(kind) = pixel_adjust_menu(ui) {
                            self.checkpoint();
                            self.decals[index].layer.adjust.push(Adjustment::new(kind));
                            (self.art_dirty, self.back_dirty) = (true, true);
                            ui.close();
                        }
                    });
                });
                if ui.add_enabled(!locked, egui::Button::new(t!("Delete"))).clicked() {
                    self.delete_selected();
                }
            }
            _ => {}
        }
    }

    /// 選んでいる柄に、マスクを足す。
    pub(super) fn add_mask(&mut self, how: MaskFrom) {
        let Some(index) = self.selected.filter(|&i| self.decals.get(i).is_some_and(|d| is_artwork(&d.picture))) else { return };
        let decal = &self.decals[index];
        let Some((w, h, rgba)) = crate::effects::raster(&decal.picture, decal.size) else { return };
        let (mw, mh) = mask_size(w, h);
        match how {
            MaskFrom::RevealAll => {
                self.checkpoint();
                self.decals[index].layer.mask = Some(Mask::reveal_all(mw as u32, mh as u32));
                self.say(t!("Added a mask. Paint black with the brush to hide parts, white to show them."));
            }
            MaskFrom::Transparency => {
                let small = shrink(w, h, &rgba, mw, mh);
                let data: Vec<u8> = small.chunks_exact(4).map(|p| p[3]).collect();
                self.checkpoint();
                self.decals[index].layer.mask = Some(Mask { w: mw as u32, h: mh as u32, data: Arc::new(data), invert: false, enabled: true });
                self.say(t!("Made a mask from the transparent parts."));
            }
            MaskFrom::Subject => {
                let (tx, rx) = std::sync::mpsc::channel();
                let picture = decal.picture.clone();
                std::thread::spawn(move || {
                    let small = shrink(w, h, &rgba, mw, mh);
                    let result = crate::effects::subject_mask(mw, mh, &small, crate::models::fine_model_ready()).map(|data| Mask { w: mw as u32, h: mh as u32, data: Arc::new(data), invert: false, enabled: true });
                    let _ = tx.send(result);
                    crate::effects::wake();
                });
                self.mask_job = Some(MaskJob { index, picture, rx });
                self.say(t!("Finding the subject on this PC. The picture is not sent anywhere."));
            }
        }
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// マスクの右クリックの項目。
    fn mask_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        let Some(mask) = self.decals[index].layer.mask.clone() else { return };
        let locked = self.decals[index].layer.sealed();
        ui.set_min_width(220.0);
        ui.add_enabled_ui(!locked, |ui| {
            if ui.button(t!("Paint on the mask")).clicked() {
                self.select_only(Some(index));
                if self.brush.is_none() {
                    self.toggle_brush();
                }
                ui.close();
            }
            if ui.button(if mask.enabled { t!("Disable mask") } else { t!("Enable mask") }).clicked() {
                self.checkpoint();
                if let Some(m) = &mut self.decals[index].layer.mask {
                    m.enabled = !m.enabled;
                }
                ui.close();
            }
            if ui.button(t!("Invert mask")).clicked() {
                self.checkpoint();
                if let Some(m) = &mut self.decals[index].layer.mask {
                    m.invert = !m.invert;
                }
                ui.close();
            }
            ui.separator();
            if ui.button(t!("Delete mask")).clicked() {
                self.checkpoint();
                self.decals[index].layer.mask = None;
                self.brush = None;
                ui.close();
            }
        });
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// マスクを筆で描く・やめる。
    pub(super) fn toggle_brush(&mut self) {
        if self.brush.is_some() {
            self.brush = None;
        } else if self.selected.is_some_and(|i| self.decals.get(i).is_some_and(|d| d.layer.mask.is_some() && !d.layer.sealed())) {
            self.brush = Some(Brush::default());
            self.say(t!("Painting on the mask: drag on the box or the dieline. Black hides, white shows. X switches, [ and ] change the size, Esc ends."));
        }
    }

    /// 筆で描く相手（選んでいる、マスクのある、ロックしていない柄）。
    pub(super) fn brush_target(&self) -> Option<usize> {
        self.brush.as_ref()?;
        self.selected.filter(|&i| self.decals.get(i).is_some_and(|d| d.layer.mask.is_some() && !d.layer.sealed() && is_artwork(&d.picture)))
    }

    /// 筆で、展開図の点 `at`（パネル `panel`、`back` の面）に描く。`start` なら、ひと続きの線の始まり。
    pub(super) fn brush_at(&mut self, panel: usize, at: P2, back: bool, start: bool) {
        let Some(index) = self.brush_target() else { return };
        if self.decals[index].back != back {
            return;
        }
        let decal = self.decals[index].decal;
        let Some((_, q)) = self.surface.decal_at(&self.template, std::slice::from_ref(&decal), panel, at) else {
            if let Some(b) = &mut self.brush {
                b.last = None;
            }
            return;
        };
        if start {
            self.checkpoint();
            if let Some(b) = &mut self.brush {
                b.last = None;
            }
        }
        let q = if back { [-q[0], q[1]] } else { q };
        let Some(brush) = self.brush.as_mut() else { return };
        let Some(mask) = self.decals[index].layer.mask.as_mut() else { return };
        let (mw, mh) = (mask.w as usize, mask.h as usize);
        let size = decal.size;
        let p = [(q[0] / size[0] + 0.5) * mw as f32, (0.5 - q[1] / size[1]) * mh as f32];
        // 半径（マスクの画素で、横と縦）。
        let r = [brush.size * 0.5 / size[0] * mw as f32, brush.size * 0.5 / size[1] * mh as f32];
        // 反転したマスクでは、見た目どおりになるように、描く値も逆にする。
        let value = if brush.hide != mask.invert { 0.0 } else { 255.0 };
        let hardness = brush.hardness.clamp(0.0, 1.0);
        let data = Arc::make_mut(&mut mask.data);
        let from = brush.last.unwrap_or(p);
        let dist = ((p[0] - from[0]) / r[0].max(0.5)).hypot((p[1] - from[1]) / r[1].max(0.5));
        let steps = (dist * 4.0).ceil().max(1.0) as usize;
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let c = [from[0] + (p[0] - from[0]) * t, from[1] + (p[1] - from[1]) * t];
            let (x0, x1) = (((c[0] - r[0]).floor().max(0.0)) as usize, ((c[0] + r[0]).ceil().max(0.0) as usize).min(mw));
            let (y0, y1) = (((c[1] - r[1]).floor().max(0.0)) as usize, ((c[1] + r[1]).ceil().max(0.0) as usize).min(mh));
            for y in y0..y1 {
                for x in x0..x1 {
                    let d = ((x as f32 + 0.5 - c[0]) / r[0].max(0.5)).hypot((y as f32 + 0.5 - c[1]) / r[1].max(0.5));
                    if d >= 1.0 {
                        continue;
                    }
                    // 硬さのぶんは塗りつぶし、残りは縁へ向かって薄くする。1 回ぶんの濃さは、重ねて塗ると濃くなるよう控えめに。
                    let edge = if d <= hardness { 1.0 } else { 1.0 - (d - hardness) / (1.0 - hardness).max(1e-3) };
                    let flow = edge * if start && s == 1 { 1.0 } else { 0.5 };
                    let o = y * mw + x;
                    let v = data[o] as f32;
                    data[o] = (v + (value - v) * flow).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        brush.last = Some(p);
        if back {
            self.back_dirty = true;
        } else {
            self.art_dirty = true;
        }
    }

    /// 筆の線が終わった。
    pub(super) fn brush_end(&mut self) {
        if let Some(b) = &mut self.brush {
            b.last = None;
        }
    }

    /// プロパティの欄の、筆の設定。キーでも変えられる（X で隠す・見せるを入れ替え、[ と ] で太さ、Esc で終わる）。
    pub(super) fn brush_options(&mut self, ui: &mut egui::Ui) {
        let typing = ui.ctx().egui_wants_keyboard_input();
        let Some(brush) = self.brush.as_mut() else { return };
        if !typing {
            ui.input(|i| {
                if i.key_pressed(egui::Key::X) {
                    brush.hide = !brush.hide;
                }
                if i.key_pressed(egui::Key::OpenBracket) {
                    brush.size = (brush.size / 1.25).max(0.5);
                }
                if i.key_pressed(egui::Key::CloseBracket) {
                    brush.size = (brush.size * 1.25).min(200.0);
                }
            });
        }
        let mut stop = !typing && ui.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(t!("Mask brush"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t!("Done")).clicked() {
                        stop = true;
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut brush.hide, true, t!("Hide (black)"));
                ui.selectable_value(&mut brush.hide, false, t!("Show (white)"));
            });
            ui.add(egui::Slider::new(&mut brush.size, 0.5..=200.0).logarithmic(true).text(t!("Size")).suffix(" mm"));
            let mut hard = brush.hardness * 100.0;
            if ui.add(egui::Slider::new(&mut hard, 0.0..=100.0).text(t!("Hardness")).suffix(" %")).changed() {
                brush.hardness = hard / 100.0;
            }
        });
        if stop {
            self.brush = None;
        }
    }
}

/// 調整の数値の欄。
fn adjust_numbers(ui: &mut egui::Ui, kind: &mut AdjustKind) {
    let slider = |ui: &mut egui::Ui, value: &mut i16, range: std::ops::RangeInclusive<i16>, label: &str| {
        let mut v = *value as i32;
        ui.add(egui::Slider::new(&mut v, *range.start() as i32..=*range.end() as i32).text(label));
        *value = v as i16;
    };
    match kind {
        AdjustKind::BrightnessContrast { brightness, contrast } => {
            slider(ui, brightness, -150..=150, t!("Brightness"));
            slider(ui, contrast, -50..=100, t!("Contrast"));
        }
        AdjustKind::HueSaturation { hue, saturation, lightness } => {
            slider(ui, hue, -180..=180, t!("Hue"));
            slider(ui, saturation, -100..=100, t!("Saturation"));
            slider(ui, lightness, -100..=100, t!("Lightness"));
        }
        AdjustKind::Levels { black, white, gamma } => {
            let (mut b, mut w) = (*black as i32, *white as i32);
            ui.add(egui::Slider::new(&mut b, 0..=253).text(t!("Black point")));
            ui.add(egui::Slider::new(&mut w, 2..=255).text(t!("White point")));
            let mut g = *gamma as f32 / 100.0;
            ui.add(egui::Slider::new(&mut g, 0.1..=9.99).logarithmic(true).max_decimals(2).text(t!("Midtones")));
            *kind = AdjustKind::Levels { black: b as u8, white: w as u8, gamma: (g * 100.0).round() as u16 }.clamped();
        }
        AdjustKind::ClearBackground { tolerance } => {
            let mut v = *tolerance as i32;
            ui.add(egui::Slider::new(&mut v, 1..=200).text(t!("Tolerance"))).on_hover_text(t!("How close a color must be to count as the background. Larger values make more of the picture transparent"));
            *tolerance = v as u8;
        }
        AdjustKind::ReduceColors { colors } => {
            let mut v = *colors as i32;
            ui.add(egui::Slider::new(&mut v, 2..=64).text(t!("Colors")));
            *colors = v as u8;
        }
        AdjustKind::Subject { fine } => {
            ui.checkbox(fine, t!("High quality (downloads a 176 MB model the first time)")).on_hover_text(t!("Uses a larger model for finer edges. It is downloaded once and kept on this PC"));
            if *fine && !crate::models::fine_model_ready() && !matches!(crate::models::state(), crate::models::Fetch::Running(..)) {
                ui.weak(t!("The high-quality model is not on this PC yet. Until it is, nothing is cut out."));
            }
        }
        AdjustKind::Curves { points, count } => {
            curve_editor(ui, points, count);
        }
        AdjustKind::Exposure { exposure, offset, gamma } => {
            let mut e = *exposure as f32 / 100.0;
            ui.add(egui::Slider::new(&mut e, -20.0..=20.0).max_decimals(2).text(t!("Exposure")));
            *exposure = (e * 100.0).round() as i16;
            let mut o = *offset as f32 / 1000.0;
            ui.add(egui::Slider::new(&mut o, -0.5..=0.5).max_decimals(3).text(t!("Offset")));
            *offset = (o * 1000.0).round() as i16;
            let mut g = *gamma as f32 / 100.0;
            ui.add(egui::Slider::new(&mut g, 0.01..=9.99).logarithmic(true).max_decimals(2).text(t!("Gamma correction")));
            *gamma = (g * 100.0).round().clamp(1.0, 999.0) as u16;
        }
        AdjustKind::Vibrance { vibrance, saturation } => {
            slider(ui, vibrance, -100..=100, t!("Vibrance"));
            slider(ui, saturation, -100..=100, t!("Saturation"));
        }
        AdjustKind::ColorBalance { shadows, midtones, highlights, preserve } => {
            for (label, values) in [(t!("Shadows"), shadows), (t!("Midtones"), midtones), (t!("Highlights"), highlights)] {
                ui.label(label);
                for (k, (lo, hi)) in [(t!("Cyan"), t!("Red")), (t!("Magenta"), t!("Green")), (t!("Yellow"), t!("Blue"))].into_iter().enumerate() {
                    let mut v = values[k] as i32;
                    ui.horizontal(|ui| {
                        ui.small(lo);
                        ui.add(egui::Slider::new(&mut v, -100..=100));
                        ui.small(hi);
                    });
                    values[k] = v as i8;
                }
            }
            ui.checkbox(preserve, t!("Preserve luminosity"));
        }
        AdjustKind::BlackWhite { weights } => {
            for (k, label) in [t!("Reds"), t!("Yellows"), t!("Greens"), t!("Cyans"), t!("Blues"), t!("Magentas")].into_iter().enumerate() {
                let mut v = weights[k] as i32;
                ui.add(egui::Slider::new(&mut v, -200..=300).suffix(" %").text(label));
                weights[k] = v as i16;
            }
        }
        AdjustKind::PhotoFilter { color, density, preserve } => {
            ui.horizontal(|ui| {
                ui.label(t!("Color"));
                ui.color_edit_button_srgb(color);
            });
            let mut d = *density as i32;
            ui.add(egui::Slider::new(&mut d, 0..=100).suffix(" %").text(t!("Density")));
            *density = d as u8;
            ui.checkbox(preserve, t!("Preserve luminosity"));
        }
        AdjustKind::Posterize { levels } => {
            let mut v = *levels as i32;
            ui.add(egui::Slider::new(&mut v, 2..=255).text(t!("Levels")));
            *levels = v as u8;
        }
        AdjustKind::Threshold { level } => {
            let mut v = *level as i32;
            ui.add(egui::Slider::new(&mut v, 1..=255).text(t!("Threshold level")));
            *level = v as u8;
        }
        AdjustKind::GradientMap { from, to, reverse } => {
            ui.horizontal(|ui| {
                ui.label(t!("Dark"));
                ui.color_edit_button_srgb(from);
                ui.label(t!("Light"));
                ui.color_edit_button_srgb(to);
            });
            ui.checkbox(reverse, t!("Reverse"));
        }
        AdjustKind::Invert | AdjustKind::FlattenShading => {}
    }
}

/// トーンカーブの欄。点をドラッグで動かし、線の上を押すと点を足す（8 つまで）。点をダブルクリックか右クリックで消す（両端は消せない）。
fn curve_editor(ui: &mut egui::Ui, points: &mut [[u8; 2]; 8], count: &mut u8) {
    let side = 160.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    for k in 1..4 {
        let t = k as f32 / 4.0;
        let stroke = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.3));
        painter.line_segment([egui::pos2(rect.left() + side * t, rect.top()), egui::pos2(rect.left() + side * t, rect.bottom())], stroke);
        painter.line_segment([egui::pos2(rect.left(), rect.top() + side * t), egui::pos2(rect.right(), rect.top() + side * t)], stroke);
    }
    let to_screen = |p: [u8; 2]| egui::pos2(rect.left() + p[0] as f32 / 255.0 * side, rect.bottom() - p[1] as f32 / 255.0 * side);
    let from_screen = |p: egui::Pos2| [(((p.x - rect.left()) / side) * 255.0).round().clamp(0.0, 255.0) as u8, (((rect.bottom() - p.y) / side) * 255.0).round().clamp(0.0, 255.0) as u8];
    let n = *count as usize;
    let drag_id = response.id.with("point");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let near = |p: egui::Pos2, pts: &[[u8; 2]]| pts.iter().enumerate().map(|(i, q)| (i, to_screen(*q).distance(p))).filter(|(_, d)| *d < 8.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
    if response.drag_started() && let Some(p) = response.interact_pointer_pos() {
        dragging = near(p, &points[..n]);
        if dragging.is_none() && n < 8 {
            points[n] = from_screen(p);
            *count += 1;
            dragging = Some(n);
        }
    }
    if response.clicked() && let Some(p) = response.interact_pointer_pos() && near(p, &points[..n]).is_none() && n < 8 {
        points[n] = from_screen(p);
        *count += 1;
    }
    if (response.double_clicked() || response.secondary_clicked()) && let Some(p) = response.interact_pointer_pos() && let Some(i) = near(p, &points[..n]) {
        let mut sorted: Vec<[u8; 2]> = points[..n].to_vec();
        let target = sorted[i];
        sorted.sort_by_key(|q| q[0]);
        let pos = sorted.iter().position(|q| *q == target).unwrap_or(0);
        if n > 2 && pos != 0 && pos != n - 1 {
            sorted.remove(pos);
            points[..sorted.len()].copy_from_slice(&sorted);
            *count -= 1;
        }
        dragging = None;
    }
    if response.dragged() && let (Some(i), Some(p)) = (dragging, response.interact_pointer_pos()) && i < *count as usize {
        points[i] = from_screen(p);
    }
    if response.drag_stopped() {
        dragging = None;
    }
    ui.data_mut(|d| d.insert_temp(drag_id, dragging));
    let n = *count as usize;
    let lut = crate::effects::curve_lut(&points[..n]);
    let line: Vec<egui::Pos2> = (0..=64).map(|k| { let x = k * 255 / 64; egui::pos2(rect.left() + x as f32 / 255.0 * side, rect.bottom() - lut[x] * side) }).collect();
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.6, visuals.text_color())));
    for p in &points[..n] {
        painter.circle(to_screen(*p), 3.5, visuals.extreme_bg_color, egui::Stroke::new(1.4, visuals.strong_text_color()));
    }
    response.on_hover_text(t!("Drag a point to bend the curve. Click on the curve to add a point (up to 8). Double-click or right-click a point to remove it"));
    if ui.small_button(t!("Reset")).clicked() {
        *points = pakenoba_core::project::CURVE_STRAIGHT;
        *count = 2;
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::art::{PX_PER_MM, PsdGroup};
    use pakenoba_core::project::{ShapeKind, ShapeSpec};

    /// 正面の中央に、塗りの四角を置いた案件（ほかの柄はない）。
    fn app_with(colors: &[[u8; 3]]) -> App {
        let mut app = App::new(&Initial::default());
        app.decals.clear();
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        for &c in colors {
            let spec = ShapeSpec { kind: ShapeKind::Rect, fill: Some(c), stroke: None, stroke_width: 1.0, radius: 0.0 };
            app.decals.push(Placed { decal: Decal { panel, at: [0.5, 0.5], size: [30.0, 30.0], angle: 0.0 }, picture: Picture::Shape(spec), back: false, layer: Layer::default() });
        }
        app
    }

    /// 柄 0 の中央の、統合した絵の色。
    fn center(app: &App) -> [u8; 3] {
        let c = app.decals.iter().find(|d| is_artwork(&d.picture)).unwrap().center(&app.template);
        let tpl = &app.template;
        let (x, y) = (((c[0] - tpl.min[0]) * PX_PER_MM) as usize, ((tpl.max[1] - c[1]) * PX_PER_MM) as usize);
        let art = app.painted(false);
        let o = (y * app.sheet.w + x) * 4;
        [art[o], art[o + 1], art[o + 2]]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
    }

    #[test]
    fn グループにまとめて_出して_解く() {
        let mut app = app_with(&[[255, 0, 0], [0, 0, 255], [0, 255, 0]]);
        app.select_only(Some(0));
        app.extra = vec![1];
        app.group_selected();
        // 下から: 赤、青（中身）、見出し、緑。
        assert!(matches!(app.decals[2].picture, Picture::Group { .. }));
        assert_eq!(members_of(&app.decals, 2), vec![0, 1]);
        assert!(app.decals[3].layer.depth == 0);
        // いちばん上の中身を上へ動かすと、グループから出て、見出しの上へ。
        app.select_only(Some(1));
        app.reorder_selected(1);
        assert_eq!(members_of(&app.decals, 1), vec![0]);
        assert!(app.decals[2].layer.depth == 0 && matches!(app.decals[2].picture, Picture::Shape(_)));
        // もう一度下へ動かすと、いちばん上の中身として戻る。
        app.reorder_selected(-1);
        assert_eq!(members_of(&app.decals, 2), vec![0, 1]);
        // グループを解くと、中身は残る。
        app.ungroup(2);
        assert_eq!(app.decals.len(), 3);
        assert!(app.decals.iter().all(|d| d.layer.depth == 0 && is_artwork(&d.picture)));
        // 元に戻せる。
        app.undo();
        assert_eq!(members_of(&app.decals, 2), vec![0, 1]);
    }

    #[test]
    fn 通過でないグループは_中身を1枚にしてから重ねる() {
        // 赤の上に青。グループの不透明度 50%。
        let mut app = app_with(&[[255, 0, 0], [0, 0, 255]]);
        app.select_only(Some(0));
        app.extra = vec![1];
        app.group_selected();
        app.decals[2].layer.opacity = 50;
        // 通過: 赤 50%、その上に青 50%。
        let pass = center(&app);
        // 通常: 中身（青）を 1 枚にしてから 50%。白の上の青 50%。
        app.decals[2].layer.blend = Blend::Normal;
        let normal = center(&app);
        // グループを隠すと、中身も見えない（紙の色だけ）。
        app.decals[2].layer.hidden = true;
        let paper = center(&app);
        assert!(!crate::paint::shown_in_group(&app.decals, 1));
        // 紙の上の青 50%。赤は混ざらない。
        let half = |p: u8, c: u8| (p as u32 + c as u32).div_ceil(2) as u8;
        assert!(near(normal, [half(paper[0], 0), half(paper[1], 0), half(paper[2], 255)]), "{normal:?} {paper:?}");
        assert!(pass[1] + 30 < normal[1] && pass[2] + 30 < normal[2], "通過では、下の赤が透ける {pass:?}");
    }

    #[test]
    fn 調整レイヤーは下に効き_柄だけの調整はその柄だけに効く() {
        let mut app = app_with(&[[255, 0, 0]]);
        let plain = center(&app);
        assert!(near(plain, [255, 0, 0]));
        app.select_only(Some(0));
        app.add_adjustment_layer(AdjustKind::Invert);
        assert!(matches!(app.decals[1].picture, Picture::Adjustment));
        assert!(near(center(&app), [0, 255, 255]), "{:?}", center(&app));
        // 不透明度 0 なら効かない。目を閉じても効かない。
        app.decals[1].layer.opacity = 0;
        assert!(near(center(&app), [255, 0, 0]));
        app.decals[1].layer.opacity = 100;
        app.decals[1].layer.adjust[0].on = false;
        assert!(near(center(&app), [255, 0, 0]));
        // 柄だけの調整（色相を 120 度）。書き出しでは、できるまで待つ。
        app.decals.remove(1);
        app.select_only(Some(0));
        app.add_adjustment(AdjustKind::HueSaturation { hue: 120, saturation: 0, lightness: 0 });
        assert!(near(center(&app), [0, 255, 0]), "{:?}", center(&app));
    }

    #[test]
    fn マスクで隠し_筆で描ける() {
        let mut app = app_with(&[[255, 0, 0]]);
        app.select_only(Some(0));
        app.add_mask(MaskFrom::RevealAll);
        assert!(near(center(&app), [255, 0, 0]));
        // 反転すると、すべて隠れる。
        app.decals[0].layer.mask.as_mut().unwrap().invert = true;
        assert!(near(center(&app), [255, 255, 255]) || center(&app)[1] > 200, "{:?}", center(&app));
        app.decals[0].layer.mask.as_mut().unwrap().invert = false;
        // 筆で中央を隠す。
        app.toggle_brush();
        assert!(app.brush_target().is_some());
        let at = app.decals[0].center(&app.template);
        let panel = app.decals[0].panel;
        app.brush_at(panel, at, false, true);
        app.brush_end();
        let m = app.decals[0].layer.mask.clone().unwrap();
        assert!(m.data[(m.h / 2 * m.w + m.w / 2) as usize] < 10);
        assert_eq!(m.data[0], 255, "端は描いていない");
        assert!(center(&app)[1] > 200, "中央は隠れて、下の紙が見える {:?}", center(&app));
        // 元に戻すと、描く前のマスク。
        app.undo();
        let m = app.decals[0].layer.mask.clone().unwrap();
        assert!(m.data.iter().all(|&v| v == 255));
    }

    #[test]
    fn グループと調整とマスクは_保存して開くと戻り_型紙のpsdにも入る() {
        let mut app = app_with(&[[255, 0, 0], [0, 0, 255]]);
        app.select_only(Some(0));
        app.add_adjustment(AdjustKind::Levels { black: 20, white: 230, gamma: 140 });
        app.add_mask(MaskFrom::RevealAll);
        app.extra = vec![1];
        app.group_selected();
        app.decals[2].layer.blend = Blend::Multiply;
        app.select_only(Some(2));
        app.add_adjustment_layer(AdjustKind::BrightnessContrast { brightness: 30, contrast: 10 });
        let project = app.project().unwrap();
        // 画面を確かめるための見本（PAKENOBA_SAMPLE に保存先を書いたときだけ）。
        if let Some(path) = std::env::var_os("PAKENOBA_SAMPLE") {
            project.save(std::path::Path::new(&path)).unwrap();
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        project.write(&mut bytes).unwrap();
        let opened = pakenoba_core::project::Project::read(std::io::Cursor::new(bytes.into_inner())).unwrap();
        assert!(opened.problems.is_empty(), "{:?}", opened.problems);
        let mut again = App::new(&Initial::default());
        let problems = again.apply_project(opened.project);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(again.decals.len(), app.decals.len());
        for (a, b) in again.decals.iter().zip(&app.decals) {
            assert_eq!(a.layer, b.layer);
            assert_eq!(std::mem::discriminant(&a.picture), std::mem::discriminant(&b.picture));
        }
        assert_eq!(center(&again), center(&app));
        // PSD: グループの始まりと終わり、マスク、調整レイヤー。
        let layers = again.decal_layers(false);
        assert!(matches!(layers[0].group, Some(PsdGroup::Start { .. })));
        assert!(layers[1].mask.is_some());
        assert!(matches!(layers[3].group, Some(PsdGroup::End)));
        assert!(layers[4].group.is_none() && layers[4].image.iter().any(|&v| v > 0));
        let psd = crate::art::save_psd(&again.sheet, None, &layers, Some(&again.painted(false))).unwrap();
        let file = craft_psd::PsdFile::from_bytes(&psd).unwrap();
        assert!(file.iter_layers().count() >= 5);
    }

    #[test]
    fn 画像のロックと位置のロックは_それぞれ別のものを止める() {
        let mut app = app_with(&[[255, 0, 0]]);
        app.select_only(Some(0));
        // 画像をロック: 調整もマスクも足せない。つかんで動かすことはできる。
        app.decals[0].layer.lock_pixels = true;
        app.add_adjustment(AdjustKind::Invert);
        assert!(app.decals[0].layer.adjust.is_empty());
        assert_eq!(crate::paint::spots_on(&app.decals, false).0, vec![0]);
        // 位置をロック: つかめないが、調整は足せる。
        app.decals[0].layer.lock_pixels = false;
        app.decals[0].layer.lock_position = true;
        assert!(crate::paint::spots_on(&app.decals, false).0.is_empty());
        app.add_adjustment(AdjustKind::Invert);
        assert_eq!(app.decals[0].layer.adjust.len(), 1);
        assert!(app.knob_target(None).is_none());
        // 小さな絵は、柄の色（赤）を出す。
        let thumb = picture_thumb(&app.decals[0]);
        let mid = (THUMB_PX / 2 * THUMB_PX + THUMB_PX / 2) * 4;
        assert!(thumb[mid] > 200 && thumb[mid + 1] < 60, "{:?}", &thumb[mid..mid + 4]);
    }

    #[test]
    fn グループの中にグループを作れて_クリップは土台の見える所にだけ出る() {
        // 下から: 赤（土台）、青（クリップ）、緑。
        let mut app = app_with(&[[255, 0, 0], [0, 0, 255], [0, 255, 0]]);
        // 緑を隠して、青を赤でクリップ。赤を小さくすると、はみ出た青は見えない。
        app.decals[2].layer.hidden = true;
        app.toggle_clip(1);
        assert!(app.decals[1].layer.clipped);
        assert!(near(center(&app), [0, 0, 255]));
        app.decals[0].layer.hidden = true;
        let paper = center(&app);
        assert!(paper[2] < 250 || paper[0] > 200, "土台が隠れると、クリップしたものも見えない {paper:?}");
        assert!(!near(paper, [0, 0, 255]));
        app.decals[0].layer.hidden = false;
        // いちばん下はクリップできない。
        app.toggle_clip(0);
        assert!(!app.decals[0].layer.clipped);
        // 赤と青をグループにし、そのグループと緑を、さらにグループに（入れ子）。
        app.select_only(Some(0));
        app.extra = vec![1];
        app.group_selected();
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![1, 1, 0, 0]);
        assert!(app.decals[1].layer.clipped, "グループの中でも、クリップは続く");
        app.select_only(Some(2));
        app.extra = vec![3];
        app.group_selected();
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![2, 2, 1, 1, 0]);
        assert_eq!(crate::paint::ancestors(&app.decals, 0), vec![2, 4]);
        // 外のグループを隠すと、中のものも見えない。
        app.decals[4].layer.hidden = true;
        assert!(!crate::paint::shown_in_group(&app.decals, 0));
        app.decals[4].layer.hidden = false;
        // 外のグループを解くと、1 段浅くなる。
        app.ungroup(4);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![1, 1, 0, 0]);
        // PSD では、クリップと入れ子のグループが入る。
        app.select_only(Some(2));
        app.extra = vec![3];
        app.group_selected();
        // 画面を確かめるための見本（PAKENOBA_SAMPLE2 に保存先を書いたときだけ）。トーンカーブの調整レイヤーも足す。
        if let Some(path) = std::env::var_os("PAKENOBA_SAMPLE2") {
            let mut sample = app.project().unwrap();
            let mut layer = sample.decals[0].clone();
            layer.source = pakenoba_core::project::Source::Adjustment;
            layer.layer = Layer { depth: 0, adjust: vec![Adjustment::new(AdjustKind::Curves { points: [[0, 0], [90, 150], [255, 255], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0]], count: 3 })], ..Layer::default() };
            sample.decals.push(layer);
            sample.save(std::path::Path::new(&path)).unwrap();
        }
        let layers = app.decal_layers(false);
        let starts = layers.iter().filter(|l| matches!(l.group, Some(PsdGroup::Start { .. }))).count();
        let ends = layers.iter().filter(|l| matches!(l.group, Some(PsdGroup::End))).count();
        assert_eq!((starts, ends), (2, 2));
        assert!(layers.iter().any(|l| l.clipped));
        let psd = crate::art::save_psd(&app.sheet, None, &layers, Some(&app.painted(false))).unwrap();
        assert!(craft_psd::PsdFile::from_bytes(&psd).is_ok());
    }

    #[test]
    fn 並べ替えで_グループを出入りする() {
        let mut app = app_with(&[[255, 0, 0], [0, 0, 255], [0, 255, 0]]);
        app.select_only(Some(1));
        app.group_selected();
        // 下から: 赤、青(1)、見出し、緑。
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        // 赤を上へ: グループのいちばん下の中身になる。
        app.select_only(Some(0));
        app.reorder_selected(1);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![1, 1, 0, 0]);
        // もう一度下へ: グループから出る。
        app.reorder_selected(-1);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        // 緑を下へ: グループのいちばん上の中身になる。
        app.select_only(Some(3));
        app.reorder_selected(-1);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![0, 1, 1, 0]);
        assert!(matches!(app.decals[3].picture, Picture::Group { .. }));
        // ドラッグで見出しのすぐ下に落とすと、いちばん上の中身。上に落とすと、グループの外。
        app.move_layer(0, 3, false);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![1, 1, 1, 0]);
        app.move_layer(2, 3, true);
        assert_eq!(app.decals.iter().map(|d| d.layer.depth).collect::<Vec<_>>(), vec![1, 1, 0, 0]);
        assert!(!matches!(app.decals[3].picture, Picture::Group { .. }));
    }
}
