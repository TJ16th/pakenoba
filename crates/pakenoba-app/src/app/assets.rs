//! 素材の欄。この案件の素材（取り込んだ画像）と、付属の見本。
//!
//! 素材を、立体や展開図へドラッグして置く。落とした場所が紙の表か裏かで、置く面が決まる。
//! コレクション（フォルダの登録）は、ファイルのダイアログと設定の保存が要るので、まだ作っていない。

use std::sync::Arc;

use eframe::egui::{self, Color32, Rect, Sense, vec2};

use pakenoba_core::surface::Decal;
use pakenoba_core::template::P2;

use super::{App, Tool};
use crate::paint::{Picture, Placed};

/// 素材 1 つ。
pub(super) struct Asset {
    pub name: String,
    pub picture: Picture,
    /// 置くときの大きさ（mm）。
    pub size: P2,
    /// 一覧に出す小さい絵。初めて出すときに作る。
    pub thumb: Option<egui::TextureHandle>,
    /// 付属の見本か（そうでなければ、この案件に取り込んだ素材）。
    pub bundled: bool,
}

/// 付属の見本。
pub(super) fn bundled() -> Vec<Asset> {
    vec![
        Asset { name: "ロゴ（見本）".to_owned(), picture: Picture::Logo, size: [46.0, 46.0], thumb: None, bundled: true },
        Asset { name: "一周する帯（見本）".to_owned(), picture: Picture::Band, size: [120.0, 14.0], thumb: None, bundled: true },
    ]
}

/// 一覧に出す小さい絵（RGBA）。絵の縦横の比を保って、正方形の中央に収める。
pub(super) fn thumbnail(picture: &Picture, size: P2, side: usize) -> Vec<u8> {
    // 帯のように細長い絵は、そのままだと線になるので、見える太さまで切り出す。
    let shown = [size[0].min(size[1] * 4.0), size[1].min(size[0] * 4.0)];
    let mm_per_px = shown[0].max(shown[1]) / side as f32;
    let mut out = vec![0u8; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let q = [(x as f32 + 0.5 - side as f32 * 0.5) * mm_per_px, (side as f32 * 0.5 - y as f32 - 0.5) * mm_per_px];
            if q[0].abs() > shown[0] * 0.5 || q[1].abs() > shown[1] * 0.5 {
                continue;
            }
            let color = picture.sample(q, size);
            let o = (y * side + x) * 4;
            out[o..o + 4].copy_from_slice(&[color[0].round() as u8, color[1].round() as u8, color[2].round() as u8, (color[3].clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
    }
    out
}

/// 同じ絵か（画像は、同じデータを指しているかで見る）。
fn same_picture(a: &Picture, b: &Picture) -> bool {
    match (a, b) {
        (Picture::Logo, Picture::Logo) | (Picture::Band, Picture::Band) => true,
        (Picture::Image { rgba: x, .. }, Picture::Image { rgba: y, .. }) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

const THUMB: f32 = 52.0;

impl App {
    /// その素材を、箱で何か所に使っているか。
    pub(super) fn uses(&self, asset: usize) -> usize {
        let picture = &self.assets[asset].picture;
        self.decals.iter().filter(|decal| same_picture(&decal.picture, picture)).count()
    }

    /// 素材を、柄として置く。`at` は展開図上の位置、`back` が真なら紙の裏。
    pub(super) fn place_asset(&mut self, asset: usize, panel: usize, at: P2, back: bool) {
        let Some(source) = self.assets.get(asset) else { return };
        let size = if matches!(source.picture, Picture::Band) { [self.dims.front_and_around(&self.template).1, source.size[1]] } else { source.size };
        let mut decal = Decal { panel, at: [0.5, 0.5], size, angle: 0.0 };
        decal.set_center(&self.template, at);
        self.decals.push(Placed { picture: source.picture.clone(), decal, back });
        self.selected = Some(self.decals.len() - 1);
        self.tool = Tool::Select;
        self.art_dirty = true;
        self.back_dirty |= back;
    }

    /// 左（デザイン）: 素材の欄。
    pub(super) fn assets_panel(&mut self, ui: &mut egui::Ui) {
        if !self.assets_open {
            return;
        }
        egui::Panel::left("assets").resizable(true).default_size(196.0).size_range(150.0..=320.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.strong("素材");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tip = if self.assets_inside { "素材の欄を、道具の帯の左へ移す" } else { "素材の欄を、道具の帯の右へ移す" };
                    if ui.small_button("位置").on_hover_text(tip).clicked() {
                        self.assets_inside = !self.assets_inside;
                    }
                });
            });
            ui.small("箱や展開図へドラッグして置きます。");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for bundled in [false, true] {
                    ui.add_space(4.0);
                    ui.weak(if bundled { "付属の見本" } else { "この案件の素材" });
                    let list: Vec<usize> = (0..self.assets.len()).filter(|&i| self.assets[i].bundled == bundled).collect();
                    if list.is_empty() {
                        ui.small("画像（PNG）をウィンドウにドロップすると、ここに入ります。");
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for index in list {
                            self.asset_tile(ui, index);
                        }
                    });
                }
                ui.add_space(4.0);
                ui.weak("コレクション");
                ui.small("フォルダを登録して、案件をまたいで使う素材を並べる機能は、これから作ります。");
            });
        });
    }

    /// 素材 1 つぶんのマス。クリックで正面の中央に置く。ドラッグで、落とした場所に置く。
    fn asset_tile(&mut self, ui: &mut egui::Ui, index: usize) {
        let uses = self.uses(index);
        let asset = &mut self.assets[index];
        let (rect, response) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click_and_drag());
        let texture = asset.thumb.get_or_insert_with(|| {
            let side = 96;
            let image = egui::ColorImage::from_rgba_unmultiplied([side, side], &thumbnail(&asset.picture, asset.size, side));
            ui.ctx().load_texture(format!("asset-{index}"), image, egui::TextureOptions::LINEAR)
        });
        let visuals = ui.visuals();
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, if response.hovered() { visuals.widgets.hovered.bg_fill } else { visuals.extreme_bg_color });
        painter.image(texture.id(), rect.shrink(4.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        if uses > 0 {
            // 箱で使っている素材の印。
            painter.circle_filled(rect.right_bottom() - vec2(7.0, 7.0), 3.5, visuals.selection.bg_fill);
        }
        let name = asset.name.clone();
        let size = asset.size;
        let picture = asset.picture.clone();
        if response.drag_started() {
            self.carry = Some(index);
        }
        if response.clicked() {
            let size = if matches!(picture, Picture::Band) { [self.dims.front_and_around(&self.template).1, size[1]] } else { size };
            self.add_decal(picture, size);
            self.tool = Tool::Select;
        }
        response.on_hover_text(if uses > 0 { format!("{name}（箱で {uses} か所に使用）") } else { name });
    }

    /// 運んでいる途中の素材を、カーソルのそばに出す。
    pub(super) fn carry_ghost(&self, ui: &egui::Ui) {
        let (Some(asset), Some(pointer)) = (self.carry.and_then(|i| self.assets.get(i)), ui.input(|i| i.pointer.interact_pos())) else { return };
        let Some(texture) = &asset.thumb else { return };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("carried-asset")));
        let rect = Rect::from_center_size(pointer, vec2(THUMB, THUMB));
        painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(200));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 一覧の絵は_縦横の比を保って中央に収まる() {
        // 正方形のロゴは、マスいっぱい。中央は不透明、角は透明（丸いので）。
        let side = 32;
        let logo = thumbnail(&Picture::Logo, [46.0, 46.0], side);
        let alpha = |image: &[u8], x: usize, y: usize| image[(y * side + x) * 4 + 3];
        assert_eq!(alpha(&logo, side / 2, side / 2), 255);
        assert_eq!(alpha(&logo, 0, 0), 0);
        // 細長い帯は、上下に余白ができる。横は端まで。
        let band = thumbnail(&Picture::Band, [360.0, 14.0], side);
        assert_eq!(alpha(&band, 1, side / 2), 255);
        assert_eq!(alpha(&band, side / 2, 1), 0);
        assert_eq!(alpha(&band, side / 2, side - 2), 0);
    }

    #[test]
    fn 同じ絵かどうかを見分ける() {
        let data = Arc::new(vec![0u8; 4]);
        let a = Picture::Image { w: 1, h: 1, rgba: data.clone() };
        let b = Picture::Image { w: 1, h: 1, rgba: data };
        let c = Picture::Image { w: 1, h: 1, rgba: Arc::new(vec![0u8; 4]) };
        assert!(same_picture(&a, &b));
        assert!(!same_picture(&a, &c), "中身が同じでも、別に取り込んだ画像は別の素材");
        assert!(same_picture(&Picture::Logo, &Picture::Logo));
        assert!(!same_picture(&Picture::Logo, &Picture::Band));
    }
}
