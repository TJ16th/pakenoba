//! ECMA の型を、組んだ形と展開図の小さな絵を見比べて選ぶ一覧。
//!
//! 番号と言葉だけでは、どの閉じ方がどんな形か分かりにくい。1 つの型ごとに、上と底のフタを半分開けた箱の絵と、
//! 展開図の絵を並べる。絵は、見えている型の分だけ、少しずつ作って取っておく。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use pakenoba_core::ecma::{COMMON, Closure, Code, LISTED};
use pakenoba_core::template::Template;
use pakenoba_i18n::t;

use super::{App, theme};

/// 絵を作るときの寸法（mm）。型どうしを見比べやすいよう、どれも同じ。
const SIZE: (f32, f32, f32, f32) = (100.0, 60.0, 150.0, 0.5);
/// 1 枚の札の大きさ。
const CARD: Vec2 = Vec2::new(300.0, 214.0);
/// 札の中の、絵の高さ。
const ART_H: f32 = 140.0;

/// 1 つの型の絵（座標は絵ごとの単位のまま。描くときに枠へ合わせる）。
pub(super) struct Thumb {
    /// 組んだ箱の面（画面へ映した多角形と、色の濃さ。奥のものから並ぶ）。
    faces: Vec<(Vec<[f32; 2]>, f32)>,
    /// 展開図の線（始点、終点、切り線か）。
    lines: Vec<([f32; 2], [f32; 2], bool)>,
}

/// 一覧の窓の状態。
#[derive(Default)]
pub(super) struct Gallery {
    /// 上の閉じ方で絞る。
    top: Option<Closure>,
    /// 底の閉じ方で絞る。
    bottom: Option<Closure>,
    thumbs: HashMap<u32, Option<Thumb>>,
}

/// 箱を見る向き（斜め上、正面と右の側面が見える）。
fn view() -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (yaw, pitch) = (35f32.to_radians(), 22f32.to_radians());
    // カメラのある向き（箱から見て）。
    let d = [yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()];
    let right = norm([d[2], 0.0, -d[0]]);
    let up = cross(d, right);
    (d, right, up)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// 型の絵を作る。作れない型は None。
fn make_thumb(code: Code) -> Option<Thumb> {
    let (l, w, h, t) = SIZE;
    let tpl = Template::ecma(code, l, w, h, t).ok()?;
    // 上と底のフタを半分ほど開けて、閉じ方が見えるようにする。
    let places = tpl.transforms_at(1.0, [0.45, 0.45]);
    let world = tpl.world_at(1.0);
    let (d, right, up) = view();
    let light = norm([d[0] + 0.3, d[1] + 0.8, d[2] - 0.2]);
    let mut faces: Vec<(Vec<[f32; 2]>, f32, f32)> = Vec::new();
    for (panel, place) in tpl.panels.iter().zip(&places) {
        let pts: Vec<[f32; 3]> = panel.outline.iter().map(|&q| world.point(place.at(q).point([q[0], q[1], 0.5 * t]))).collect();
        if pts.len() < 3 {
            continue;
        }
        // 面の向き（多角形の面積のベクトル）。
        let mut n = [0.0f32; 3];
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            n = [n[0] + (a[1] - b[1]) * (a[2] + b[2]), n[1] + (a[2] - b[2]) * (a[0] + b[0]), n[2] + (a[0] - b[0]) * (a[1] + b[1])];
        }
        let mut n = norm(n);
        // こちらを向いていなければ、箱の内側（裏）が見えている。
        let inside = dot(n, d) < 0.0;
        if inside {
            n = [-n[0], -n[1], -n[2]];
        }
        let lit = 0.62 + 0.38 * dot(n, light).max(0.0);
        let shade = if inside { lit * 0.62 } else { lit };
        let depth = pts.iter().map(|&p| dot(p, d)).sum::<f32>() / pts.len() as f32;
        faces.push((pts.iter().map(|&p| [dot(p, right), -dot(p, up)]).collect(), shade, depth));
    }
    // 奥のものから描く。
    faces.sort_by(|a, b| a.2.total_cmp(&b.2));
    let drawing = pakenoba_core::ecma::drawing(code, l, w, h, t);
    // 展開図は、上が箱の上になる向き（紙の座標は上が +v。画面は下が +y）。
    let lines = drawing.paths.iter().filter(|p| p.points.len() >= 2).map(|p| ([p.points[0][0], -p.points[0][1]], [p.points[1][0], -p.points[1][1]], p.group == 0)).collect();
    Some(Thumb { faces: faces.into_iter().map(|(f, s, _)| (f, s)).collect(), lines })
}

/// 点の集まりを、枠に収めて描くための変換（縦横の比は保つ）。
fn fit(points: impl Iterator<Item = [f32; 2]>, rect: Rect) -> impl Fn([f32; 2]) -> Pos2 {
    let (lo, hi) = points.fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
    let size = [(hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3)];
    let s = (rect.width() / size[0]).min(rect.height() / size[1]);
    let offset = rect.center() - Vec2::new(size[0] * s, size[1] * s) * 0.5;
    move |p| offset + Vec2::new((p[0] - lo[0]) * s, (p[1] - lo[1]) * s)
}

fn draw_box(painter: &egui::Painter, thumb: &Thumb, rect: Rect) {
    let to = fit(thumb.faces.iter().flat_map(|(f, _)| f.iter().copied()), rect);
    for (face, shade) in &thumb.faces {
        let pts: Vec<Pos2> = face.iter().map(|&p| to(p)).collect();
        let v = (232.0 * shade).clamp(0.0, 255.0) as u8;
        let fill = Color32::from_rgb(v, (v as f32 * 0.985) as u8, (v as f32 * 0.955) as u8);
        // 凹んだ面（切り欠きのある差し込みなど）もあるので、三角形に分けて塗る。
        let tris = pakenoba_core::template::triangles(&face.iter().map(|&p| [p[0], p[1]]).collect::<Vec<_>>());
        let mut mesh = egui::Mesh::default();
        for tri in tris {
            for p in tri {
                mesh.colored_vertex(to(p), fill);
            }
        }
        mesh.indices = (0..mesh.vertices.len() as u32).collect();
        painter.add(egui::Shape::mesh(mesh));
        painter.add(egui::Shape::closed_line(pts, Stroke::new(0.8, Color32::from_rgb(70, 70, 70))));
    }
}

fn draw_flat(painter: &egui::Painter, thumb: &Thumb, rect: Rect) {
    let to = fit(thumb.lines.iter().flat_map(|l| [l.0, l.1]), rect);
    // 折り線を先に、切り線を上に。
    for cut in [false, true] {
        let stroke = if cut { Stroke::new(1.0, Color32::from_rgb(222, 222, 222)) } else { Stroke::new(1.0, Color32::from_rgb(0, 150, 220)) };
        for &(a, b, _) in thumb.lines.iter().filter(|l| l.2 == cut) {
            painter.line_segment([to(a), to(b)], stroke);
        }
    }
}

impl Gallery {
    /// 絞り込んだ型の番号（よく使う型を先に）。
    fn codes(&self) -> Vec<Code> {
        let mut out: Vec<Code> = COMMON.iter().chain(LISTED.iter().filter(|n| !COMMON.contains(n))).filter_map(|&n| Code::from_u32(n)).collect();
        out.retain(|c| self.top.is_none_or(|t| c.top == t) && self.bottom.is_none_or(|b| c.bottom == b));
        out
    }
}

/// 閉じ方で絞る、ボタンの並び。
fn closure_chips(ui: &mut egui::Ui, label: &str, value: &mut Option<Closure>, top: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([56.0, 22.0], egui::Label::new(egui::RichText::new(label).color(theme::MUTED)));
        ui.selectable_value(value, None, t!("All styles"));
        for c in Closure::ALL.into_iter().filter(|c| if top { c.on_top() } else { c.on_bottom() }) {
            ui.selectable_value(value, Some(c), c.label());
        }
    });
}

impl App {
    /// 右の「ECMA の番号」から、一覧を開く。
    pub(super) fn open_ecma_gallery(&mut self) {
        if self.ecma_gallery.is_none() {
            self.ecma_gallery = Some(Gallery::default());
        }
    }

    /// 一覧の窓（開いていれば、毎回の描き直しで）。
    pub(super) fn ecma_gallery_window(&mut self, ctx: &egui::Context) {
        let Some(mut gallery) = self.ecma_gallery.take() else { return };
        let current = (self.dims.sides == super::ECMA).then(|| Code::from_u32(self.dims.ecma)).flatten();
        let mut open = true;
        let mut chosen = None;
        egui::Window::new(t!("Choose an ECMA style")).open(&mut open).default_size([960.0, 680.0]).default_pos([16.0, 60.0]).resizable(true).collapsible(false).show(ctx, |ui| {
            ui.label(egui::RichText::new(t!("Each style shows the box with the top and bottom half open, and its dieline. Click one to use it; the box on the main view changes, so you can compare.")).color(theme::MUTED));
            ui.add_space(4.0);
            closure_chips(ui, t!("Top of the box"), &mut gallery.top, true);
            closure_chips(ui, t!("Bottom of the box"), &mut gallery.bottom, false);
            let codes = gallery.codes();
            ui.weak(t!("{} styles", codes.len()));
            ui.separator();
            // 絵は、1 回の描き直しで作る時間を区切る（残りは次の描き直しで）。
            let deadline = Instant::now() + Duration::from_millis(12);
            let mut pending = false;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                    for code in codes {
                        let n = code.to_u32();
                        let (rect, response) = ui.allocate_exact_size(CARD, egui::Sense::click());
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        if let std::collections::hash_map::Entry::Vacant(slot) = gallery.thumbs.entry(n) {
                            if Instant::now() < deadline {
                                slot.insert(make_thumb(code));
                            } else {
                                pending = true;
                            }
                        }
                        let selected = current == Some(code);
                        let painter = ui.painter_at(rect);
                        let fill = if response.hovered() { theme::HOVER } else { theme::SUNKEN };
                        painter.rect_filled(rect, 6.0, fill);
                        if selected {
                            painter.rect_stroke(rect.shrink(1.0), 6.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
                        }
                        let art = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(rect.width() - 16.0, ART_H));
                        let (left, right) = (Rect::from_min_max(art.min, Pos2::new(art.center().x - 4.0, art.max.y)), Rect::from_min_max(Pos2::new(art.center().x + 4.0, art.min.y), art.max));
                        match gallery.thumbs.get(&n) {
                            Some(Some(thumb)) => {
                                draw_box(&painter, thumb, left.shrink(4.0));
                                draw_flat(&painter, thumb, right.shrink(4.0));
                            }
                            Some(None) => {
                                painter.text(art.center(), egui::Align2::CENTER_CENTER, t!("No preview"), egui::FontId::proportional(11.0), theme::FAINT);
                            }
                            None => {
                                painter.text(art.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(14.0), theme::FAINT);
                            }
                        }
                        let mut y = art.max.y + 6.0;
                        let name = if COMMON.contains(&n) { format!("{}   ★ {}", code.name(), t!("Common")) } else { code.name() };
                        painter.text(Pos2::new(rect.min.x + 10.0, y), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(13.0), theme::TEXT);
                        y += 18.0;
                        for (label, c, front) in [(t!("Top of the box"), code.top, code.top_front), (t!("Bottom of the box"), code.bottom, code.bottom_front)] {
                            let side = if c.has_side() { format!("（{}）", if front { t!("Front panel") } else { t!("Back panel") }) } else { String::new() };
                            painter.text(Pos2::new(rect.min.x + 10.0, y), egui::Align2::LEFT_TOP, format!("{label}: {}{side}", c.label()), egui::FontId::proportional(11.0), theme::MUTED);
                            y += 15.0;
                        }
                        if response.on_hover_text(t!("Use this style")).clicked() {
                            chosen = Some(code);
                        }
                    }
                });
            });
            if pending {
                ctx.request_repaint();
            }
        });
        if let Some(code) = chosen {
            self.set_ecma(code);
        }
        if open {
            self.ecma_gallery = Some(gallery);
        }
    }
}
