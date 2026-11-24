//! 図形の道具と、図形の柄の設定。
//!
//! 図形の道具（U）で箱や展開図を押すと、そこに四角か丸を置く。塗り、線、線の太さ、角の丸みを変えられる。
//! 図形は、形と色から、その場で描く。大きくしても、ぼやけない。

use pakenoba_i18n::t;
use eframe::egui;

use pakenoba_core::project::{SHAPE_RADIUS, SHAPE_STROKE, ShapeKind, ShapeSpec};
use pakenoba_core::surface::Decal;
use pakenoba_core::template::P2;

use super::App;
use crate::paint::{Picture, Placed};

/// 新しく置く図形の大きさ（mm）。
const FIRST_SIZE: P2 = [40.0, 30.0];

/// はじめの図形（四角、塗りだけ）。
pub(super) fn default_shape() -> ShapeSpec {
    ShapeSpec {
        kind: ShapeKind::Rect,
        fill: Some([8, 126, 139]),
        stroke: None,
        stroke_width: 1.0,
        radius: 0.0,
    }
}

/// 塗りか線の、使う／使わないと色。変えたら真。
fn color_toggle(ui: &mut egui::Ui, color: &mut Option<[u8; 3]>, last: [u8; 3], tip: &str) -> bool {
    let mut on = color.is_some();
    let mut rgb = color.unwrap_or(last);
    let mut changed = ui.checkbox(&mut on, "").on_hover_text(tip).changed();
    ui.add_enabled_ui(on, |ui| {
        changed |= ui
            .color_edit_button_srgb(&mut rgb)
            .on_hover_text(tip)
            .changed();
    });
    if changed {
        *color = on.then_some(rgb);
    }
    changed
}

/// 図形の操作（形、塗り、線、線の太さ、角の丸み）。変えたら真。
fn shape_controls(ui: &mut egui::Ui, spec: &mut ShapeSpec, compact: bool) -> bool {
    let mut changed = false;
    let row = |ui: &mut egui::Ui, label: &str| {
        if !compact {
            ui.end_row();
            ui.label(label);
        }
    };
    ui.horizontal(|ui| {
        for (kind, label) in [(ShapeKind::Rect, t!("Rectangle")), (ShapeKind::Ellipse, t!("Ellipse"))] {
            changed |= ui.selectable_value(&mut spec.kind, kind, label).changed();
        }
    });
    row(ui, t!("Fill"));
    ui.horizontal(|ui| {
        changed |= color_toggle(
            ui,
            &mut spec.fill,
            [8, 126, 139],
            t!("Fill color (uncheck for no fill)"),
        );
    });
    row(ui, t!("Stroke"));
    ui.horizontal(|ui| {
        changed |= color_toggle(
            ui,
            &mut spec.stroke,
            [20, 20, 20],
            t!("Stroke color (uncheck for no stroke)"),
        );
        ui.add_enabled_ui(spec.stroke.is_some(), |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut spec.stroke_width)
                        .range(SHAPE_STROKE)
                        .speed(0.05)
                        .max_decimals(1)
                        .suffix(" mm"),
                )
                .on_hover_text(t!("Stroke width (inward from the edge)"))
                .changed();
        });
    });
    row(ui, t!("Corner radius"));
    ui.add_enabled_ui(spec.kind == ShapeKind::Rect, |ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut spec.radius)
                    .range(SHAPE_RADIUS)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" mm"),
            )
            .on_hover_text(t!("Rounding of the rectangle's corners (radius)"))
            .changed();
    });
    if !compact {
        ui.end_row();
    }
    changed
}

impl App {
    /// 図形の柄を、パネル `panel` の、展開図の上の点 `at` を中心に置き、選ぶ。
    pub(super) fn add_shape_at(&mut self, panel: usize, at: P2, back: bool) {
        self.checkpoint();
        let mut decal = Decal {
            panel,
            at: [0.5, 0.5],
            size: FIRST_SIZE,
            angle: 0.0,
        };
        decal.set_center(&self.template, at);
        self.decals.push(Placed {
            decal,
            picture: Picture::Shape(self.shape_style.clone()),
            back,
            layer: Default::default(),
        });
        self.select_only(Some(self.decals.len() - 1));
        self.log_added(self.decals.len() - 1);
        (self.art_dirty, self.back_dirty) = (true, true);
        self.say(t!("Placed a shape. Change its form, colors and size in the right panel."));
    }

    /// 図形の柄 `index` の形と色を `spec` に変える。
    pub(super) fn reshape(&mut self, index: usize, spec: ShapeSpec) {
        let Some(placed) = self.decals.get_mut(index) else {
            return;
        };
        if matches!(&placed.picture, Picture::Shape(old) if *old == spec) {
            return;
        }
        placed.picture = Picture::Shape(spec);
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 変えたことを、元に戻せるように覚える。続けて変えているあいだは、1 回ぶんにまとめる。
    fn note_change(&mut self, earlier: super::Snapshot) {
        if !self.editing {
            self.history.record(earlier);
        }
        self.editing = true;
    }

    /// 右の欄の、図形の柄の設定。プロパティの欄の上に出す。
    pub(super) fn shape_properties(&mut self, ui: &mut egui::Ui, index: usize, locked: bool) {
        let Some(Picture::Shape(mut spec)) = self.decals.get(index).map(|d| d.picture.clone())
        else {
            return;
        };
        let earlier = self.snapshot();
        let mut changed = false;
        ui.add_enabled_ui(!locked, |ui| {
            egui::Grid::new("shape-style")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t!("Form"));
                    changed |= shape_controls(ui, &mut spec, false);
                });
        });
        if changed {
            self.reshape(index, spec.clone());
            // 次に置く図形も、同じにする。
            self.shape_style = spec;
            self.note_change(earlier);
        }
        ui.add_space(4.0);
    }

    /// 上の帯（図形の道具のとき）。選んでいるのが図形なら、それを変える。そうでなければ、次に置く図形。
    pub(super) fn shape_bar(&mut self, ui: &mut egui::Ui) {
        let target = self.selected.filter(|&i| {
            matches!(
                self.decals.get(i).map(|d| &d.picture),
                Some(Picture::Shape(_))
            ) && !self.decals[i].layer.locked
        });
        let mut spec = match target.map(|i| &self.decals[i].picture) {
            Some(Picture::Shape(spec)) => spec.clone(),
            _ => self.shape_style.clone(),
        };
        let earlier = self.snapshot();
        ui.label(t!("Form"));
        let changed = shape_controls(ui, &mut spec, true);
        ui.separator();
        ui.weak(if target.is_some() {
            t!("The selected shape")
        } else {
            t!("Click the box or the dieline to place a shape there")
        });
        if changed {
            if let Some(index) = target {
                self.reshape(index, spec.clone());
                self.note_change(earlier);
            }
            self.shape_style = spec;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn 図形を置いて色を変えると_保存して戻る() {
        let mut app = App::new(&Initial::default());
        let before = app.decals.len();
        let panel = app
            .template
            .panels
            .iter()
            .position(|p| p.name == "正面")
            .unwrap();
        let at = Decal {
            panel,
            at: [0.5, 0.5],
            size: [1.0, 1.0],
            angle: 0.0,
        }
        .center(&app.template);
        app.add_shape_at(panel, at, false);
        let index = app.decals.len() - 1;
        assert_eq!(app.decals.len(), before + 1);
        assert_eq!(app.decals[index].decal.size, FIRST_SIZE);
        let spec = ShapeSpec {
            kind: ShapeKind::Ellipse,
            fill: None,
            stroke: Some([200, 0, 0]),
            stroke_width: 2.0,
            radius: 0.0,
        };
        app.reshape(index, spec.clone());
        assert!(matches!(&app.decals[index].picture, Picture::Shape(s) if *s == spec));
        // 保存して開き直しても、同じ図形。
        let project = app.project().unwrap();
        let mut again = App::new(&Initial::default());
        let problems = again.apply_project(project);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(matches!(&again.decals[index].picture, Picture::Shape(s) if *s == spec));
        // 元に戻すと、置く前。
        app.undo();
        assert_eq!(app.decals.len(), before);
    }

    #[test]
    fn 図形の絵は_塗りと線と外を描き分ける() {
        let spec = ShapeSpec {
            kind: ShapeKind::Rect,
            fill: Some([0, 0, 255]),
            stroke: Some([255, 0, 0]),
            stroke_width: 2.0,
            radius: 0.0,
        };
        let picture = Picture::Shape(spec);
        let size = [40.0, 30.0];
        let center = picture.sample([0.0, 0.0], size);
        assert_eq!((center[2], center[3]), (255.0, 1.0));
        let line = picture.sample([19.0, 0.0], size);
        assert!(line[0] > 250.0 && line[3] > 0.99, "{line:?}");
        let outside = picture.sample([21.0, 0.0], size);
        assert_eq!(outside[3], 0.0);
        // 丸は、四隅が外。
        let ellipse = Picture::Shape(ShapeSpec {
            kind: ShapeKind::Ellipse,
            ..default_shape()
        });
        assert_eq!(ellipse.sample([19.0, 14.0], size)[3], 0.0);
        assert_eq!(ellipse.sample([0.0, 14.0], size)[3], 1.0);
    }
}
