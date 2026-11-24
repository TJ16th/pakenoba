//! 柄の選び方（1 つ、いくつも）と、カーソルの下の柄の強調、整列。
//!
//! 選んでいる柄は、主に選んでいるもの（`selected`。プロパティの欄に出す）と、いっしょに選んでいるもの（`extra`）。
//! Shift を押しながらクリックで足したり外したりし、Ctrl+A ですべて選ぶ。いくつも選んでいると、まとめて動かす、消す、整列する。
//!
//! カーソルの下の柄は、立体、表、裏、レイヤーの欄のどこで指していても、すべての表示で同じ柄に色の枠を出す。

use pakenoba_i18n::t;
use eframe::egui;

use pakenoba_core::surface::panel_at;
use pakenoba_core::template::P2;

use super::App;

/// 整列のしかた。展開図（表）の向きで、左、横の中央、右、上、縦の中央、下。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Align {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
}

impl App {
    /// 選んでいる柄の番号（主に選んでいるものが先頭）。
    pub(super) fn selection(&self) -> Vec<usize> {
        let mut all: Vec<usize> = self.selected.into_iter().chain(self.extra.iter().copied()).filter(|&i| i < self.decals.len()).collect();
        let mut seen = Vec::new();
        all.retain(|i| {
            let fresh = !seen.contains(i);
            seen.push(*i);
            fresh
        });
        all
    }

    pub(super) fn is_selected(&self, index: usize) -> bool {
        self.selected == Some(index) || self.extra.contains(&index)
    }

    /// その柄だけを選ぶ（None なら、何も選ばない）。
    pub(super) fn select_only(&mut self, index: Option<usize>) {
        // 別の柄を選んだら、パスの選択は外す。
        if self.path_pick.is_some_and(|(i, ..)| Some(i) != index) {
            self.path_pick = None;
        }
        self.selected = index;
        self.extra.clear();
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// クリックで選ぶ。Shift を押していれば、足すか外す。押していなければ、その柄だけ。
    pub(super) fn select_click(&mut self, index: Option<usize>, shift: bool) {
        match (index, shift) {
            (Some(index), true) => {
                if self.is_selected(index) {
                    // 外す。主に選んでいるものを外したら、残りの最後を主にする。
                    self.extra.retain(|&i| i != index);
                    if self.selected == Some(index) {
                        self.selected = self.extra.pop();
                    }
                } else {
                    if let Some(old) = self.selected {
                        self.extra.push(old);
                    }
                    self.selected = Some(index);
                }
                (self.art_dirty, self.back_dirty) = (true, true);
            }
            // Shift を押して何もないところを押したときは、選んだものを保つ。
            (None, true) => {}
            (index, false) => self.select_only(index),
        }
    }

    /// すべての柄を選ぶ（隠した柄は除く）。
    pub(super) fn select_all(&mut self) {
        let all: Vec<usize> = (0..self.decals.len()).filter(|&i| crate::paint::is_artwork(&self.decals[i].picture) && crate::paint::shown_in_group(&self.decals, i)).collect();
        self.selected = all.last().copied();
        self.extra = all[..all.len().saturating_sub(1)].to_vec();
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// つかんでいる柄（`moved`）を動かしたあとに、いっしょに選んでいる柄も同じだけ動かす。
    /// `before` は、つかんでいる柄の、動かす前の中心（展開図の上）。ロックした柄は動かさない。
    pub(super) fn move_along(&mut self, moved: usize, before: P2) {
        let Some(after) = self.decals.get(moved).map(|d| d.center(&self.template)) else { return };
        let delta = [after[0] - before[0], after[1] - before[1]];
        if delta[0].abs() < 1e-6 && delta[1].abs() < 1e-6 {
            return;
        }
        for index in self.selection() {
            if index == moved || self.decals[index].layer.fixed() || !crate::paint::is_artwork(&self.decals[index].picture) {
                continue;
            }
            self.shift_decal(index, delta);
        }
    }

    /// 柄を、展開図の上で `delta`（mm）だけずらす。行き先のパネルが変われば、そのパネルに置き直す（絵は回らない）。
    fn shift_decal(&mut self, index: usize, delta: P2) {
        let template = &self.template;
        let decal = &mut self.decals[index];
        let c = decal.center(template);
        let to = [c[0] + delta[0], c[1] + delta[1]];
        match panel_at(template, to) {
            Some(panel) => self.surface.drag(template, decal, panel, to, [0.0, 0.0]),
            None => decal.set_center(template, to),
        }
    }

    /// 柄の、展開図の上での外接する四角（左下と右上）。
    pub(super) fn sheet_bounds(&self, index: usize) -> (P2, P2) {
        let decal = &self.decals[index];
        let c = decal.center(&self.template);
        let (cos, sin) = (decal.angle.cos().abs(), decal.angle.sin().abs());
        let half = [decal.size[0] * 0.5 * cos + decal.size[1] * 0.5 * sin, decal.size[0] * 0.5 * sin + decal.size[1] * 0.5 * cos];
        ([c[0] - half[0], c[1] - half[1]], [c[0] + half[0], c[1] + half[1]])
    }

    /// 選んでいる柄をそろえる。2 つ以上なら、選んだもの全体の四角に。1 つなら、置いているパネルに（面の中央に置く、など）。
    /// 裏の柄だけを選んでいるときは、「裏」の表示（左右が逆）で見た左右にする。
    pub(super) fn align(&mut self, how: Align) {
        let chosen: Vec<usize> = self.selection().into_iter().filter(|&i| !self.decals[i].layer.fixed() && crate::paint::is_artwork(&self.decals[i].picture)).collect();
        if chosen.is_empty() {
            return;
        }
        let (lo, hi) = if chosen.len() == 1 {
            let outline = &self.template.panels[self.decals[chosen[0]].panel].outline;
            let lo = outline.iter().fold([f32::INFINITY; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
            let hi = outline.iter().fold([f32::NEG_INFINITY; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
            (lo, hi)
        } else {
            chosen.iter().map(|&i| self.sheet_bounds(i)).fold(([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]), |(lo, hi), (a, b)| ([lo[0].min(a[0]), lo[1].min(a[1])], [hi[0].max(b[0]), hi[1].max(b[1])]))
        };
        let mirrored = chosen.iter().all(|&i| self.decals[i].back);
        let how = match (how, mirrored) {
            (Align::Left, true) => Align::Right,
            (Align::Right, true) => Align::Left,
            (other, _) => other,
        };
        self.checkpoint();
        for index in chosen {
            let (a, b) = self.sheet_bounds(index);
            let delta = match how {
                Align::Left => [lo[0] - a[0], 0.0],
                Align::Right => [hi[0] - b[0], 0.0],
                Align::CenterX => [(lo[0] + hi[0] - a[0] - b[0]) * 0.5, 0.0],
                Align::Top => [0.0, hi[1] - b[1]],
                Align::Bottom => [0.0, lo[1] - a[1]],
                Align::CenterY => [0.0, (lo[1] + hi[1] - a[1] - b[1]) * 0.5],
            };
            self.shift_decal(index, delta);
        }
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 選んでいる柄（3 つ以上）を、横（`horizontal`）か縦に、すき間が等しくなるように並べる。両端の柄は動かさない。
    pub(super) fn distribute(&mut self, horizontal: bool) {
        let axis = if horizontal { 0 } else { 1 };
        let mut chosen: Vec<(usize, P2, P2)> = self.selection().into_iter().filter(|&i| !self.decals[i].layer.fixed() && crate::paint::is_artwork(&self.decals[i].picture)).map(|i| {
            let (a, b) = self.sheet_bounds(i);
            (i, a, b)
        }).collect();
        if chosen.len() < 3 {
            return;
        }
        chosen.sort_by(|x, y| (x.1[axis] + x.2[axis]).total_cmp(&(y.1[axis] + y.2[axis])));
        let span = chosen.last().map(|c| c.2[axis]).unwrap_or(0.0) - chosen[0].1[axis];
        let widths: f32 = chosen.iter().map(|c| c.2[axis] - c.1[axis]).sum();
        let gap = (span - widths) / (chosen.len() - 1) as f32;
        self.checkpoint();
        let mut edge = chosen[0].1[axis];
        for (index, a, b) in chosen {
            let mut delta = [0.0, 0.0];
            delta[axis] = edge - a[axis];
            self.shift_decal(index, delta);
            edge += b[axis] - a[axis] + gap;
        }
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 上の帯とプロパティの欄に出す、整列のボタン。
    pub(super) fn align_buttons(&mut self, ui: &mut egui::Ui) {
        let count = self.selection().len();
        let tip = if count >= 2 { t!("to the box around all the selected artwork") } else { t!("to the panel it is on") };
        for (how, label) in [(Align::Left, t!("Left")), (Align::CenterX, t!("Horizontal center")), (Align::Right, t!("Right")), (Align::Top, t!("Top")), (Align::CenterY, t!("Vertical center")), (Align::Bottom, t!("Bottom"))] {
            if ui.small_button(label).on_hover_text(t!("Align {label} ({tip})")).clicked() {
                self.align(how);
            }
        }
        if count >= 3 {
            if ui.small_button(t!("Distribute horizontally")).clicked() {
                self.distribute(true);
            }
            if ui.small_button(t!("Distribute vertically")).clicked() {
                self.distribute(false);
            }
        }
    }

    /// 1 回の画面の終わりに、カーソルの下の柄が変わっていたら、絵を描き直す。
    pub(super) fn settle_hover(&mut self, ctx: &egui::Context) {
        let now = std::mem::take(&mut self.hover_now).filter(|&i| i < self.decals.len());
        // 運んでいる間とつかんでいる間は、強調しない（描き直しを増やさない）。
        let now = if self.carry.is_some() || self.grab.is_some() { None } else { now };
        if now != self.hover_decal {
            self.hover_decal = now;
            (self.art_dirty, self.back_dirty) = (true, true);
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Picture;

    fn three() -> App {
        let mut app = App::new(&Initial::default());
        for x in [0.2, 0.5, 0.9] {
            app.add_decal(Picture::Logo, [20.0, 20.0]);
            let last = app.decals.len() - 1;
            app.decals[last].decal.at = [x, 0.5];
        }
        app
    }

    #[test]
    fn shift_で足したり外したりし_すべて選べる() {
        let mut app = three();
        app.select_click(Some(0), false);
        app.select_click(Some(2), true);
        assert_eq!(app.selection(), vec![2, 0]);
        app.select_click(Some(2), true);
        assert_eq!(app.selection(), vec![0], "主に選んでいるものを外すと、残りが主になる");
        app.select_click(None, true);
        assert_eq!(app.selection(), vec![0], "Shift で何もないところを押しても、外さない");
        app.select_click(Some(1), false);
        assert_eq!(app.selection(), vec![1]);
        app.decals[0].layer.hidden = true;
        app.select_all();
        let mut all = app.selection();
        all.sort();
        assert_eq!(all, vec![1, 2], "隠した柄は選ばない");
    }

    #[test]
    fn いっしょに選んだ柄は_同じだけ動き_ロックした柄は動かない() {
        let mut app = three();
        app.select_all();
        app.decals[2].layer.locked = true;
        let before: Vec<P2> = (0..3).map(|i| app.decals[i].center(&app.template)).collect();
        let moved = app.selected.unwrap();
        let start = app.decals[moved].center(&app.template);
        // 主に選んでいる柄を 5 mm 右へ動かしたことにする。
        let c = app.decals[moved].center(&app.template);
        let template = app.template.clone();
        app.decals[moved].set_center(&template, [c[0] + 5.0, c[1] - 3.0]);
        app.move_along(moved, start);
        for (i, was) in before.iter().enumerate() {
            let now = app.decals[i].center(&app.template);
            let expected = if app.decals[i].layer.locked && i != moved { *was } else { [was[0] + 5.0, was[1] - 3.0] };
            assert!((now[0] - expected[0]).abs() < 1e-3 && (now[1] - expected[1]).abs() < 1e-3, "柄 {i}: {now:?} と {expected:?}");
        }
    }

    #[test]
    fn 整列と等間隔() {
        let mut app = three();
        // 縦の位置をばらばらにしてから、上にそろえる。
        for (i, y) in [0.3, 0.6, 0.45].into_iter().enumerate() {
            app.decals[i].decal.at[1] = y;
        }
        app.select_all();
        app.align(Align::Top);
        let tops: Vec<f32> = (0..3).map(|i| app.sheet_bounds(i).1[1]).collect();
        assert!(tops.iter().all(|t| (t - tops[0]).abs() < 1e-3), "{tops:?}");
        app.undo();
        assert!((app.sheet_bounds(0).1[1] - app.sheet_bounds(1).1[1]).abs() > 1.0, "元に戻せる");
        // 横に等間隔。両端は動かず、すき間が等しい。
        let left = app.sheet_bounds(0).0[0];
        app.decals[1].decal.at[0] = 0.3;
        assert_eq!(app.selection().len(), 3, "元に戻しても、選んだものは保つ");
        app.distribute(true);
        let boxes: Vec<(P2, P2)> = (0..3).map(|i| app.sheet_bounds(i)).collect();
        assert!((boxes[0].0[0] - left).abs() < 1e-3);
        let gaps = [boxes[1].0[0] - boxes[0].1[0], boxes[2].0[0] - boxes[1].1[0]];
        assert!((gaps[0] - gaps[1]).abs() < 1e-2, "{gaps:?}");
        // 1 つだけなら、置いているパネルの中央へ。
        app.select_only(Some(0));
        app.align(Align::CenterX);
        app.align(Align::CenterY);
        assert!((app.decals[0].at[0] - 0.5).abs() < 1e-3 && (app.decals[0].at[1] - 0.5).abs() < 1e-3, "{:?}", app.decals[0].at);
    }
}
