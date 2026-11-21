//! 型紙が、箱として成り立つかの検査。
//!
//! 形がぴったり閉じるかどうかは、型ごとのテストで確かめる。ここで確かめるのは、その先の
//! 「貼る場所があるか」「閉じているか」「開けられるか」。
//!
//! - 開いたままの辺: 組み上がったとき、2 枚の面の辺が突き合わさるだけで、裏にのりしろも差し込みもフラップもない場所。
//!   中身がこぼれる。離れた場所にある面どうしの折り線が、紙としてつながっていない場合も、ここで見つかる。
//! - 開け口: 開け口が決まっているか。開け口が、何かで留まっているか。
//! - 貼る相手: のりしろや貼りしろの、貼る相手が決まっているか。
//! - 展開図: 展開図の上で、パネルどうしが重なっていないか（重なっていたら、1 枚の紙から切り出せない）。

use std::fmt;

use crate::math::{dot, normalize, sub};
use crate::surface::{SeamParts, find_joints, on_edge};
use crate::template::{Hold, P2, Role, Template, inside};

/// 検査で見つかった問題。
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// 2 枚のパネルの辺が突き合わさるだけで、閉じていない。`length` は、閉じていない長さ（mm）。
    OpenJoint { a: usize, b: usize, length: f32 },
    /// 開け口が決まっていない。
    NoOpening,
    /// 開け口が、何にも留められていない。
    OpeningNotHeld { lid: usize },
    /// のりしろや貼りしろの、貼る相手が決まっていない。
    TabNotGlued { tab: usize },
    /// 開け口の留めの形が、型紙にまだ入っていない（箱としては成り立つが、型紙が未完成）。
    LatchNotDrawn { lid: usize },
    /// 展開図の上で、2 枚のパネルが重なっている。
    SheetOverlap { a: usize, b: usize },
}

impl Problem {
    /// 人が読める説明。
    pub fn describe(&self, tpl: &Template) -> String {
        let name = |i: usize| tpl.panels[i].name;
        match *self {
            Problem::OpenJoint { a, b, length } => format!("「{}」と「{}」の間が、約 {length:.0} mm 閉じていません。のりしろか差し込みが要ります。", name(a), name(b)),
            Problem::NoOpening => "開け口が決まっていません。".to_owned(),
            Problem::OpeningNotHeld { lid } => format!("開け口の「{}」が、何にも留められていません。差し込みが要ります。", name(lid)),
            Problem::TabNotGlued { tab } => format!("「{}」を貼る相手が決まっていません。", name(tab)),
            Problem::LatchNotDrawn { lid } => format!("開け口の「{}」の留めの形が、まだ型紙に入っていません。", name(lid)),
            Problem::SheetOverlap { a, b } => format!("展開図の上で、「{}」と「{}」が重なっています。", name(a), name(b)),
        }
    }

    /// 箱として成り立たない問題か。false なら、型紙が未完成というだけの注意。
    pub fn is_blocking(&self) -> bool {
        !matches!(self, Problem::LatchNotDrawn { .. })
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Problem::OpenJoint { a, b, length } => write!(f, "パネル {a} と {b} の間が {length:.0} mm 閉じていない"),
            Problem::NoOpening => write!(f, "開け口が決まっていない"),
            Problem::OpeningNotHeld { lid } => write!(f, "開け口のパネル {lid} が留められていない"),
            Problem::TabNotGlued { tab } => write!(f, "パネル {tab} を貼る相手が決まっていない"),
            Problem::LatchNotDrawn { lid } => write!(f, "開け口のパネル {lid} の留めの形が入っていない"),
            Problem::SheetOverlap { a, b } => write!(f, "展開図の上でパネル {a} と {b} が重なっている"),
        }
    }
}

/// 型紙を検査する。`thin` は、同じ寸法で紙の厚みをほぼ 0 にした型紙（接する辺を見つけるのに使う）。
///
/// 丸めたパネルの、曲がった辺どうしが沿っている場所は、接する辺として見つけられないので、検査の対象外。
pub fn check(tpl: &Template, thin: &Template) -> Vec<Problem> {
    let mut problems = sheet_overlaps(tpl);
    let same_shape = tpl.panels.len() == thin.panels.len() && tpl.panels.iter().zip(&thin.panels).all(|(p, q)| p.outline.len() == q.outline.len());
    if same_shape {
        problems.extend(open_joints(tpl, thin));
    }
    if tpl.openings.is_empty() {
        problems.push(Problem::NoOpening);
    }
    problems.extend(tpl.openings.iter().filter(|o| o.hold == Hold::Nothing).map(|o| Problem::OpeningNotHeld { lid: o.lid }));
    problems.extend(tpl.openings.iter().filter(|o| o.hold == Hold::Latch).map(|o| Problem::LatchNotDrawn { lid: o.lid }));
    let is_glued = |tab: usize| tpl.glued.iter().any(|&(a, b)| a == tab || b == tab);
    problems.extend((0..tpl.panels.len()).filter(|&i| matches!(tpl.panels[i].role, Role::GlueTab | Role::Fin) && !is_glued(i)).map(|tab| Problem::TabNotGlued { tab }));
    problems
}

/// 展開図の上で重なっているパネルの組を見つける。辺を共有しているだけ、角が触れているだけ、は重なりではない。
fn sheet_overlaps(tpl: &Template) -> Vec<Problem> {
    // 重なりとみなす深さ（mm）。これより浅い食い込みは、紙 1 枚ぶんのずらしや数値の誤差。
    const DEEP: f32 = 0.3;
    let edges = |i: usize| {
        let o = &tpl.panels[i].outline;
        (0..o.len()).map(move |k| (o[k], o[(k + 1) % o.len()]))
    };
    let from_edge = |p: P2, (a, b): (P2, P2)| {
        let d = [b[0] - a[0], b[1] - a[1]];
        let s = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-9)).clamp(0.0, 1.0);
        ((p[0] - a[0] - d[0] * s).powi(2) + (p[1] - a[1] - d[1] * s).powi(2)).sqrt()
    };
    // 点が、パネルの輪郭から DEEP より深く内側にあるか。
    let deep_inside = |p: P2, i: usize| inside(&tpl.panels[i].outline, p) && edges(i).all(|e| from_edge(p, e) > DEEP);
    // 辺を少しずつ進んだ点が、相手の深い内側に入っていれば、重なっている。
    let enters = |a: usize, b: usize| {
        edges(a).any(|(p, q)| {
            let steps = ((((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() / 1.0).ceil() as usize).clamp(1, 400);
            (0..=steps).any(|k| {
                let s = k as f32 / steps as f32;
                deep_inside([p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s], b)
            })
        })
    };
    let mut problems = Vec::new();
    for a in 0..tpl.panels.len() {
        for b in a + 1..tpl.panels.len() {
            if enters(a, b) || enters(b, a) {
                problems.push(Problem::SheetOverlap { a, b });
            }
        }
    }
    problems
}

/// 箱の形を作るパネルか（面と、内側へ折り込まれる面）。のりしろ、差し込み、フラップは、すき間をふさぐ側。
fn structural(role: Role) -> bool {
    matches!(role, Role::Face | Role::Gusset)
}

fn open_joints(tpl: &Template, thin: &Template) -> Vec<Problem> {
    // パネルごとに、折り線でつながっている相手との線を、相手の種類で分ける。
    // continues: 箱の形を作るパネルどうし。紙がそのまま続いているので、その部分は「接する辺」ではない。
    // backing: のりしろ、差し込み、フラップ。その線の裏に入って、すき間をふさぐ。
    let count = tpl.panels.len();
    let mut continues: Vec<Vec<[P2; 2]>> = vec![Vec::new(); count];
    let mut backing: Vec<Vec<[P2; 2]>> = vec![Vec::new(); count];
    for (index, panel) in tpl.panels.iter().enumerate() {
        let (parent, lines): (usize, Vec<[P2; 2]>) = match (&panel.hinge, &panel.curved) {
            (Some(hinge), _) => (hinge.parent, vec![[hinge.a, hinge.b]]),
            (None, Some(curved)) => (curved.parent, curved.line.windows(2).map(|w| [w[0], w[1]]).collect()),
            (None, None) => continue,
        };
        match (structural(panel.role), structural(tpl.panels[parent].role)) {
            (true, true) => {
                continues[index].extend(lines.iter().copied());
                continues[parent].extend(lines);
            }
            (false, true) => backing[parent].extend(lines),
            _ => {}
        }
    }

    let places = thin.transforms(1.0, 1.0);
    let outward = |panel: usize, at: P2| normalize(sub(places[panel].point([at[0], at[1], 0.0]), places[panel].point([at[0], at[1], 1.0])));
    let mut problems: Vec<Problem> = Vec::new();
    // 開いた状態でも接している辺の組。開け閉めの動きを持たない型では、閉じた状態と同じになる。
    let when_open = find_joints(thin, 0.0, |panel| structural(panel.role));
    let glued = |a: usize, b: usize| tpl.glued.iter().any(|&pair| pair == (a, b) || pair == (b, a));
    for joint in find_joints(thin, 1.0, |panel| structural(panel.role)) {
        // 内側へ折り込まれた面どうしが、箱の中で接しているだけの場所は、箱の口ではない。
        if tpl.panels[joint.a].role == Role::Gusset && tpl.panels[joint.b].role == Role::Gusset {
            continue;
        }
        let SeamParts { pa, ua, pb, ub, open } = joint.open_parts(tpl, &continues);
        // 背中合わせに重なっている 2 枚の縁。貼り合わせてあるか、開けると離れるもの（ゲーブル型の持ち手）なら、箱の口ではない。
        // 開けても接したままで、貼ってもいないなら（屋根の裏に付く三角形と屋根）、ほかの辺と同じく、裏にのりしろが要る。
        let thin_parts = joint.open_parts(thin, &vec![Vec::new(); count]);
        if dot(outward(joint.a, thin_parts.pa), outward(joint.b, thin_parts.pb)) < -0.98 {
            let stays = when_open.iter().any(|j| (j.a, j.edge_a, j.b, j.edge_b) == (joint.a, joint.edge_a, joint.b, joint.edge_b));
            if glued(joint.a, joint.b) || !stays {
                continue;
            }
        }
        // 開いている区間のうち、裏にのりしろ、差し込み、フラップが入っていない長さ。
        let mut covers = on_edge(pa, ua, 1.0, &backing[joint.a]);
        covers.extend(on_edge(pb, ub, -1.0, &backing[joint.b]));
        let mut bare = 0.0f32;
        for (lo, hi) in open {
            let covered: f32 = covers.iter().map(|&(c0, c1)| (c1.min(hi) - c0.max(lo)).max(0.0)).sum();
            // フラップは、角のぶんだけ辺より少し短い。半分より多くふさがっていれば、閉じているとみなす。
            if covered < (hi - lo) * 0.5 {
                bare += hi - lo - covered.min(hi - lo);
            }
        }
        if bare > 2.0 {
            // 同じ 2 枚の間の問題は、1 つにまとめる。
            match problems.iter_mut().find(|p| matches!(p, Problem::OpenJoint { a, b, .. } if (*a, *b) == (joint.a, joint.b))) {
                Some(Problem::OpenJoint { length, .. }) => *length += bare,
                _ => problems.push(Problem::OpenJoint { a: joint.a, b: joint.b, length: bare }),
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(tpl: &Template, name: &str) -> usize {
        tpl.panels.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("{name} がない"))
    }

    /// 名前で指定した 2 枚の間が開いていると指摘されていれば、その長さ（同じ名前のパネルが複数あってもよい）。
    fn open_between(tpl: &Template, problems: &[Problem], a: &str, b: &str) -> Option<f32> {
        let name = |i: usize| tpl.panels[i].name;
        problems.iter().find_map(|p| match *p {
            Problem::OpenJoint { a: x, b: y, length } if (name(x), name(y)) == (a, b) || (name(x), name(y)) == (b, a) => Some(length),
            _ => None,
        })
    }

    #[test]
    fn tuck_box_passes() {
        // 寸法と厚みを変えても、問題は見つからない。
        for (w, d, h, t) in [(120.0f32, 60.0f32, 160.0f32, 0.5f32), (120.0, 60.0, 160.0, 3.0), (60.0, 40.0, 90.0, 0.4), (220.0, 90.0, 120.0, 1.5)] {
            let tpl = Template::tuck_box(w, d, h, t);
            let problems = check(&tpl, &Template::tuck_box(w, d, h, 0.02));
            assert!(problems.is_empty(), "{w} x {d} x {h}、厚み {t}: {:?}", problems.iter().map(|p| p.describe(&tpl)).collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_tuck_box_without_its_glue_tab_is_caught() {
        // のりしろをフラップ扱いから外す（なくす）と、左側面と背面の間が開いていると指摘される。
        let mut tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        let mut thin = Template::tuck_box(120.0, 60.0, 160.0, 0.02);
        let glue = index(&tpl, "のりしろ");
        for t in [&mut tpl, &mut thin] {
            t.panels[glue].hinge = None;
        }
        let problems = check(&tpl, &thin);
        let open = open_between(&tpl, &problems, "左側面", "背面").expect("のりしろがないのに、指摘されない");
        assert!((open - 160.0).abs() < 2.0, "開いている長さ {open}");
    }

    fn said(tpl: &Template, problems: &[Problem]) -> Vec<String> {
        problems.iter().map(|p| p.describe(tpl)).collect()
    }

    #[test]
    fn prisms_pass() {
        for n in [3usize, 4, 5, 6, 8] {
            for (side, h, t) in [(70.0f32, 160.0f32, 0.5f32), (40.0, 90.0, 0.4), (70.0, 120.0, 2.0)] {
                let tpl = Template::prism(n, side, h, t);
                let problems = check(&tpl, &Template::prism(n, side, h, 0.02));
                assert!(problems.is_empty(), "{n} 角柱、一辺 {side}、厚み {t}: {:?}", said(&tpl, &problems));
            }
        }
    }

    #[test]
    fn a_prism_without_tucks_is_caught() {
        // 差し込みをなくすと、フタと、差し込みを受ける側面の間が開いていると指摘される。
        let mut tpl = Template::prism(6, 70.0, 160.0, 0.5);
        let mut thin = Template::prism(6, 70.0, 160.0, 0.02);
        for t in [&mut tpl, &mut thin] {
            for panel in t.panels.iter_mut().filter(|p| p.role == Role::Tuck) {
                panel.hinge = None;
            }
        }
        let problems = check(&tpl, &thin);
        assert!(open_between(&tpl, &problems, "上のフタ", "側面 4").is_some(), "{:?}", said(&tpl, &problems));
        assert!(open_between(&tpl, &problems, "底のフタ", "側面 4").is_some());
    }

    #[test]
    fn pillows_pass() {
        for (len, height, t, bulge) in [(180.0f32, 110.0f32, 0.5f32, 1.2f32), (110.0, 170.0, 0.4, 1.0), (260.0, 80.0, 0.5, 1.4), (180.0, 110.0, 2.0, 0.5)] {
            let tpl = Template::pillow(len, height, t, bulge);
            let problems = check(&tpl, &Template::pillow(len, height, 0.02, bulge));
            assert!(problems.is_empty(), "{len} x {height}、ふくらみ {bulge}: {:?}", said(&tpl, &problems));
        }
    }

    #[test]
    fn a_pillow_without_its_fins_is_caught() {
        // 貼りしろをなくすと、上の端が、端から端まで開いていると指摘される。
        let (len, height) = (180.0f32, 110.0f32);
        let mut tpl = Template::pillow(len, height, 0.5, 1.2);
        let mut thin = Template::pillow(len, height, 0.02, 1.2);
        for t in [&mut tpl, &mut thin] {
            for panel in t.panels.iter_mut().filter(|p| p.role == Role::Fin) {
                panel.hinge = None;
            }
        }
        let problems = check(&tpl, &thin);
        let open = open_between(&tpl, &problems, "正面", "背面").expect("上の端が指摘されない");
        assert!((open - len).abs() < 4.0, "開いている長さ {open}");
    }

    #[test]
    fn gables_pass_except_for_the_undrawn_latch() {
        for (w, d, h, roof, t) in [(160.0f32, 100.0f32, 110.0f32, 45.0f32, 0.5f32), (120.0, 80.0, 90.0, 60.0, 0.4), (200.0, 120.0, 140.0, 40.0, 1.5)] {
            let tpl = Template::gable(w, d, h, roof, t);
            let problems = check(&tpl, &Template::gable(w, d, h, roof, 0.02));
            // 箱としては成り立つ。持ち手の留めの形だけが、まだ入っていない。
            assert!(problems.iter().all(|p| !p.is_blocking()), "{w} x {d} x {h}: {:?}", said(&tpl, &problems));
            assert_eq!(problems, vec![Problem::LatchNotDrawn { lid: index(&tpl, "正面の屋根") }]);
        }
    }

    #[test]
    fn a_gable_without_its_roof_glue_tab_is_caught() {
        // 屋根ののりしろをなくすと、左奥の折り込みと背面の屋根の間が、つながっていないと指摘される。
        let mut tpl = Template::gable(160.0, 100.0, 110.0, 45.0, 0.5);
        let mut thin = Template::gable(160.0, 100.0, 110.0, 45.0, 0.02);
        let tab = index(&tpl, "屋根ののりしろ");
        for t in [&mut tpl, &mut thin] {
            t.panels[tab].hinge = None;
        }
        let problems = check(&tpl, &thin);
        assert!(open_between(&tpl, &problems, "左の折り込み", "背面の屋根").is_some(), "{:?}", said(&tpl, &problems));
    }

    #[test]
    fn overlapping_panels_on_the_sheet_are_caught() {
        // 上のフタを、展開図の上で正面に重なる位置へずらす。
        let mut tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        let (lid, front) = (index(&tpl, "上のフタ"), index(&tpl, "正面"));
        let shift = tpl.panels[front].outline[0][0] - tpl.panels[lid].outline[0][0];
        for p in &mut tpl.panels[lid].outline {
            *p = [p[0] + shift, p[1] - 40.0];
        }
        assert!(sheet_overlaps(&tpl).contains(&Problem::SheetOverlap { a: front, b: lid }));
    }

    #[test]
    fn tabs_must_say_what_they_are_glued_to() {
        let mut tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        tpl.glued.clear();
        let problems = check(&tpl, &Template::tuck_box(120.0, 60.0, 160.0, 0.02));
        assert_eq!(problems, vec![Problem::TabNotGlued { tab: index(&tpl, "のりしろ") }]);
    }

    #[test]
    fn carry_bag_has_no_opening_and_an_open_top() {
        // 下書きの手提げ型: 開け口が決まっておらず、上の端の持ち手より外側が閉じていない。
        let tpl = Template::carry_bag(150.0, 60.0, 160.0, 0.5, 0.7);
        let problems = check(&tpl, &Template::carry_bag(150.0, 60.0, 160.0, 0.02, 0.7));
        assert!(problems.contains(&Problem::NoOpening));
        assert!(open_between(&tpl, &problems, "正面", "背面").is_some());
    }
}
