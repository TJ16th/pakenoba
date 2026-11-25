//! 中央の表示。立体、展開図の表、展開図の裏。
//!
//! どれを出すかは、上の帯の 3 つのボタンで決める。並べ方は、出ている表示の組み合わせから決まる。
//! どの表示でも柄を選べて、動かせる。選択は共通。

use pakenoba_i18n::t;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, pos2, vec2};
use eframe::egui_wgpu;

use pakenoba_core::math::{Mat4, V3, cross, look_at, mat4_mul, normalize, perspective, sub};
use pakenoba_core::surface::{Decal, panel_at, pick_side, ray_through};
use pakenoba_core::template::bounding_sphere;

use super::transform::{self, Transforming};
use super::{App, Shown, Tool, Workspace};
use crate::gpu::BoxRenderer;
use crate::paint::{Picture, Placed, spots_on};

/// 表示と表示の間のすき間。
const GAP: f32 = 2.0;
/// 立体と展開図を並べるときの、立体の幅の割合。
const SOLID_SHARE: f32 = 0.55;
/// 定規の太さ（上の定規の高さと、左の定規の幅）。
const RULER_TOP: f32 = 18.0;
const RULER_LEFT: f32 = 30.0;
/// 展開図の表示の地の色。立体の表示の地と同じにする。
const BACKDROP: Color32 = super::theme::CANVAS;

/// 出ている表示の組み合わせから、それぞれの場所を決める。返すのは、立体、表、裏の順。
///
/// - 1 つ: 中央いっぱい
/// - 立体と、表か裏のどちらか: 左に立体、右に展開図
/// - 表と裏: 上下に並べる（展開図は横長のことが多いので、縦に積むほうが大きく見える）
/// - 3 つ: 左に立体を大きく、右に表と裏を上下に
pub(super) fn layout(rect: Rect, shown: Shown) -> [Option<Rect>; 3] {
    let flats = shown.front as usize + shown.back as usize;
    let (solid, rest) = match (shown.solid, flats) {
        (true, 0) => (Some(rect), None),
        (false, _) => (None, Some(rect)),
        (true, _) => {
            let split = rect.left() + rect.width() * SOLID_SHARE;
            (Some(Rect::from_min_max(rect.min, pos2(split - GAP * 0.5, rect.bottom()))), Some(Rect::from_min_max(pos2(split + GAP * 0.5, rect.top()), rect.max)))
        }
    };
    let (front, back) = match (rest, shown.front, shown.back) {
        (Some(rest), true, true) => {
            let middle = rest.center().y;
            (Some(Rect::from_min_max(rest.min, pos2(rest.right(), middle - GAP * 0.5))), Some(Rect::from_min_max(pos2(rest.left(), middle + GAP * 0.5), rest.max)))
        }
        (Some(rest), true, false) => (Some(rest), None),
        (Some(rest), false, true) => (None, Some(rest)),
        _ => (None, None),
    };
    [solid, front, back]
}

/// 画像を、画面に出せる形にする。大きすぎるときは、間引いて収める。
pub(super) fn texture(ctx: &egui::Context, name: &str, w: usize, h: usize, rgba: &[u8]) -> egui::TextureHandle {
    let limit = ctx.input(|i| i.max_texture_side).max(256);
    let step = w.max(h).div_ceil(limit).max(1);
    let image = if step == 1 {
        egui::ColorImage::from_rgba_unmultiplied([w, h], rgba)
    } else {
        let (sw, sh) = (w / step, h / step);
        let mut small = Vec::with_capacity(sw * sh * 4);
        for y in 0..sh {
            for x in 0..sw {
                let o = (y * step * w + x * step) * 4;
                small.extend_from_slice(&rgba[o..o + 4]);
            }
        }
        egui::ColorImage::from_rgba_unmultiplied([sw, sh], &small)
    };
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

/// 見る向きの決まった視点（名前、横の角度、縦の角度。ラジアン）。正面は箱の手前（+Z）の面。
/// 「斜め」は、正面、右、上の 3 面が同じように見える等角の向き。
pub(super) const VIEW_PRESETS: [(&str, f32, f32); 7] = [
    ("Isometric view", std::f32::consts::FRAC_PI_4, 0.615_479_7),
    ("Front view", 0.0, 0.0),
    ("Back view", std::f32::consts::PI, 0.0),
    ("Left view", -std::f32::consts::FRAC_PI_2, 0.0),
    ("Right view", std::f32::consts::FRAC_PI_2, 0.0),
    ("Top view", 0.0, std::f32::consts::FRAC_PI_2),
    ("Bottom view", 0.0, -std::f32::consts::FRAC_PI_2),
];

impl App {
    /// 立体の表示の右上に、見る向きを 1 回で選ぶボタンを並べる。押すと、その向きへなめらかに回る。
    /// ボタンは、組ごとに半透明の座布団に乗せる（箱の柄の上でも読めて、ボタンだと分かるように）。
    /// 1 行目は視点、2 行目は右から「表示の切り替え（照明、平行投影）」「回す（90 度ずつ、そろえる）」。
    fn view_presets(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let area = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top() + 6.0), pos2(rect.right() - 8.0, rect.top() + 32.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            cushion(ui, |ui| {
                for (label, yaw, pitch) in VIEW_PRESETS.iter().rev() {
                    if cushion_button(ui, pakenoba_i18n::tr(label), false).clicked() {
                        self.look_from(*yaw, *pitch);
                    }
                }
            });
        });
        let area = area.translate(vec2(0.0, 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cushion(ui, |ui| {
                if cushion_button(ui, t!("Parallel"), self.ortho).on_hover_text(t!("Parallel projection: no perspective, so parallel edges stay parallel")).clicked() {
                    self.ortho = !self.ortho;
                }
                ui.menu_button(t!("Lighting"), |ui| self.lighting_menu(ui)).response.on_hover_text(t!("Choose the environment that lights the box, and turn it"));
            });
            cushion(ui, |ui| {
                if cushion_button(ui, t!("Snap"), false).on_hover_text(t!("Turn to the nearest straight-on view (front, side, top...)")).clicked() {
                    let (yaw, pitch) = self.view_to.unwrap_or((self.yaw, self.pitch));
                    let quarter = std::f32::consts::FRAC_PI_2;
                    // 上下も 90 度刻みにそろえる（真上を越えて逆さのときは、逆さのまま）。
                    self.look_from((yaw / quarter).round() * quarter, (wrap_angle(pitch) / quarter).round() * quarter);
                }
                let (yaw, pitch) = self.view_to.unwrap_or((self.yaw, self.pitch));
                if cushion_button(ui, t!("Down 90°"), false).on_hover_text(t!("Turn the box 90 degrees toward you, over the top")).clicked() {
                    self.look_from(yaw, wrap_angle(pitch + std::f32::consts::FRAC_PI_2));
                }
                if cushion_button(ui, t!("Up 90°"), false).on_hover_text(t!("Turn the box 90 degrees away from you, over the bottom")).clicked() {
                    self.look_from(yaw, wrap_angle(pitch - std::f32::consts::FRAC_PI_2));
                }
                if cushion_button(ui, t!("Right 90°"), false).on_hover_text(t!("Turn the box 90 degrees to the right")).clicked() {
                    self.look_from(yaw - std::f32::consts::FRAC_PI_2, pitch);
                }
                if cushion_button(ui, t!("Left 90°"), false).on_hover_text(t!("Turn the box 90 degrees to the left")).clicked() {
                    self.look_from(yaw + std::f32::consts::FRAC_PI_2, pitch);
                }
            });
        });
    }

    /// その向きへ、なめらかに回る（展開図へ戻る前の視点の覚えは消す）。
    pub(super) fn look_from(&mut self, yaw: f32, pitch: f32) {
        self.view_before_flat = None;
        self.view_to = Some((yaw, pitch));
    }
}

/// 軸の側から見る向き（横の角度、縦の角度）。`sign` が正なら軸のプラスの側から。
fn axis_view(axis: usize, sign: f32) -> (f32, f32) {
    match (axis, sign > 0.0) {
        (0, true) => (std::f32::consts::FRAC_PI_2, 0.0),
        (0, false) => (-std::f32::consts::FRAC_PI_2, 0.0),
        (1, true) => (0.0, std::f32::consts::FRAC_PI_2),
        (1, false) => (0.0, -std::f32::consts::FRAC_PI_2),
        (_, true) => (0.0, 0.0),
        (_, false) => (std::f32::consts::PI, 0.0),
    }
}

/// 視点の向きと、画面の上の向き。`pitch` に制限はない（真上を越えて回すと、箱は上下が逆さに見える）。
pub(super) fn orbit(yaw: f32, pitch: f32) -> (V3, V3) {
    let dir = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
    let up = [-pitch.sin() * yaw.sin(), pitch.cos(), -pitch.sin() * yaw.cos()];
    (dir, up)
}

/// 角度を -π〜π にする。
pub(super) fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// マウスで回す。上下には制限なく回せる。逆さのときは、左右の向きも逆にする（手の動きと箱の動きをそろえる）。
pub(super) fn drag_orbit(yaw: &mut f32, pitch: &mut f32, delta: egui::Vec2) {
    let flip = if pitch.cos() < 0.0 { -1.0 } else { 1.0 };
    *yaw = wrap_angle(*yaw - delta.x * 0.01 * flip);
    *pitch = wrap_angle(*pitch + delta.y * 0.01);
}

/// 2 つの角度の差（0〜π）。
fn angle_gap(a: f32, b: f32) -> f32 {
    ((a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs()
}

/// 立体の表示の投影。透視か平行。平行のときは、箱の中心での見た目の大きさが透視と同じになるようにする。
pub(super) fn projection(ortho: bool, fov: f32, aspect: f32, dist: f32, radius: f32) -> Mat4 {
    let (near, far) = ((dist - radius * 1.5).max(dist * 0.02), dist + radius * 2.0);
    if !ortho {
        return perspective(fov, aspect, near, far);
    }
    let h = dist * (fov * 0.5).tan();
    let w = h * aspect;
    [[1.0 / w, 0.0, 0.0, 0.0], [0.0, 1.0 / h, 0.0, 0.0], [0.0, 0.0, -1.0 / (far - near), 0.0], [0.0, 0.0, -near / (far - near), 1.0]]
}

/// 立体の点を、表示 `rect` の上の点へ。視点の後ろなら `None`。
/// カーソルの下の柄の枠の色（パケノバの水色）。
const HOVER_COLOR: Color32 = Color32::from_rgb(38, 178, 170);

impl App {
    /// 枠を出す柄: この表示でカーソルの下にある柄、なければ、ほかの表示で見つけた柄。選んでいる柄や、ボタンを押している間は出さない。
    /// `back` は展開図の表示の面（立体の表示では None）。
    fn hover_shown(&self, here: Option<usize>, back: Option<bool>) -> Option<usize> {
        let index = here.or(self.hover_decal)?;
        let placed = self.decals.get(index)?;
        let pressing = self.grab.is_some() || self.transforming.is_some() || self.carry.is_some();
        (!pressing && !self.is_selected(index) && back.is_none_or(|b| b == placed.back)).then_some(index)
    }

    /// 柄 `index` の縁を、面ごとに分けて線で描く。`to_screen(パネル, 展開図の点)` で画面の点へ。
    fn paint_outline(&self, painter: &egui::Painter, index: usize, to_screen: impl Fn(usize, [f32; 2]) -> Option<Pos2>) {
        // 見えている部分の縁を描く。
        let (decal, _) = super::visible::trim(&self.decals[index].decal, &self.template, super::visible::of(&self.decals[index]));
        let decal = &decal;
        let half = [decal.size[0] * 0.5, decal.size[1] * 0.5];
        let corners = [[-half[0], -half[1]], [half[0], -half[1]], [half[0], half[1]], [-half[0], half[1]]];
        const STEPS: usize = 24;
        for piece in self.surface.layout(&self.template, decal) {
            let outline = &self.template.panels[piece.panel].outline;
            for e in 0..4 {
                let (a, b) = (corners[e], corners[(e + 1) % 4]);
                // 縁を細かく分け、そのパネルの中にある所だけを結ぶ（面をまたぐ柄は、面ごとに描く）。
                let mut run: Vec<Pos2> = Vec::new();
                for k in 0..=STEPS {
                    let t = k as f32 / STEPS as f32;
                    let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    let p = piece.place.point(q);
                    match pakenoba_core::template::inside(outline, p).then(|| to_screen(piece.panel, p)).flatten() {
                        Some(s) => run.push(s),
                        None => {
                            if run.len() > 1 {
                                painter.add(egui::Shape::line(std::mem::take(&mut run), egui::Stroke::new(2.0, HOVER_COLOR)));
                            }
                            run.clear();
                        }
                    }
                }
                if run.len() > 1 {
                    painter.add(egui::Shape::line(run, egui::Stroke::new(2.0, HOVER_COLOR)));
                }
            }
        }
    }
}

/// 吸い付きの補助線の色（イラストレーターのスマートガイドに近い、目立つ桃色）。
const SNAP_COLOR: Color32 = Color32::from_rgb(255, 64, 200);
/// 吸い付く距離（画面の点）。
const SNAP_REACH: f32 = 8.0;

/// 柄を、置かれているパネルの中央と端に吸い付かせる。`ppm` は、画面での 1 mm の大きさ（点）。
/// Ctrl（Mac では Command）を押している間は吸い付かない。吸い付いた先の補助線を `guides` に足す。
/// 吸い付くのは、柄の見えている部分（`area`）の中央と端。
fn snap(guides: &mut Vec<(usize, [f32; 2], [f32; 2])>, template: &pakenoba_core::template::Template, decal: Decal, area: [f32; 4], ppm: f32, ui: &egui::Ui) -> Decal {
    if ui.input(|i| i.modifiers.command) || !ppm.is_finite() || ppm <= 0.0 {
        return decal;
    }
    let (seen, _) = super::visible::trim(&decal, template, area);
    let found = pakenoba_core::surface::snap_to_panel(template, &seen, (SNAP_REACH / ppm).min(10.0));
    guides.extend(found.guides);
    // 見えている部分が動いたぶんだけ、柄を動かす。
    let (from, to) = (seen.center(template), found.decal.center(template));
    let mut out = decal;
    out.panel = found.decal.panel;
    let c = decal.center(template);
    out.set_center(template, [c[0] + to[0] - from[0], c[1] + to[1] - from[1]]);
    out
}

/// 柄を拾う範囲（見えている部分）と、その中心の柄の座標での位置。`spots_on` と同じ並び。
fn visible_spots(decals: &[Placed], template: &pakenoba_core::template::Template, back: bool) -> (Vec<usize>, Vec<Decal>, Vec<[f32; 2]>) {
    let (ids, places) = spots_on(decals, back);
    let (trimmed, offsets) = ids.iter().zip(&places).map(|(&i, d)| super::visible::trim(d, template, super::visible::of(&decals[i]))).unzip();
    (ids, trimmed, offsets)
}

/// 柄の置かれた面の上での、画面の 1 mm の大きさ（点）。
fn px_per_mm(mvp: &Mat4, rect: Rect, frame: &pakenoba_core::math::Affine) -> f32 {
    match (project(mvp, rect, frame.point([0.0, 0.0, 0.0])), project(mvp, rect, frame.point([10.0, 0.0, 0.0])), project(mvp, rect, frame.point([0.0, 10.0, 0.0]))) {
        (Some(o), Some(x), Some(y)) => ((x - o).length().max((y - o).length())) / 10.0,
        _ => 0.0,
    }
}

fn project(mvp: &Mat4, rect: Rect, p: V3) -> Option<Pos2> {
    let v = [p[0], p[1], p[2], 1.0];
    let c: Vec<f32> = (0..4).map(|r| (0..4).map(|k| mvp[k][r] * v[k]).sum()).collect();
    (c[3] > 1e-6).then(|| pos2(rect.left() + (c[0] / c[3] + 1.0) * 0.5 * rect.width(), rect.top() + (1.0 - c[1] / c[3]) * 0.5 * rect.height()))
}

/// 定規の目盛りの間隔（mm）。細かい目盛りと、数字を添える目盛り。
fn ruler_steps(px_per_mm: f32) -> (f32, f32) {
    let minor = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0].into_iter().find(|step| step * px_per_mm >= 5.0).unwrap_or(200.0);
    let major = [10.0, 50.0, 100.0, 500.0, 1000.0].into_iter().find(|step| *step >= minor * 5.0 && step * px_per_mm >= 44.0).unwrap_or(2000.0);
    (minor, major)
}

impl App {
    /// 中央。出ている表示を並べる。
    pub(super) fn views(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        // 表、裏を出せるのは、デザインの工程だけ。ほかの工程は立体だけ。
        let shown = if self.workspace == Workspace::Design { self.shown } else { Shown::SOLID_ONLY };
        let rs: Option<egui_wgpu::RenderState> = frame.wgpu_render_state().cloned();
        self.render_state.clone_from(&rs);
        self.refresh_art(ui.ctx(), rs.as_ref(), shown);
        ui.painter().rect_filled(rect, 0.0, ui.visuals().window_stroke.color);
        self.hover = None;
        let [solid, front, back] = layout(rect, shown);
        let several = shown.count() > 1;
        // 素材の欄から素材を運んでいる間は、カーソルの場所を各表示へ伝える。
        // 各表示は、「いま放したら置かれる柄」を返す。それを箱の上に仮に描いて見せ、放したら本当に置く。
        let released = ui.input(|i| i.pointer.any_released());
        let carry = self.carry.zip(ui.input(|i| i.pointer.interact_pos()));
        // 運んでいる間に R を押すと、置く向きを 90 度ずつ回す。
        if self.carry.is_some() && ui.input(|i| i.key_pressed(egui::Key::R)) {
            self.place_turn = (self.place_turn + 1) % 4;
            ui.ctx().request_repaint();
        }
        let mut candidate: Option<Placed> = None;
        // 吸い付きの補助線は、ひとつ前の画面で見つけたものを出す。消えたあとにも一度描き直す。
        let had_guides = !self.snap_shown.is_empty();
        self.snap_shown = std::mem::take(&mut self.snap_guides);
        if had_guides || !self.snap_shown.is_empty() {
            ui.ctx().request_repaint();
        }
        if let Some(rect) = solid {
            // 開け閉めと組み立ては、立体の表示に付く操作なので、立体の表示のすぐ下に置く。
            let rows = self.motion_rows(rect.width());
            let split = rect.bottom() - (10.0 + 28.0 * rows as f32);
            let (view, strip) = (Rect::from_min_max(rect.min, pos2(rect.right(), split)), Rect::from_min_max(pos2(rect.left(), split), rect.max));
            if self.workspace == Workspace::Output && self.output_kind == super::peek::PEEK_OUTPUT {
                self.peek_view(ui, view, rs.as_ref());
            } else {
                candidate = self.solid_view(ui, view, rs.as_ref(), carry.filter(|(_, p)| view.contains(*p)));
                if self.workspace == Workspace::Output {
                    self.shot_frame(ui, view);
                    self.movie_preview_overlay(ui, view);
                }
            }
            ui.painter().rect_filled(strip, 0.0, ui.visuals().panel_fill);
            ui.scope_builder(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 5.0))), |ui| self.motion_controls(ui, rows));
            if several {
                tag(ui, rect.min + vec2(8.0, 6.0), t!("3D"));
            }
        }
        if let Some(rect) = front {
            candidate = candidate.or(self.flat_view(ui, rect, false, carry.filter(|(_, p)| rect.contains(*p))));
        }
        if let Some(rect) = back {
            candidate = candidate.or(self.flat_view(ui, rect, true, carry.filter(|(_, p)| rect.contains(*p))));
        }
        if self.carry.is_some() {
            if released {
                if let Some(placed) = candidate.take() {
                    self.place(placed);
                }
                self.carry = None;
            }
            // 仮に置いた姿が変わったら、絵を作り直す。
            let same = match (&self.preview, &candidate) {
                (Some(a), Some(b)) => a.decal == b.decal && a.back == b.back,
                (None, None) => true,
                _ => false,
            };
            if !same {
                self.preview = candidate;
                self.art_dirty = true;
                self.back_dirty = true;
                ui.ctx().request_repaint();
            }
        }
        self.carry_ghost(ui);
    }

    /// 立体。
    /// 返すのは、素材を運んでいるとき、いま放したら置かれる柄。
    fn solid_view(&mut self, ui: &mut egui::Ui, rect: Rect, rs: Option<&egui_wgpu::RenderState>, carry: Option<(usize, Pos2)>) -> Option<Placed> {
        let response = ui.interact(rect, ui.scope_id().with("solid-view"), Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(rect);
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.3, 6.0);
            }
        }
        let Some(rs) = rs else {
            painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            painter.text(rect.center(), Align2::CENTER_CENTER, t!("The box cannot be shown because wgpu is not available."), FontId::proportional(13.0), Color32::LIGHT_RED);
            return None;
        };
        if self.backend.is_empty() {
            let info = rs.adapter.get_info();
            self.backend = format!("{:?} / {}", info.backend, info.name);
        }
        let mesh = self.template.build_mesh_at(self.fold, [1.0 - self.open[0], 1.0 - self.open[1]]);
        self.triangles = mesh.len() / 3;
        let (center, radius) = bounding_sphere(&mesh);

        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round().max(2.0) as u32, (rect.height() * ppp).round().max(2.0) as u32];
        let aspect = size[0] as f32 / size[1] as f32;
        self.solid_aspect = aspect;
        let fov = 30f32.to_radians();
        // 縦長の画面でも箱がはみ出さないように、狭いほうに合わせて離れる。
        let dist = radius / (fov * 0.5).sin() * 1.1 / self.zoom / aspect.min(1.0);
        let (dir, up) = orbit(self.yaw, self.pitch);
        let eye = [center[0] + dir[0] * dist, center[1] + dir[1] * dist, center[2] + dir[2] * dist];
        let view = look_at(eye, center, up);

        // カーソルの下にある、立体の上の点（パネルと、展開図上の位置）。
        let forward = normalize(sub(center, eye));
        let side = normalize(cross(forward, up));
        let upward = cross(side, forward);
        // 型紙は写しを使う（つかんだ柄といっしょに選んだ柄を動かすときに、App を書き換えるため）。
        let shared = std::rc::Rc::clone(&self.template);
        let template = &*shared;
        // 画面の点を通る光線（始まりと向き）。平行投影では、向きはどれも同じで、始まりがずれる。
        let ortho = self.ortho;
        let half_h = dist * (fov * 0.5).tan();
        let ray = |pos: Pos2| -> (V3, V3) {
            let (x, y) = ((pos.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (pos.y - rect.top()) / rect.height() * 2.0);
            if ortho {
                let (dx, dy) = (x * half_h * aspect, y * half_h);
                ([eye[0] + side[0] * dx + upward[0] * dy, eye[1] + side[1] * dx + upward[1] * dy, eye[2] + side[2] * dx + upward[2] * dy], forward)
            } else {
                (eye, ray_through(forward, side, upward, fov, aspect, x, y))
            }
        };
        let under = |pos: Pos2| {
            let (origin, dir) = ray(pos);
            pick_side(template, &mesh, origin, dir)
        };
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let hit = pointer.and_then(under);
        if response.hovered() || response.dragged() {
            self.hover = hit.map(|(panel, _, _)| panel);
        }
        // 紙の表（箱の外側）に当たったら表の柄、裏（内側）に当たったら裏の柄を探す。
        // 手のひらの道具では、柄をつかまない（どこをドラッグしても箱が回る）。
        let hand = self.tool == Tool::Hand;
        // 拾う範囲は、柄の見えている部分（透明な余白は拾わない）。つかんだ位置は、柄の座標に直す。
        let sides = [visible_spots(&self.decals, template, false), visible_spots(&self.decals, template, true)];
        let surface = &self.surface;
        let decal_at = |(panel, at, back): (usize, [f32; 2], bool)| {
            let (ids, places, offsets) = &sides[back as usize];
            surface.decal_at(template, places, panel, at).filter(|_| !hand).map(|(i, g)| (ids[i], [g[0] + offsets[i][0], g[1] + offsets[i][1]]))
        };
        let decal_under = hit.and_then(decal_at);
        // 選んでいる柄の、大きさと回転のつまみ。柄の置かれた面を広げた平面の上で、展開図の座標のまま計算する。
        let mvp = mat4_mul(&projection(ortho, fov, aspect, dist, radius), &view);
        let placements = template.transforms_at(self.fold, [1.0 - self.open[0], 1.0 - self.open[1]]);
        let world = template.world_at(self.fold);
        let frame_of = |d: &Decal| world.then_after(&placements[d.panel].at(d.center(template)));

        let quad_of = |placed: &Placed| {
            let d = &placed.decal;
            let frame = frame_of(d);
            let pts: Option<Vec<Pos2>> = transform::corners(d, template, super::visible::of(placed)).iter().map(|c| project(&mvp, rect, frame.point([c[0], c[1], 0.0]))).collect();
            pts.map(|v| [v[0], v[1], v[2], v[3]])
        };
        let knobs = self.knob_target(None).and_then(|i| quad_of(&self.decals[i]).map(|q| (i, q)));
        let handle_under = |p: Pos2| knobs.and_then(|(i, quad)| transform::handle_at(quad, p).map(|h| (i, h, quad)));
        let press = ui.input(|i| i.pointer.press_origin());
        // マスクを筆で描いている間は、柄をつかまず、箱も回さない。
        let brushing = self.brush_target().is_some();
        if response.drag_started() && !brushing && let Some((index, handle, _)) = press.and_then(handle_under) {
            let start = self.decals[index].decal;
            if let Some(pressed) = press.and_then(|p| { let (o, d) = ray(p); transform::ray_to_sheet(&frame_of(&start), o, d) }) {
                self.history.record(self.snapshot());
                self.transforming = Some(Transforming { index, handle, start, pressed, area: super::visible::of(&self.decals[index]) });
            }
        }
        if response.drag_started() && self.transforming.is_none() && !brushing {
            // 押した場所に柄があれば、柄をつかむ。なければ、箱を回す。
            let pressed = press.and_then(under);
            self.grab = pressed.and_then(decal_at);
            if let Some((index, _)) = self.grab {
                // 動かす前の場所へ戻せるように、覚えておく。
                self.history.record(self.snapshot());
                // 選んでいる柄をつかんだら、選んだものをまとめて動かす。選んでいない柄なら、その柄だけを選ぶ。
                if self.is_selected(index) {
                    if let Some(old) = self.selected.filter(|&old| old != index) {
                        self.extra.retain(|&i| i != index);
                        self.extra.push(old);
                    }
                    self.selected = Some(index);
                } else {
                    self.select_only(Some(index));
                }
                self.art_dirty = true;
            }
        }
        if let (true, Some(t), Some(p)) = (response.dragged(), self.transforming, pointer) {
            if let Some(at) = { let (o, d) = ray(p); transform::ray_to_sheet(&frame_of(&t.start), o, d) } {
                let keys = ui.input(|i| i.modifiers);
                self.decals[t.index].decal = transform::apply(&t, template, at, keys.shift == self.ratio_lock, keys.alt, keys.shift);
                self.art_dirty = true;
                self.back_dirty = true;
            }
        } else if response.dragged() && !brushing {
            match (self.grab, hit) {
                (Some((index, grab)), Some((panel, at, back))) => {
                    // 柄は、置かれている側の面の上だけを動く（表の柄は外側、裏の柄は内側）。
                    let area = self.decals.get(index).map_or(super::visible::FULL, super::visible::of);
                    if let Some(decal) = self.decals.get_mut(index).filter(|decal| decal.back == back) {
                        // 立体の上では、向きを面ごとの「上」にそろえて保つ（同じ面に戻れば、同じ向きに戻る）。
                        let before = decal.center(template);
                        self.surface.drag_on_solid(template, decal, back, panel, at, grab);
                        let ppm = px_per_mm(&mvp, rect, &frame_of(&decal.decal));
                        decal.decal = snap(&mut self.snap_guides, template, decal.decal, area, ppm, ui);
                        self.move_along(index, before);
                        self.art_dirty = true;
                    }
                }
                (Some(_), None) => {}
                (None, _) => {
                    // 自分で回したら、視点の自動の動きは止める。
                    self.view_to = None;
                    drag_orbit(&mut self.yaw, &mut self.pitch, response.drag_delta());
                }
            }
        }
        if response.drag_stopped() {
            if self.grab.is_some() {
                self.log_transform();
            }
            self.grab = None;
            self.transforming = None;
        }
        if brushing {
            if (response.drag_started() || response.clicked()) && let Some((panel, at, back)) = press.and_then(under) {
                self.brush_at(panel, at, back, true);
            } else if response.dragged() && let Some((panel, at, back)) = hit {
                self.brush_at(panel, at, back, false);
            }
            if response.drag_stopped() {
                self.brush_end();
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
        let on_handle = pointer.and_then(handle_under);
        // 文字の道具では、文字の柄を押せば選び、ほかを押せば、そこに文字を置く。
        let text_click = response.clicked() && self.tool == Tool::Text && !decal_under.is_some_and(|(index, _)| matches!(self.decals[index].picture, Picture::Text(_)));
        // 図形の道具では、図形を押せば選び、ほかを押せば、そこに図形を置く。
        let shape_click = response.clicked() && self.tool == Tool::Shape && !decal_under.is_some_and(|(index, _)| matches!(self.decals[index].picture, Picture::Shape(_)));
        if response.clicked() && on_handle.is_some() {
            // つまみを押しただけなら、選び直さない。
        } else if text_click && let Some((panel, at, back)) = hit {
            self.add_text_at(panel, at, back);
        } else if shape_click && let Some((panel, at, back)) = hit {
            self.add_shape_at(panel, at, back);
        } else if response.clicked() && self.workspace == Workspace::Shape && self.dims.sides == super::IMPORTED {
            // 型の画面で、読み込んだ型なら、押した面の折り線を選ぶ。
            self.pick_fold(hit.map(|(panel, _, _)| panel));
        } else if response.clicked() && self.tool != Tool::Hand && !brushing {
            let shift = ui.input(|i| i.modifiers.shift);
            self.select_click(decal_under.map(|(index, _)| index), shift);
        }
        // 右クリックは、選んでいる柄の上なら選んだものを保つ（まとめて消す、など）。
        if response.secondary_clicked() && self.tool != Tool::Hand && !decal_under.is_some_and(|(index, _)| self.is_selected(index)) {
            self.select_only(decal_under.map(|(index, _)| index));
        }
        // カーソルの下の柄は、ほかの表示でも光らせる。
        // 箱を回したり展開図をずらしたりしている間（ボタンを押している間）は光らせない（目障りで、描き直しも増えるので）。
        let pressing = ui.input(|i| i.pointer.any_down());
        if response.hovered() && self.grab.is_none() && !pressing && !response.dragged() {
            self.hover_now = self.hover_now.or(decal_under.map(|(index, _)| index));
        }
        if let Some(t) = self.transforming.filter(|_| response.dragged()) {
            ui.ctx().set_cursor_icon(knobs.map_or(egui::CursorIcon::Grabbing, |(_, quad)| transform::cursor(quad, t.handle)));
        } else if self.grab.is_some() && response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if let Some((_, handle, quad)) = on_handle.filter(|_| response.hovered()) {
            ui.ctx().set_cursor_icon(transform::cursor(quad, handle));
        } else if self.tool == Tool::Text && hit.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if self.tool == Tool::Shape && hit.is_some() && response.hovered() && decal_under.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if decal_under.is_some() && response.hovered() && !pressing {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // 運んでいる素材を、いま放したら置かれる柄。外側の面なら表、内側の面なら裏。
        // 向きは、画面で見て上が上になるようにする（面の辺にそろえて、90 度刻み）。
        let candidate = carry.and_then(|(asset, pos)| {
            let (panel, at, back) = under(pos)?;
            // 画面で少し上（だめなら下）の点が、展開図ではどちらの向きかを調べる。
            let upward = [(-6.0f32, 1.0f32), (6.0, -1.0)].into_iter().find_map(|(dy, sign)| {
                let (p, a, b) = under(pos + vec2(0.0, dy))?;
                let d = [(a[0] - at[0]) * sign, (a[1] - at[1]) * sign];
                (p == panel && b == back && d[0].hypot(d[1]) > 1e-3).then_some(d)
            });
            let quarter = std::f32::consts::FRAC_PI_2;
            let angle = upward.map_or(0.0, |d| ((-d[0]).atan2(d[1]) / quarter).round() * quarter);
            self.asset_decal(asset, panel, at, back, angle)
        });
        let candidate = candidate.map(|mut placed| {
            let ppm = px_per_mm(&mvp, rect, &frame_of(&placed.decal));
            placed.decal = snap(&mut self.snap_guides, template, placed.decal, super::visible::of(&placed), ppm, ui);
            placed
        });

        let proj = projection(ortho, fov, aspect, dist, radius);
        let look = self.look(self.yaw);

        // 抜き穴を、絵柄の座標（0〜1）に直して渡す。
        let (sheet_w, sheet_h) = (self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]);
        let mut holes = [[0.0f32; 4]; 2];
        for (slot, h) in holes.iter_mut().zip(&self.template.holes) {
            *slot = [(h[0] - self.template.min[0]) / sheet_w, (self.template.max[1] - h[1]) / sheet_h, h[2] / sheet_w, h[3] / sheet_h];
        }
        self.sync_environment(ui.ctx(), rs);
        let renderer = self.renderer.get_or_insert_with(|| BoxRenderer::new(rs));
        if let Some(id) = renderer.render(rs, size, &mesh, mat4_mul(&proj, &view), &look, holes) {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(id, rect, uv, Color32::WHITE);
        }
        // 大きさと回転のつまみ（動かしたあとの場所で）。
        if let Some(quad) = self.knob_target(None).and_then(|i| quad_of(&self.decals[i])) {
            transform::paint(&painter, quad, ui.visuals().selection.bg_fill);
        }
        // カーソルの下の柄の枠（柄の絵は描き直さずに、線で重ねる）。
        if let Some(index) = self.hover_shown(decal_under.map(|(i, _)| i), None) {
            let to_screen = |panel: usize, p: [f32; 2]| project(&mvp, rect, world.then_after(&placements[panel].at(p)).point([p[0], p[1], 0.0]));
            self.paint_outline(&painter, index, to_screen);
        }
        // 選んだ折り線と、それで動くパネル（読み込んだ型の、型の画面）。
        if self.workspace == Workspace::Shape
            && let Some(panel) = self.fold_pick.and_then(|p| template.panels.get(p))
            && let Some(hinge) = &panel.hinge
        {
            let index = self.fold_pick.unwrap_or(0);
            let to_screen = |i: usize, p: [f32; 2]| project(&mvp, rect, world.then_after(&placements[i].at(p)).point([p[0], p[1], 0.0]));
            let outline: Vec<Pos2> = panel.outline.iter().filter_map(|&p| to_screen(index, p)).collect();
            if outline.len() == panel.outline.len() {
                painter.add(egui::Shape::closed_line(outline, egui::Stroke::new(1.5, super::views::HOVER_COLOR)));
            }
            if let (Some(a), Some(b)) = (to_screen(hinge.parent, hinge.a), to_screen(hinge.parent, hinge.b)) {
                painter.line_segment([a, b], egui::Stroke::new(3.0, Color32::from_rgb(255, 170, 40)));
            }
        }
        // 吸い付いた先の補助線。丸めた面にも沿うよう、細かく分けて描く。
        for &(panel, a, b) in &self.snap_shown {
            let points: Vec<Pos2> = (0..=32)
                .filter_map(|k| {
                    let q = [a[0] + (b[0] - a[0]) * k as f32 / 32.0, a[1] + (b[1] - a[1]) * k as f32 / 32.0];
                    project(&mvp, rect, world.then_after(&placements[panel].at(q)).point([q[0], q[1], 0.0]))
                })
                .collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, SNAP_COLOR)));
        }

        self.view_presets(ui, rect);
        // 向きの目印（Blender と同じ形）。X は右、Y は上、Z は手前。軸の先の丸を押すと、その軸の側から見る
        // （もう一度押すと、反対の側から）。白抜きの丸は、軸のマイナスの側。奥のものから描く。
        let origin = pos2(rect.left() + 44.0, rect.bottom() - 44.0);
        let colors = [Color32::from_rgb(226, 104, 92), Color32::from_rgb(124, 196, 96), Color32::from_rgb(98, 150, 236)];
        let mut balls: Vec<(usize, f32)> = (0..3).flat_map(|a| [(a, 1.0), (a, -1.0)]).collect();
        balls.sort_by(|a, b| (view[a.0][2] * a.1).total_cmp(&(view[b.0][2] * b.1)));
        let gizmo = ui.interact(Rect::from_center_size(origin, vec2(84.0, 84.0)), ui.scope_id().with("axis-gizmo"), Sense::click());
        let pressed = gizmo.clicked().then(|| gizmo.interact_pointer_pos()).flatten();
        let mut chosen = None;
        for (axis, sign) in balls {
            let tip = origin + vec2(view[axis][0], -view[axis][1]) * 28.0 * sign;
            let color = colors[axis];
            let hot = gizmo.hover_pos().is_some_and(|p| (p - tip).length() <= 9.0);
            if sign > 0.0 {
                painter.line_segment([origin, tip], (2.0, color));
                painter.circle_filled(tip, if hot { 9.0 } else { 8.0 }, color);
                painter.text(tip, Align2::CENTER_CENTER, ["X", "Y", "Z"][axis], FontId::proportional(11.0), Color32::from_gray(20));
            } else {
                painter.circle(tip, if hot { 7.0 } else { 6.0 }, color.gamma_multiply(0.25), (1.2, color));
            }
            if pressed.is_some_and(|p| (p - tip).length() <= 9.0) {
                chosen = Some((axis, sign));
            }
        }
        if let Some((axis, sign)) = chosen {
            let (yaw, pitch) = axis_view(axis, sign);
            // いまその向きから見ていれば、反対の側から。
            let (yaw, pitch) = if angle_gap(self.yaw, yaw) < 0.05 && (self.pitch - pitch).abs() < 0.05 { axis_view(axis, -sign) } else { (yaw, pitch) };
            self.look_from(yaw, pitch);
        }
        if gizmo.hovered() {
            gizmo.on_hover_text(t!("Click an axis to view from that side (again for the opposite side)"));
        }
        // ダブルクリック: 柄の上なら、外のアプリで直す（文字なら、右の欄で打つ）。
        // ベクターの柄なら、押したところのパスを選ぶ（Illustrator のダイレクト選択に当たる）。
        if response.double_clicked() && self.tool != Tool::Hand && let Some((index, grab)) = decal_under {
            if matches!(self.decals[index].picture, Picture::Image { vector: true, .. }) {
                self.pick_path(index, grab);
            } else {
                self.select_only(Some(index));
                self.edit_selected();
            }
        }
        // 柄を右クリックしたら、その場で使える操作（差し替え、消す）。
        if self.selected.is_some() && self.tool != Tool::Hand {
            response.context_menu(|ui| self.decal_menu(ui));
        }
        candidate
    }

    /// 展開図。表（印刷面）か、裏（表を左右に裏返した向き）。いつも平らで、正面を向いている。
    /// 返すのは、素材を運んでいるとき、いま放したら置かれる柄。
    fn flat_view(&mut self, ui: &mut egui::Ui, rect: Rect, back: bool, carry: Option<(usize, Pos2)>) -> Option<Placed> {
        let response = ui.interact(rect, ui.scope_id().with(("flat-view", back)), Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(rect);
        let visuals = ui.visuals().clone();
        painter.rect_filled(rect, 0.0, BACKDROP);

        let shared = std::rc::Rc::clone(&self.template);
        let template = &*shared;
        let (sheet_w, sheet_h) = (template.max[0] - template.min[0], template.max[1] - template.min[1]);
        let area = Rect::from_min_max(rect.min + vec2(RULER_LEFT, RULER_TOP), rect.max);
        let fit = (area.width() / sheet_w).min(area.height() / sheet_h) * 0.9;
        let cam = &mut self.flat[back as usize];
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if let (true, Some(pointer)) = (scroll != 0.0, response.hover_pos()) {
                // カーソルの下の点を動かさずに、拡大、縮小する。
                let before = cam.zoom;
                cam.zoom = (cam.zoom * (scroll * 0.002).exp()).clamp(0.5, 16.0);
                let center = area.center() + cam.pan;
                cam.pan += (pointer - center) * (1.0 - cam.zoom / before);
            }
        }
        let reset = response.double_clicked();
        // 1 mm が画面の何 px か。
        let scale = fit * cam.zoom;
        let image = Rect::from_center_size(area.center() + cam.pan, vec2(sheet_w * scale, sheet_h * scale));
        // 画面の点から、展開図上の位置（mm）へ。裏は、左右が逆。
        let to_sheet = |pos: Pos2| {
            let (x, y) = ((pos.x - image.left()) / scale, (pos.y - image.top()) / scale);
            [if back { template.max[0] - x } else { template.min[0] + x }, template.max[1] - y]
        };
        let under = |pos: Pos2| {
            let at = to_sheet(pos);
            panel_at(template, at).map(|panel| (panel, at))
        };
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let hit = pointer.filter(|p| area.contains(*p)).and_then(under);
        if response.hovered() || response.dragged() {
            self.hover = hit.map(|(panel, _)| panel);
        }
        // 表の表示では表の柄、裏の表示では裏の柄を扱う。手のひらの道具では、柄をつかまない。
        let hand = self.tool == Tool::Hand;
        let (ids, places, offsets) = visible_spots(&self.decals, template, back);
        let surface = &self.surface;
        let decal_at = |(panel, at): (usize, [f32; 2])| surface.decal_at(template, &places, panel, at).filter(|_| !hand).map(|(i, g)| (ids[i], [g[0] + offsets[i][0], g[1] + offsets[i][1]]));
        let decal_under = hit.and_then(decal_at);
        // 選んでいる柄の、大きさと回転のつまみ（画面での四隅）。
        let to_screen = |p: [f32; 2]| pos2(image.left() + (if back { template.max[0] - p[0] } else { p[0] - template.min[0] }) * scale, image.top() + (template.max[1] - p[1]) * scale);
        let knobs = self.knob_target(back).map(|i| (i, transform::corners(&self.decals[i], template, super::visible::of(&self.decals[i])).map(to_screen)));
        let handle_under = |p: Pos2| knobs.and_then(|(i, quad)| transform::handle_at(quad, p).map(|h| (i, h, quad)));
        let press = ui.input(|i| i.pointer.press_origin());
        // マスクを筆で描いている間は、柄をつかまず、箱も回さない。
        let brushing = self.brush_target().is_some();
        if response.drag_started() && !brushing && let Some((index, handle, _)) = press.and_then(handle_under) {
            self.history.record(self.snapshot());
            self.transforming = Some(Transforming { index, handle, start: self.decals[index].decal, pressed: to_sheet(press.unwrap_or_default()), area: super::visible::of(&self.decals[index]) });
        }
        if response.drag_started() && self.transforming.is_none() && !brushing {
            let pressed = press.and_then(under);
            self.grab = pressed.and_then(decal_at);
            if let Some((index, _)) = self.grab {
                // 動かす前の場所へ戻せるように、覚えておく。
                self.history.record(self.snapshot());
                // 選んでいる柄をつかんだら、選んだものをまとめて動かす。選んでいない柄なら、その柄だけを選ぶ。
                if self.is_selected(index) {
                    if let Some(old) = self.selected.filter(|&old| old != index) {
                        self.extra.retain(|&i| i != index);
                        self.extra.push(old);
                    }
                    self.selected = Some(index);
                } else {
                    self.select_only(Some(index));
                }
                self.art_dirty = true;
            }
        }
        if let (true, Some(t), Some(p)) = (response.dragged(), self.transforming, pointer) {
            let keys = ui.input(|i| i.modifiers);
            self.decals[t.index].decal = transform::apply(&t, template, to_sheet(p), keys.shift == self.ratio_lock, keys.alt, keys.shift);
            self.art_dirty = true;
            self.back_dirty = true;
        } else if response.dragged() && !brushing {
            match (self.grab, hit) {
                (Some((index, grab)), Some((panel, at))) => {
                    let area = self.decals.get(index).map_or(super::visible::FULL, super::visible::of);
                    if let Some(decal) = self.decals.get_mut(index) {
                        let before = decal.center(template);
                        self.surface.drag(template, decal, panel, at, grab);
                        decal.decal = snap(&mut self.snap_guides, template, decal.decal, area, scale, ui);
                        self.move_along(index, before);
                        self.art_dirty = true;
                    }
                }
                (Some(_), None) => {}
                // 何もないところをドラッグしたら、展開図をずらす。
                (None, _) => self.flat[back as usize].pan += response.drag_delta(),
            }
        }
        if response.drag_stopped() {
            if self.grab.is_some() {
                self.log_transform();
            }
            self.grab = None;
            self.transforming = None;
        }
        if brushing {
            if (response.drag_started() || response.clicked()) && let Some((panel, at)) = press.and_then(under) {
                self.brush_at(panel, at, back, true);
            } else if response.dragged() && let Some((panel, at)) = hit {
                self.brush_at(panel, at, back, false);
            }
            if response.drag_stopped() {
                self.brush_end();
            }
            // 筆の大きさの丸を、カーソルのところに出す。
            if let (Some(p), Some(brush)) = (response.hover_pos(), self.brush.as_ref()) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                let r = brush.size * 0.5 * scale;
                ui.painter().circle_stroke(p, r, egui::Stroke::new(1.5, egui::Color32::WHITE));
                ui.painter().circle_stroke(p, r + 1.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
            }
        }
        let on_handle = response.interact_pointer_pos().or(response.hover_pos()).and_then(handle_under);
        let text_click = response.clicked() && self.tool == Tool::Text && !decal_under.is_some_and(|(index, _)| matches!(self.decals[index].picture, Picture::Text(_)));
        // 図形の道具では、図形を押せば選び、ほかを押せば、そこに図形を置く。
        let shape_click = response.clicked() && self.tool == Tool::Shape && !decal_under.is_some_and(|(index, _)| matches!(self.decals[index].picture, Picture::Shape(_)));
        if response.clicked() && on_handle.is_some() {
            // つまみを押しただけなら、選び直さない。
        } else if text_click && let Some((panel, at)) = hit {
            self.add_text_at(panel, at, back);
        } else if shape_click && let Some((panel, at)) = hit {
            self.add_shape_at(panel, at, back);
        } else if response.clicked() && self.tool != Tool::Hand && !brushing {
            let shift = ui.input(|i| i.modifiers.shift);
            self.select_click(decal_under.map(|(index, _)| index), shift);
        }
        // ベクターの柄をダブルクリックしたら、そのパスを選ぶ。
        if response.double_clicked() && self.tool != Tool::Hand && let Some((index, grab)) = decal_under
            && matches!(self.decals[index].picture, Picture::Image { vector: true, .. })
        {
            self.pick_path(index, grab);
        }
        // 右クリックは、選んでいる柄の上なら選んだものを保つ（まとめて消す、など）。
        if response.secondary_clicked() && self.tool != Tool::Hand && !decal_under.is_some_and(|(index, _)| self.is_selected(index)) {
            self.select_only(decal_under.map(|(index, _)| index));
        }
        // カーソルの下の柄は、ほかの表示でも光らせる。
        // 箱を回したり展開図をずらしたりしている間（ボタンを押している間）は光らせない（目障りで、描き直しも増えるので）。
        let pressing = ui.input(|i| i.pointer.any_down());
        if response.hovered() && self.grab.is_none() && !pressing && !response.dragged() {
            self.hover_now = self.hover_now.or(decal_under.map(|(index, _)| index));
        }
        if let Some(t) = self.transforming.filter(|_| response.dragged()) {
            ui.ctx().set_cursor_icon(knobs.map_or(egui::CursorIcon::Grabbing, |(_, quad)| transform::cursor(quad, t.handle)));
        } else if self.grab.is_some() && response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if let Some((_, handle, quad)) = on_handle.filter(|_| response.hovered()) {
            ui.ctx().set_cursor_icon(transform::cursor(quad, handle));
        } else if self.tool == Tool::Text && hit.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if self.tool == Tool::Shape && hit.is_some() && response.hovered() && decal_under.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if decal_under.is_some() && response.hovered() && !pressing {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // 運んでいる素材を、いま放したら置かれる柄。表の表示なら表、裏の表示なら裏。展開図では、上が上。
        let candidate = carry.filter(|(_, p)| area.contains(*p)).and_then(|(asset, pos)| {
            let (panel, at) = under(pos)?;
            self.asset_decal(asset, panel, at, back, 0.0)
        });
        let candidate = candidate.map(|mut placed| {
            placed.decal = snap(&mut self.snap_guides, template, placed.decal, super::visible::of(&placed), scale, ui);
            placed
        });

        // 展開図の絵。裏は、左右を逆にして貼る。
        let texture = if back { &self.back_tex } else { &self.front_tex };
        if let Some(texture) = texture {
            let uv = if back { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) };
            painter.with_clip_rect(area.intersect(rect)).image(texture.id(), image, uv, Color32::WHITE);
        }
        // 大きさと回転のつまみ（動かしたあとの場所で）。
        if let Some(i) = self.knob_target(back) {
            let quad = transform::corners(&self.decals[i], template, super::visible::of(&self.decals[i])).map(to_screen);
            transform::paint(&painter.with_clip_rect(area.intersect(rect)), quad, visuals.selection.bg_fill);
        }
        if let Some(index) = self.hover_shown(decal_under.map(|(i, _)| i), Some(back)) {
            self.paint_outline(&painter.with_clip_rect(area.intersect(rect)), index, |_, p| Some(to_screen(p)));
        }
        // 吸い付いた先の補助線。
        for &(_, a, b) in &self.snap_shown {
            painter.with_clip_rect(area.intersect(rect)).line_segment([to_screen(a), to_screen(b)], egui::Stroke::new(1.0, SNAP_COLOR));
        }

        // 定規（mm）。原点は、展開図の左上。
        let ruler_fill = visuals.panel_fill;
        let line = visuals.weak_text_color();
        let font = FontId::proportional(12.0);
        let (minor, major) = ruler_steps(scale);
        let top = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(rect.right(), rect.top() + RULER_TOP));
        let left = Rect::from_min_max(pos2(rect.left(), rect.top() + RULER_TOP), pos2(rect.left() + RULER_LEFT, rect.bottom()));
        painter.rect_filled(top, 0.0, ruler_fill);
        painter.rect_filled(left, 0.0, ruler_fill);
        // 目盛りは、展開図のある範囲（0 から、展開図の幅や高さまで）にだけ打つ。
        let ticks = |from: f32, to: f32, length: f32| {
            let first = (from.max(0.0) / minor).ceil() as i32;
            let last = (to.min(length) / minor).floor() as i32;
            (first..=last).map(move |n| n as f32 * minor)
        };
        let top_painter = painter.with_clip_rect(Rect::from_min_max(pos2(area.left(), rect.top()), pos2(rect.right(), area.top())));
        for mm in ticks((area.left() - image.left()) / scale, (area.right() - image.left()) / scale, sheet_w) {
            let x = image.left() + mm * scale;
            let labelled = (mm / major).fract().abs() < 1e-3;
            top_painter.line_segment([pos2(x, top.bottom() - if labelled { 7.0 } else { 3.0 }), pos2(x, top.bottom())], (1.0, line));
            if labelled {
                top_painter.text(pos2(x + 3.0, top.top() + 1.0), Align2::LEFT_TOP, format!("{mm:.0}"), font.clone(), line);
            }
        }
        let left_painter = painter.with_clip_rect(Rect::from_min_max(pos2(rect.left(), area.top()), pos2(area.left(), rect.bottom())));
        for mm in ticks((area.top() - image.top()) / scale, (area.bottom() - image.top()) / scale, sheet_h) {
            let y = image.top() + mm * scale;
            let labelled = (mm / major).fract().abs() < 1e-3;
            left_painter.line_segment([pos2(left.right() - if labelled { 7.0 } else { 3.0 }, y), pos2(left.right(), y)], (1.0, line));
            if labelled {
                left_painter.text(pos2(left.right() - 9.0, y + 1.0), Align2::RIGHT_TOP, format!("{mm:.0}"), font.clone(), line);
            }
        }
        // 定規の角。単位を出す。
        painter.rect_filled(Rect::from_min_max(rect.min, area.min), 0.0, ruler_fill);
        painter.text(rect.min + vec2(RULER_LEFT * 0.5, RULER_TOP * 0.5), Align2::CENTER_CENTER, "mm", font, line);

        tag(ui, area.min + vec2(8.0, 6.0), if back { t!("Back") } else { t!("Front") });
        // ダブルクリック: 柄の上なら、外のアプリで直す（文字なら、右の欄で打つ）。何もないところなら、表示を元に戻す。
        if reset {
            match decal_under {
                Some((index, _)) if self.tool != Tool::Hand => {
                    self.select_only(Some(index));
                    self.edit_selected();
                }
                _ => self.flat[back as usize] = super::FlatCam::default(),
            }
        }
        // 柄を右クリックしたら、その場で使える操作（差し替え、消す）。
        if self.selected.is_some() && self.tool != Tool::Hand {
            response.context_menu(|ui| self.decal_menu(ui));
        }
        candidate
    }
}

/// 表示の名前の札。
fn tag(ui: &egui::Ui, at: Pos2, text: &str) {
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), Color32::WHITE);
    let rect = Rect::from_min_size(at, galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(150));
    painter.galley(rect.min + vec2(6.0, 3.0), galley, Color32::WHITE);
}

/// 表示の上に浮かぶボタンの組の、半透明の座布団（右から左へ並べる）。
fn cushion(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(0x3a, 0x3a, 0x3a, 0xd8))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(0x48, 0x48, 0x48)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.spacing_mut().button_padding = vec2(6.0, 2.0);
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                add(ui);
            });
        });
}

/// 座布団の上のボタン。`on` は、入っている切り替え（明るい地で見せる）。
fn cushion_button(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let text = egui::RichText::new(text).size(11.5);
    let button = egui::Button::new(text).frame_when_inactive(on).fill(if on { super::theme::SELECTED } else { egui::Color32::TRANSPARENT }).corner_radius(4.0);
    ui.add(button)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(solid: bool, front: bool, back: bool) -> Shown {
        Shown { solid, front, back }
    }

    #[test]
    fn 表示が1つなら中央いっぱい() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(1000.0, 600.0));
        assert_eq!(layout(rect, shown(true, false, false)), [Some(rect), None, None]);
        assert_eq!(layout(rect, shown(false, true, false)), [None, Some(rect), None]);
        assert_eq!(layout(rect, shown(false, false, true)), [None, None, Some(rect)]);
    }

    #[test]
    fn 立体と展開図は左右に並ぶ() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        for (front, back) in [(true, false), (false, true)] {
            let [solid, f, b] = layout(rect, shown(true, front, back));
            let (solid, flat) = (solid.unwrap(), f.or(b).unwrap());
            assert!(solid.right() < flat.left(), "立体が左、展開図が右");
            assert!(solid.width() > flat.width(), "立体のほうが広い");
            assert_eq!((solid.height(), flat.height()), (600.0, 600.0));
        }
    }

    #[test]
    fn 表と裏は上下に並ぶ() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        let [solid, front, back] = layout(rect, shown(false, true, true));
        let (front, back) = (front.unwrap(), back.unwrap());
        assert!(solid.is_none());
        assert!(front.bottom() < back.top(), "表が上、裏が下");
        assert_eq!((front.width(), back.width()), (1000.0, 1000.0));
    }

    #[test]
    fn 表示が3つなら左に立体で右に表と裏() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        let [solid, front, back] = layout(rect, shown(true, true, true));
        let (solid, front, back) = (solid.unwrap(), front.unwrap(), back.unwrap());
        assert!(solid.right() < front.left() && solid.right() < back.left());
        assert!(front.bottom() < back.top());
        assert_eq!(front.width(), back.width());
        // 表示どうしは重ならず、全体からはみ出さない。
        for view in [solid, front, back] {
            assert!(rect.contains_rect(view));
        }
    }

    #[test]
    fn 最後の1つの表示は隠せない() {
        let mut s = Shown::SOLID_ONLY;
        assert!(!s.toggle(super::super::View::Solid));
        assert_eq!(s, Shown::SOLID_ONLY);
        assert!(s.toggle(super::super::View::Front));
        assert!(s.toggle(super::super::View::Solid));
        assert_eq!(s, shown(false, true, false));
        s.solo(super::super::View::Back);
        assert_eq!(s, shown(false, false, true));
    }

    #[test]
    fn 定規の目盛りは拡大に合わせて細かくなる() {
        // 1 mm が 1 px のとき: 細かい目盛りは 5 mm、数字は 50 mm ごと。
        assert_eq!(ruler_steps(1.0), (5.0, 50.0));
        // 拡大すると、細かくなる。
        let (minor, major) = ruler_steps(8.0);
        assert!(minor <= 1.0 && major <= 10.0);
        // 縮小すると、粗くなる。数字どうしが重ならない間隔を保つ。
        let (minor, major) = ruler_steps(0.2);
        assert!(minor * 0.2 >= 5.0 && major * 0.2 >= 44.0);
    }
}
