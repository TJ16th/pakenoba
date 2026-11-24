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

/// 柄だけに効く調整の種類（はじめの値）。
pub(in crate::app) const ARTWORK_ADJUST: [AdjustKind; 8] = [
    AdjustKind::BrightnessContrast { brightness: 0, contrast: 0 },
    AdjustKind::HueSaturation { hue: 0, saturation: 0, lightness: 0 },
    AdjustKind::Levels { black: 0, white: 255, gamma: 100 },
    AdjustKind::Invert,
    AdjustKind::Subject { fine: false },
    AdjustKind::ClearBackground { tolerance: 40 },
    AdjustKind::FlattenShading,
    AdjustKind::ReduceColors { colors: 6 },
];

/// 調整レイヤー（下のすべてに効く）にできる種類。
pub(in crate::app) const LAYER_ADJUST: [AdjustKind; 4] = [ARTWORK_ADJUST[0], ARTWORK_ADJUST[1], ARTWORK_ADJUST[2], ARTWORK_ADJUST[3]];

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
    }
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

    /// グループの決まり（中身は、見出しのすぐ前に続ける）に合わない印を直す。
    pub(super) fn tidy_groups(&mut self) {
        let groups: Vec<bool> = self.decals.iter().map(|d| matches!(d.picture, Picture::Group { .. })).collect();
        let backs: Vec<bool> = self.decals.iter().map(|d| d.back).collect();
        let mut flags: Vec<bool> = self.decals.iter().map(|d| d.layer.in_group).collect();
        pakenoba_core::project::group_rule(&groups, &backs, &mut flags);
        for (d, f) in self.decals.iter_mut().zip(flags) {
            d.layer.in_group = f;
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
                    if let Some(g) = group_of(&self.decals, index)
                        && matches!(self.decals[g].picture, Picture::Group { open: false })
                    {
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
            if layer.in_group {
                ui.add_space(16.0);
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
                    if self.decals[index].layer.in_group && ui.button(t!("Take out of the group")).clicked() {
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
        let indent = if self.decals[index].layer.in_group { 34.0 } else { 20.0 };
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
                    ui.add_enabled_ui(artwork.is_some(), |ui| {
                        ui.weak(t!("This artwork only"));
                        for kind in ARTWORK_ADJUST {
                            if ui.button(adjust_label(kind)).clicked() {
                                self.add_adjustment(kind);
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    ui.weak(t!("Adjustment layer (everything below)"));
                    for kind in LAYER_ADJUST {
                        if ui.button(adjust_label(kind)).clicked() {
                            self.add_adjustment_layer(kind);
                            ui.close();
                        }
                    }
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
        // グループの中へ入る場所なら、線を字下げする。
        let into = !matches!(self.decals[dragged].picture, Picture::Group { .. }) && (self.decals[over].layer.in_group || (!above && matches!(self.decals[over].picture, Picture::Group { .. })));
        let x = if into { rect.left() + 18.0 } else { rect.left() };
        ui.painter().hline(x..=rect.right(), y, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
        if ui.input(|i| i.pointer.any_released()) {
            self.layer_drag = None;
            self.move_layer(dragged, over, above);
        }
    }

    /// 柄やグラフの塊（グループなら、中身と見出し）。並びの中の番号（下から）。
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

    /// 柄 `dragged` を、行 `over` のすぐ上（`above`）か、すぐ下へ動かす。グループの中身の間に落とせば、そのグループに入る。
    pub(super) fn move_layer(&mut self, dragged: usize, over: usize, above: bool) {
        if dragged >= self.decals.len() || over >= self.decals.len() || dragged == over {
            return;
        }
        let block = self.block(dragged);
        if block.contains(&over) {
            return;
        }
        let is_group = matches!(self.decals[dragged].picture, Picture::Group { .. });
        let over_group = matches!(self.decals[over].picture, Picture::Group { .. });
        let over_member = self.decals[over].layer.in_group;
        // 入る先と、グループに入るか。
        let (anchor, after, into) = if is_group {
            // グループは、ほかのグループの中には入れない。中身の上に落としたら、そのグループの外へ。
            match (over_member, group_of(&self.decals, over)) {
                (true, Some(g)) if above => (g, true, false),
                (true, Some(g)) => (members_of(&self.decals, g)[0], false, false),
                _ if over_group && !above => (members_of(&self.decals, over).first().copied().unwrap_or(over), false, false),
                _ => (over, above, false),
            }
        } else if over_group && !above {
            // 見出しのすぐ下: グループのいちばん上の中身になる。
            (over, false, true)
        } else {
            (over, above, over_member)
        };
        self.checkpoint();
        let moving: Vec<Placed> = block.iter().map(|&i| self.decals[i].clone()).collect();
        let anchor_after_removal = anchor - block.iter().filter(|&&i| i < anchor).count();
        let mut sorted = block.clone();
        sorted.sort_unstable();
        for &i in sorted.iter().rev() {
            self.decals.remove(i);
        }
        let at = if after { anchor_after_removal + 1 } else { anchor_after_removal };
        let count = moving.len();
        for (k, mut item) in moving.into_iter().enumerate() {
            if !is_group {
                item.layer.in_group = into;
            }
            self.decals.insert(at + k, item);
        }
        self.tidy_groups();
        self.select_only(Some(at + count - 1));
        self.grab = None;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 選んでいる柄を、同じ面の中で、重なりの上（+1）か下（-1）へ 1 つ動かす。グループの端では、グループを出入りする。
    pub(super) fn reorder_selected(&mut self, step: isize) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else { return };
        let Some(other) = self.neighbor(index, step) else { return };
        let header = |i: usize, s: &Self| matches!(s.decals[i].picture, Picture::Group { .. });
        if header(index, self) {
            // グループごと、となりの塊（柄 1 つか、グループ 1 つ）の向こうへ。
            let target = if step > 0 { other } else { group_of(&self.decals, other).unwrap_or(other) };
            let target = if step > 0 && self.decals[target].layer.in_group { group_of(&self.decals, target).unwrap_or(target) } else { target };
            let over = if step < 0 { self.block(target)[0] } else { target };
            self.move_layer(index, over, step > 0);
            return;
        }
        self.checkpoint();
        let member = self.decals[index].layer.in_group;
        let to = if step > 0 && member && header(other, self) {
            // グループのいちばん上から出る: 見出しの上へ。
            self.decals[index].layer.in_group = false;
            self.decals.swap(index, other);
            other
        } else if step < 0 && member && !self.decals[other].layer.in_group {
            // グループのいちばん下から出る: その場で、グループの下になる。
            self.decals[index].layer.in_group = false;
            index
        } else if step > 0 && !member && (self.decals[other].layer.in_group || header(other, self)) {
            // 上のグループに、いちばん下の中身として入る。
            self.decals[index].layer.in_group = true;
            index
        } else if step < 0 && !member && header(other, self) {
            // 下のグループに、いちばん上の中身として入る。
            self.decals[index].layer.in_group = true;
            self.decals.swap(index, other);
            other
        } else {
            self.decals.swap(index, other);
            other
        };
        self.tidy_groups();
        self.select_only(Some(to));
        self.grab = None;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// グループを作り、選んでいるレイヤーを入れる。何も選んでいなければ、空のグループを表のいちばん上に作る。
    pub(super) fn group_selected(&mut self) {
        let back = self.selected.and_then(|i| self.decals.get(i)).is_some_and(|d| d.back);
        let mut chosen: Vec<usize> = self.selection().into_iter().filter(|&i| self.decals[i].back == back && !matches!(self.decals[i].picture, Picture::Group { .. })).collect();
        chosen.sort_unstable();
        self.checkpoint();
        let number = self.decals.iter().filter(|d| matches!(d.picture, Picture::Group { .. })).count() + 1;
        let header = Placed { decal: NOWHERE, picture: Picture::Group { open: true }, back, layer: Layer { name: Some(t!("Group {number}")), blend: Blend::PassThrough, ..Layer::default() } };
        if chosen.is_empty() {
            self.decals.push(header);
            self.select_only(Some(self.decals.len() - 1));
            (self.art_dirty, self.back_dirty) = (true, true);
            return;
        }
        let top = *chosen.last().unwrap();
        let mut moving: Vec<Placed> = chosen.iter().map(|&i| self.decals[i].clone()).collect();
        for &i in chosen.iter().rev() {
            self.decals.remove(i);
        }
        let mut at = top + 1 - chosen.len();
        // 入れる場所がほかのグループの中なら、そのグループの上へ。
        if let Some(below) = (0..at).rev().find(|&i| self.decals[i].back == back)
            && self.decals[below].layer.in_group
            && let Some(g) = group_of(&self.decals, below)
        {
            at = g + 1;
        }
        let count = moving.len();
        for item in &mut moving {
            item.layer.in_group = true;
        }
        for (k, item) in moving.into_iter().enumerate() {
            self.decals.insert(at + k, item);
        }
        self.decals.insert(at + count, header);
        self.tidy_groups();
        self.select_only(Some(at + count));
        self.extra = (at..at + count).collect();
        (self.art_dirty, self.back_dirty) = (true, true);
        self.say(t!("Grouped {count} layer(s). Press Ctrl+Z to undo."));
    }

    /// グループを解く（中身は、そのまま残す）。
    pub(super) fn ungroup(&mut self, header: usize) {
        if !matches!(self.decals.get(header).map(|d| &d.picture), Some(Picture::Group { .. })) {
            return;
        }
        self.checkpoint();
        for i in members_of(&self.decals, header) {
            self.decals[i].layer.in_group = false;
        }
        self.decals.remove(header);
        self.select_only(None);
        self.tidy_groups();
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 柄を、グループの外（グループのすぐ上）へ出す。
    fn take_out_of_group(&mut self, index: usize) {
        let Some(g) = group_of(&self.decals, index) else { return };
        self.checkpoint();
        let mut item = self.decals.remove(index);
        item.layer.in_group = false;
        // 見出しは 1 つ下にずれている。
        self.decals.insert(g, item);
        self.tidy_groups();
        self.select_only(Some(g));
        (self.art_dirty, self.back_dirty) = (true, true);
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
        let in_group = index.is_some_and(|i| self.decals[i].layer.in_group);
        let at = index.map_or(self.decals.len(), |i| i + 1);
        self.checkpoint();
        let layer = Layer { adjust: vec![Adjustment::new(kind)], in_group, ..Layer::default() };
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
                        for kind in LAYER_ADJUST {
                            if ui.button(adjust_label(kind)).clicked() {
                                self.checkpoint();
                                self.decals[index].layer.adjust.push(Adjustment::new(kind));
                                (self.art_dirty, self.back_dirty) = (true, true);
                                ui.close();
                            }
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
        AdjustKind::Invert | AdjustKind::FlattenShading => {}
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
        assert!(!app.decals[3].layer.in_group);
        // いちばん上の中身を上へ動かすと、グループから出て、見出しの上へ。
        app.select_only(Some(1));
        app.reorder_selected(1);
        assert_eq!(members_of(&app.decals, 1), vec![0]);
        assert!(!app.decals[2].layer.in_group && matches!(app.decals[2].picture, Picture::Shape(_)));
        // もう一度下へ動かすと、いちばん上の中身として戻る。
        app.reorder_selected(-1);
        assert_eq!(members_of(&app.decals, 2), vec![0, 1]);
        // グループを解くと、中身は残る。
        app.ungroup(2);
        assert_eq!(app.decals.len(), 3);
        assert!(app.decals.iter().all(|d| !d.layer.in_group && is_artwork(&d.picture)));
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
}
