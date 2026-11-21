//! 型紙が、箱として成り立つかの検査。
//!
//! 形がぴったり閉じるかどうかは、型ごとのテストで確かめる。ここで確かめるのは、その先の
//! 「貼る場所があるか」「閉じているか」「開けられるか」。
//!
//! - 開いたままの辺: 組み上がったとき、2 枚の面の辺が突き合わさるだけで、裏にのりしろも差し込みもフラップもない場所。
//!   中身がこぼれる。離れた場所にある面どうしの折り線が、紙としてつながっていない場合も、ここで見つかる。
//! - 開け口: 開け口が決まっているか。開け口が、何かで留まっているか。

use std::fmt;

use crate::math::{dot, normalize, sub};
use crate::surface::{SeamParts, find_joints, on_edge};
use crate::template::{Hold, P2, Role, Template};

/// 検査で見つかった問題。
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// 2 枚のパネルの辺が突き合わさるだけで、閉じていない。`length` は、閉じていない長さ（mm）。
    OpenJoint { a: usize, b: usize, length: f32 },
    /// 開け口が決まっていない。
    NoOpening,
    /// 開け口が、何にも留められていない。
    OpeningNotHeld { lid: usize },
}

impl Problem {
    /// 人が読める説明。
    pub fn describe(&self, tpl: &Template) -> String {
        let name = |i: usize| tpl.panels[i].name;
        match *self {
            Problem::OpenJoint { a, b, length } => format!("「{}」と「{}」の間が、約 {length:.0} mm 閉じていません。のりしろか差し込みが要ります。", name(a), name(b)),
            Problem::NoOpening => "開け口が決まっていません。".to_owned(),
            Problem::OpeningNotHeld { lid } => format!("開け口の「{}」が、何にも留められていません。差し込みが要ります。", name(lid)),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Problem::OpenJoint { a, b, length } => write!(f, "パネル {a} と {b} の間が {length:.0} mm 閉じていない"),
            Problem::NoOpening => write!(f, "開け口が決まっていない"),
            Problem::OpeningNotHeld { lid } => write!(f, "開け口のパネル {lid} が留められていない"),
        }
    }
}

/// 型紙を検査する。`thin` は、同じ寸法で紙の厚みをほぼ 0 にした型紙（接する辺を見つけるのに使う）。
///
/// 丸めたパネルの、曲がった辺どうしが沿っている場所は、接する辺として見つけられないので、検査の対象外。
pub fn check(tpl: &Template, thin: &Template) -> Vec<Problem> {
    let mut problems = Vec::new();
    let same_shape = tpl.panels.len() == thin.panels.len() && tpl.panels.iter().zip(&thin.panels).all(|(p, q)| p.outline.len() == q.outline.len());
    if same_shape {
        problems.extend(open_joints(tpl, thin));
    }
    if tpl.openings.is_empty() {
        problems.push(Problem::NoOpening);
    }
    problems.extend(tpl.openings.iter().filter(|o| o.hold == Hold::Nothing).map(|o| Problem::OpeningNotHeld { lid: o.lid }));
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
    for joint in find_joints(thin, |panel| structural(panel.role)) {
        // 内側へ折り込まれた面どうしが、箱の中で接しているだけの場所は、箱の口ではない。
        if tpl.panels[joint.a].role == Role::Gusset && tpl.panels[joint.b].role == Role::Gusset {
            continue;
        }
        let SeamParts { pa, ua, pb, ub, open } = joint.open_parts(tpl, &continues);
        // 背中合わせに重なっている 2 枚（貼り合わせた持ち手、屋根の裏に付いた三角形）の縁は、箱の口ではない。
        let thin_parts = joint.open_parts(thin, &vec![Vec::new(); count]);
        if dot(outward(joint.a, thin_parts.pa), outward(joint.b, thin_parts.pb)) < -0.98 {
            continue;
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

    #[test]
    fn prism_lids_are_not_held() {
        // 下書きの角柱: フタに差し込みがない。
        for n in [3usize, 5, 6] {
            let tpl = Template::prism(n, 70.0, 160.0, 0.5);
            let problems = check(&tpl, &Template::prism(n, 70.0, 160.0, 0.02));
            assert_eq!(problems, vec![Problem::OpeningNotHeld { lid: index(&tpl, "上のフタ") }, Problem::OpeningNotHeld { lid: index(&tpl, "底のフタ") }], "{n} 角柱");
        }
    }

    #[test]
    fn pillow_top_edge_is_open_outside_the_handle() {
        // 下書きのピロー型: 上の端のうち、持ち手より外側が閉じていない。
        let (len, height) = (180.0f32, 110.0f32);
        let tpl = Template::pillow(len, height, 0.5, 1.2);
        let problems = check(&tpl, &Template::pillow(len, height, 0.02, 1.2));
        assert_eq!(problems.len(), 1, "{problems:?}");
        let open = open_between(&tpl, &problems, "正面", "背面").expect("上の端が指摘されない");
        // 持ち手は、長さの 27% から 73% まで。残りの 54% が開いている。
        assert!((open - len * 0.54).abs() < 2.0, "開いている長さ {open}");
    }

    #[test]
    fn gable_far_wing_is_not_connected() {
        // 下書きのゲーブル型: 左奥の三角形が、展開図の右端に離れていて、左の三角形とつながっていない。
        let tpl = Template::gable(160.0, 100.0, 110.0, 45.0, 0.5);
        let problems = check(&tpl, &Template::gable(160.0, 100.0, 110.0, 45.0, 0.02));
        assert_eq!(problems.len(), 1, "{:?}", problems.iter().map(|p| p.describe(&tpl)).collect::<Vec<_>>());
        assert!(open_between(&tpl, &problems, "左の三角", "左の折り込み").is_some());
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
