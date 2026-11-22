//! 素材の欄。この案件の素材（取り込んだ画像）と、付属の見本。素材の差し替えも、ここ。
//!
//! 素材を、立体や展開図へドラッグして置く。落とした場所が紙の表か裏かで、置く面が決まる。
//! コレクション（フォルダの登録）は、ファイルのダイアログと設定の保存が要るので、まだ作っていない。

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, Rect, Sense, vec2};

use pakenoba_core::project;
use pakenoba_core::surface::Decal;
use pakenoba_core::template::P2;

use super::{App, Tool, file};
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
    /// 取り込んだ元のファイル（中身、取り込んだ日時、中身の印）。案件のファイルに、そのまま入れる。付属の見本にはない。
    pub stored: Option<pakenoba_core::project::Asset>,
}

/// 元に戻すために取っておく、この案件の素材 1 つ。絵と中身は共有しているので、取っておいても軽い。
#[derive(Clone)]
pub(super) struct Stash {
    name: String,
    picture: Picture,
    size: P2,
    stored: Option<project::Asset>,
}

/// 差し替える範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Replace {
    /// 素材そのもの（素材の欄での番号）。その素材を使っている柄が、すべて新しい絵になる。
    Asset(usize),
    /// 柄 1 つだけ（柄の番号）。
    Decal(usize),
}

/// 付属の見本。
pub(super) fn bundled() -> Vec<Asset> {
    vec![
        Asset { name: "ロゴ（見本）".to_owned(), picture: Picture::Logo, size: [46.0, 46.0], thumb: None, bundled: true, stored: None },
        Asset { name: "一周する帯（見本）".to_owned(), picture: Picture::Band, size: [120.0, 14.0], thumb: None, bundled: true, stored: None },
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
pub(super) fn same_picture(a: &Picture, b: &Picture) -> bool {
    match (a, b) {
        (Picture::Logo, Picture::Logo) | (Picture::Band, Picture::Band) => true,
        (Picture::Image { rgba: x, .. }, Picture::Image { rgba: y, .. }) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// 一覧の絵の、もとの大きさ（px）。大きく見るときにも使うので、多めに取る。
const THUMB_PX: usize = 256;

impl App {
    /// その素材を、箱で何か所に使っているか。
    pub(super) fn uses(&self, asset: usize) -> usize {
        let picture = &self.assets[asset].picture;
        self.decals.iter().filter(|decal| same_picture(&decal.picture, picture)).count()
    }

    /// 置くときの大きさ（mm）。帯は、箱を一周する長さにする。
    fn asset_size(&self, asset: &Asset) -> P2 {
        if matches!(asset.picture, Picture::Band) { [self.dims.front_and_around(&self.template).1, asset.size[1]] } else { asset.size }
    }

    /// 素材を、その場所に置いたときの柄。`at` は展開図上の位置、`back` が真なら紙の裏。
    pub(super) fn asset_decal(&self, asset: usize, panel: usize, at: P2, back: bool, angle: f32) -> Option<Placed> {
        let source = self.assets.get(asset)?;
        let mut decal = Decal { panel, at: [0.5, 0.5], size: self.asset_size(source), angle };
        decal.set_center(&self.template, at);
        Some(Placed { picture: source.picture.clone(), decal, back })
    }

    /// 柄を置いて、選ぶ。
    pub(super) fn place(&mut self, placed: Placed) {
        self.checkpoint();
        self.decals.push(placed);
        self.selected = Some(self.decals.len() - 1);
        self.tool = Tool::Select;
        self.art_dirty = true;
        self.back_dirty = true;
    }

    /// 左（デザイン）: 素材の欄。
    pub(super) fn assets_panel(&mut self, ui: &mut egui::Ui) {
        if !self.assets_open {
            return;
        }
        egui::Panel::left("assets").resizable(true).default_size(236.0).size_range(170.0..=520.0).show(ui, |ui| {
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
            ui.horizontal(|ui| {
                ui.label("絵の大きさ");
                ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(40.0, 200.0);
                ui.add(egui::Slider::new(&mut self.thumb_size, 48.0..=200.0).show_value(false));
            });
            // 下: 選んだ素材を、大きく見る。
            if let Some(selected) = self.asset_selected.filter(|&i| i < self.assets.len()) {
                egui::Panel::bottom("asset-detail").resizable(false).show(ui, |ui| self.asset_detail(ui, selected));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for bundled in [false, true] {
                    ui.add_space(6.0);
                    ui.label(if bundled { "付属の見本" } else { "この案件の素材" });
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
                ui.add_space(6.0);
                ui.label("コレクション");
                ui.small("フォルダを登録して、案件をまたいで使う素材を並べる機能は、これから作ります。");
            });
        });
    }

    /// 一覧に出す絵。初めて出すときに作る。
    fn thumb(&mut self, ctx: &egui::Context, index: usize) -> egui::TextureId {
        let asset = &mut self.assets[index];
        asset
            .thumb
            .get_or_insert_with(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied([THUMB_PX, THUMB_PX], &thumbnail(&asset.picture, asset.size, THUMB_PX));
                ctx.load_texture(format!("asset-{index}"), image, egui::TextureOptions::LINEAR)
            })
            .id()
    }

    /// 素材 1 つぶんのマス。クリックで選ぶ（置かない）。ドラッグで、箱や展開図へ運ぶ。
    fn asset_tile(&mut self, ui: &mut egui::Ui, index: usize) {
        let uses = self.uses(index);
        let texture = self.thumb(ui.ctx(), index);
        let side = self.thumb_size;
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
        let selected = self.asset_selected == Some(index);
        let visuals = ui.visuals();
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, if response.hovered() { visuals.widgets.hovered.bg_fill } else { visuals.extreme_bg_color });
        painter.image(texture, rect.shrink(4.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        if selected {
            painter.rect_stroke(rect, 4.0, egui::Stroke::new(2.0, visuals.selection.bg_fill), egui::StrokeKind::Inside);
        }
        if uses > 0 {
            // 箱で使っている素材の印。
            painter.circle_filled(rect.right_bottom() - vec2(8.0, 8.0), 4.0, visuals.selection.bg_fill);
        }
        if response.drag_started() {
            self.carry = Some(index);
            self.asset_selected = Some(index);
        }
        if response.clicked() {
            self.asset_selected = if selected { None } else { Some(index) };
        }
        if response.secondary_clicked() {
            self.asset_selected = Some(index);
        }
        let bundled = self.assets[index].bundled;
        let response = response.on_hover_text(&self.assets[index].name);
        response.context_menu(|ui| {
            if bundled {
                ui.label("付属の見本は、差し替えられません");
            } else if ui.button("素材を差し替える…").on_hover_text("この素材を使っている柄が、すべて新しい絵になります").clicked() {
                self.replace_request = Some(Replace::Asset(index));
                ui.close();
            }
        });
    }

    /// 選んだ素材を、大きく見る。名前、大きさ、使っている数。置くのは、ドラッグか、下のボタン。
    fn asset_detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let uses = self.uses(index);
        let texture = self.thumb(ui.ctx(), index);
        ui.add_space(6.0);
        let side = ui.available_width().min(260.0);
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::drag());
        let image = Rect::from_center_size(rect.center(), vec2(side, side));
        ui.painter().rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
        ui.painter().image(texture, image.shrink(6.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        // 大きい絵からも、箱へ運べる。
        if response.drag_started() {
            self.carry = Some(index);
        }
        let asset = &self.assets[index];
        let size = self.asset_size(asset);
        ui.label(&asset.name);
        if let Picture::Image { w, h, .. } = &asset.picture {
            ui.small(format!("画像: {w} × {h} px"));
        }
        ui.small(format!("置く大きさ: {:.0} × {:.0} mm", size[0], size[1]));
        if let Some(when) = asset.stored.as_ref().and_then(|s| file::show_time(&s.imported)) {
            ui.small(format!("取り込んだ日時: {when}"));
        }
        ui.small(if uses > 0 { format!("箱で {uses} か所に使っています") } else { "まだ箱に置いていません".to_owned() });
        ui.add_space(2.0);
        let picture = asset.picture.clone();
        let bundled = asset.bundled;
        ui.horizontal(|ui| {
            if ui.button("正面に置く").on_hover_text("箱や展開図へドラッグすると、落とした場所に置けます").clicked() {
                self.add_decal(picture, size);
                self.tool = Tool::Select;
            }
            if !bundled && ui.button("差し替える…").on_hover_text("この素材を使っている柄が、すべて新しい絵になります（右クリックでも）").clicked() {
                self.replace_request = Some(Replace::Asset(index));
            }
        });
        ui.add_space(4.0);
    }

    /// 運んでいる途中の素材を、カーソルのそばに出す（箱の上では、仮に置いた姿を見せるので、出さない）。
    pub(super) fn carry_ghost(&self, ui: &egui::Ui) {
        let (Some(asset), Some(pointer)) = (self.carry.and_then(|i| self.assets.get(i)), ui.input(|i| i.pointer.interact_pos())) else { return };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
        let (Some(texture), None) = (&asset.thumb, &self.preview) else { return };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("carried-asset")));
        let rect = Rect::from_center_size(pointer, vec2(64.0, 64.0));
        painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(200));
    }
    /// 元に戻すために取っておく、この案件の素材。
    pub(super) fn library(&self) -> Vec<Stash> {
        self.assets.iter().filter(|a| !a.bundled).map(|a| Stash { name: a.name.clone(), picture: a.picture.clone(), size: a.size, stored: a.stored.clone() }).collect()
    }

    /// 取っておいた素材に戻す。小さい絵は、同じ絵のものを使い回す。
    pub(super) fn restore_library(&mut self, library: Vec<Stash>) {
        let (mut assets, mine): (Vec<Asset>, Vec<Asset>) = std::mem::take(&mut self.assets).into_iter().partition(|a| a.bundled);
        for stash in library {
            let thumb = mine.iter().find(|a| same_picture(&a.picture, &stash.picture)).and_then(|a| a.thumb.clone());
            assets.push(Asset { name: stash.name, picture: stash.picture, size: stash.size, thumb, bundled: false, stored: stash.stored });
        }
        self.assets = assets;
        self.asset_selected = self.asset_selected.filter(|&i| i < self.assets.len());
        self.carry = None;
    }

    /// 柄の絵が、この案件のどの素材か（付属の見本や、素材の欄にない絵なら None）。
    pub(super) fn asset_of(&self, picture: &Picture) -> Option<usize> {
        self.assets.iter().position(|a| !a.bundled && same_picture(&a.picture, picture))
    }

    /// 選んでいる柄の、右クリックのメニュー。
    pub(super) fn decal_menu(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else {
            ui.close();
            return;
        };
        ui.set_min_width(240.0);
        ui.label(format!("{} {}", self.decals[index].picture.label(), index + 1));
        ui.separator();
        match self.asset_of(&self.decals[index].picture) {
            Some(asset) => {
                let uses = self.uses(asset);
                if ui.button(format!("素材を差し替える…（{uses} か所）")).on_hover_text("この素材を使っている柄が、すべて新しい絵になります").clicked() {
                    self.replace_request = Some(Replace::Asset(asset));
                    ui.close();
                }
                if ui.button("この柄だけ差し替える…").clicked() {
                    self.replace_request = Some(Replace::Decal(index));
                    ui.close();
                }
            }
            None => {
                if ui.button("この柄を画像に差し替える…").clicked() {
                    self.replace_request = Some(Replace::Decal(index));
                    ui.close();
                }
            }
        }
        ui.separator();
        if ui.button("消す").clicked() {
            self.delete_selected();
            ui.close();
        }
    }

    /// 差し替える画像を選んでもらう。
    fn ask_replacement(&mut self) -> Option<PathBuf> {
        if let Some(answer) = self.answer_replace.take() {
            return Some(answer);
        }
        rfd::FileDialog::new().set_title("差し替える画像を選ぶ").add_filter("画像（PNG）", &["png", "PNG"]).pick_file()
    }

    /// 頼まれた差し替えを行う。画像を選んでもらい、絵を入れ替える。元に戻せる。
    ///
    /// 場所、向き、表と裏はそのまま。幅もそのままで、高さを新しい絵の縦横の比に合わせる。
    pub(super) fn run_replace(&mut self) {
        let Some(target) = self.replace_request.take() else { return };
        // 範囲が、いまもあるか（メニューを開いてから、消されたかもしれない）。
        let valid = match target {
            Replace::Asset(index) => self.assets.get(index).is_some_and(|a| !a.bundled),
            Replace::Decal(index) => index < self.decals.len(),
        };
        if !valid {
            return;
        }
        let Some(path) = self.ask_replacement() else { return };
        self.replace_with(target, &path);
    }

    /// 画像のファイルで、差し替える。
    fn replace_with(&mut self, target: Replace, path: &std::path::Path) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return self.say(format!("差し替えませんでした。{} を開けませんでした: {e}", path.display())),
        };
        let picture = match super::decode_png(&bytes) {
            Ok(picture) => picture,
            Err(e) => return self.say(format!("差し替えませんでした。{e}")),
        };
        let Picture::Image { w, h, .. } = &picture else { return };
        let ratio = *h as f32 / *w as f32;
        let name = path.file_name().map_or_else(|| "画像".to_owned(), |n| n.to_string_lossy().into_owned());
        match target {
            Replace::Asset(index) => {
                let Some(asset) = self.assets.get(index).filter(|a| !a.bundled) else { return };
                let (old, old_name, width) = (asset.picture.clone(), asset.name.clone(), asset.size[0]);
                self.checkpoint();
                let mut count = 0;
                for placed in &mut self.decals {
                    if same_picture(&placed.picture, &old) {
                        placed.picture = picture.clone();
                        placed.decal.size[1] = placed.decal.size[0] * ratio;
                        count += 1;
                    }
                }
                let size = [width, width * ratio];
                let asset = &mut self.assets[index];
                asset.stored = Some(project::Asset::new(name.clone(), bytes, size, file::now_local()));
                (asset.name, asset.picture, asset.size, asset.thumb) = (name.clone(), picture, size, None);
                self.say(format!("素材「{old_name}」を「{name}」に差し替えました（{count} か所の柄）。Ctrl+Z で戻せます。"));
            }
            Replace::Decal(index) => {
                let Some(placed) = self.decals.get(index) else { return };
                let width = placed.decal.size[0];
                let stored = project::Asset::new(name.clone(), bytes, [width, width * ratio], file::now_local());
                self.checkpoint();
                // 同じ中身の素材がすでにあれば、それを使う。なければ、素材の欄に足す。
                let known = self.assets.iter().find(|a| a.stored.as_ref().is_some_and(|s| s.id() == stored.id())).map(|a| a.picture.clone());
                let picture = match known {
                    Some(picture) => picture,
                    None => {
                        self.assets.push(Asset { name: name.clone(), picture: picture.clone(), size: stored.size, thumb: None, bundled: false, stored: Some(stored) });
                        picture
                    }
                };
                let placed = &mut self.decals[index];
                placed.picture = picture;
                placed.decal.size[1] = width * ratio;
                self.say(format!("柄 {} を「{name}」に差し替えました。Ctrl+Z で戻せます。", index + 1));
            }
        }
        self.art_dirty = true;
        self.back_dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
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


    /// 単色の PNG のファイルの中身。
    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize)).unwrap();
        }
        out
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pakenoba-app-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 素材 1 つ（正方形の画像）を、柄として 2 か所に置き、見本のロゴも 1 つ置いた状態。
    fn app_with_asset(dir: &std::path::Path) -> App {
        let path = dir.join("ロゴ 旧版.png");
        std::fs::write(&path, png(4, 4, [200, 0, 0])).unwrap();
        let mut app = App::new(&Initial::default());
        app.add_png(&path);
        let (picture, size) = (app.decals[0].picture.clone(), app.decals[0].size);
        app.add_decal(picture, size);
        app.decals[1].decal.at = [0.2, 0.8];
        app.decals[1].decal.size = [30.0, 30.0];
        app.add_decal(Picture::Logo, [46.0, 46.0]);
        app
    }

    #[test]
    fn 素材を差し替えると_使っている柄がすべて変わり_元に戻せる() {
        let dir = scratch("replace-all");
        let mut app = app_with_asset(&dir);
        let asset = app.asset_of(&app.decals[0].picture).unwrap();
        let before = (app.decals.clone(), app.assets[asset].name.clone(), app.fingerprint());
        let at = [app.decals[0].decal.at, app.decals[1].decal.at];

        // 横長（2:1）の新しい版に差し替える。
        let new = dir.join("ロゴ 新版.png");
        std::fs::write(&new, png(8, 4, [0, 0, 200])).unwrap();
        app.replace_with(Replace::Asset(asset), &new);

        assert_eq!(app.assets[asset].name, "ロゴ 新版.png");
        assert_eq!(app.uses(asset), 2);
        for (i, width) in [(0, 50.0), (1, 30.0)] {
            let placed = &app.decals[i];
            assert!(same_picture(&placed.picture, &app.assets[asset].picture));
            // 場所と幅はそのまま。高さは新しい絵の比（2:1）に合わせる。
            assert_eq!(placed.decal.at, at[i]);
            assert_eq!(placed.decal.size, [width, width * 0.5]);
        }
        assert!(matches!(app.decals[2].picture, Picture::Logo));
        assert_eq!(app.assets[asset].size, [50.0, 25.0]);
        assert!(app.assets[asset].stored.as_ref().is_some_and(|s| s.name == "ロゴ 新版.png" && !s.imported.is_empty()));
        assert_ne!(app.fingerprint(), before.2);

        // 元に戻すと、素材も柄も、差し替える前に戻る。
        app.undo();
        assert_eq!(app.assets[asset].name, before.1);
        assert_eq!(app.fingerprint(), before.2);
        for (now, then) in app.decals.iter().zip(&before.0) {
            assert!(same_picture(&now.picture, &then.picture));
            assert_eq!(now.decal.size, then.decal.size);
        }
        // やり直せる。
        app.redo();
        assert_eq!(app.assets[asset].name, "ロゴ 新版.png");
        assert!(same_picture(&app.decals[1].picture, &app.assets[asset].picture));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 柄だけを差し替えると_ほかの柄はそのまま() {
        let dir = scratch("replace-one");
        let mut app = app_with_asset(&dir);
        let asset = app.asset_of(&app.decals[0].picture).unwrap();
        let count = app.assets.len();
        let new = dir.join("別の絵.png");
        std::fs::write(&new, png(2, 6, [0, 150, 0])).unwrap();
        app.replace_with(Replace::Decal(1), &new);
        // 素材が 1 つ増え、柄 2 だけがそれを使う。
        assert_eq!(app.assets.len(), count + 1);
        assert_eq!(app.uses(asset), 1);
        assert!(same_picture(&app.decals[1].picture, &app.assets[count].picture));
        assert_eq!(app.decals[1].decal.size, [30.0, 90.0]);
        assert_eq!(app.decals[0].decal.size, [50.0, 50.0]);

        // 同じ画像で、見本のロゴの柄も差し替えると、素材は増えずに同じものを使う。
        app.replace_with(Replace::Decal(2), &new);
        assert_eq!(app.assets.len(), count + 1);
        assert!(same_picture(&app.decals[2].picture, &app.assets[count].picture));

        // 2 回戻すと、増えた素材もなくなる。
        app.undo();
        app.undo();
        assert_eq!(app.assets.len(), count);
        assert!(matches!(app.decals[2].picture, Picture::Logo));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 画像でないファイルでは_差し替えない() {
        let dir = scratch("replace-bad");
        let mut app = app_with_asset(&dir);
        let before = app.fingerprint();
        let bad = dir.join("文書.png");
        std::fs::write(&bad, "PNG ではない").unwrap();
        app.replace_with(Replace::Asset(app.assets.len() - 1), &bad);
        app.replace_with(Replace::Decal(0), &dir.join("ないファイル.png"));
        assert_eq!(app.fingerprint(), before);
        assert!(!app.history.can_redo());
        // 付属の見本は、差し替えない。
        app.replace_with(Replace::Asset(0), &dir.join("ロゴ 旧版.png"));
        assert_eq!(app.fingerprint(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
