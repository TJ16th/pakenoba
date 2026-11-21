//! 面のつながりと、立体の上で置く柄。
//!
//! 柄は、展開図まるごとの絵とは別に、立体の上で置く部品（ロゴ、帯、画像）。
//!
//! - 柄は「どのパネルの、どのあたりにあるか」で持つ。寸法を変えて型紙を作り直しても、同じパネルの同じあたりに残る。
//! - 柄がパネルの外へはみ出すと、つながっている隣のパネルへ続く。つながり方は 2 種類。
//!   折り線: 展開図の上でも地続きなので、そのまま続く。
//!   合わせ目: 組み上がると接するが、展開図の上では離れている辺。隣のパネルの座標へ、回転と移動で写して続ける。
//! - 合わせ目は、組み上がった形から自動で見つける。

use crate::math::{Affine, V3, add, cross, dot, length, normalize, scale, sub};
use crate::template::{P2, Panel, Template, Vertex, inside};

/// 展開図の上での回転と移動（裏返しは含まない）。
#[derive(Clone, Copy, Debug)]
pub struct Aff2 {
    pub cos: f32,
    pub sin: f32,
    pub t: P2,
}

impl Aff2 {
    pub const IDENTITY: Aff2 = Aff2 { cos: 1.0, sin: 0.0, t: [0.0, 0.0] };

    pub fn new(angle: f32, t: P2) -> Aff2 {
        Aff2 { cos: angle.cos(), sin: angle.sin(), t }
    }

    pub fn vector(&self, v: P2) -> P2 {
        [self.cos * v[0] - self.sin * v[1], self.sin * v[0] + self.cos * v[1]]
    }

    pub fn point(&self, p: P2) -> P2 {
        let v = self.vector(p);
        [v[0] + self.t[0], v[1] + self.t[1]]
    }

    /// `self` を後に適用する合成。
    pub fn then_after(&self, inner: &Aff2) -> Aff2 {
        Aff2 { cos: self.cos * inner.cos - self.sin * inner.sin, sin: self.sin * inner.cos + self.cos * inner.sin, t: self.point(inner.t) }
    }

    pub fn inverse(&self) -> Aff2 {
        let back = Aff2 { cos: self.cos, sin: -self.sin, t: [0.0, 0.0] };
        let t = back.vector(self.t);
        Aff2 { t: [-t[0], -t[1]], ..back }
    }

    pub fn angle(&self) -> f32 {
        self.sin.atan2(self.cos)
    }
}

fn sub2(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn along(p: P2, dir: P2, s: f32) -> P2 {
    [p[0] + dir[0] * s, p[1] + dir[1] * s]
}

fn len2(a: P2) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

fn edge(tpl: &Template, panel: usize, index: usize) -> (P2, P2) {
    let outline = &tpl.panels[panel].outline;
    (outline[index], outline[(index + 1) % outline.len()])
}

/// 合わせ目。組み上がると接する、2 枚のパネルの辺の組。
#[derive(Clone, Copy, Debug)]
pub struct Seam {
    /// 1 枚目のパネルと、その辺の番号（頂点の番号と同じ。その頂点から次の頂点まで）。
    pub a: usize,
    pub edge_a: usize,
    pub b: usize,
    pub edge_b: usize,
    /// 接している区間の中央が、それぞれの辺のどこにあるか（0〜1）。
    at_a: f32,
    at_b: f32,
    /// 接している区間の、半分の長さ（mm）。
    half: f32,
}

/// 組み上がった形から、合わせ目を見つける。
///
/// `thin` は、同じ寸法で紙の厚みをほぼ 0 にした型紙。厚みがあると、接する辺どうしが紙 1 枚ぶん離れて、
/// 見分けにくくなるため。辺の番号は厚みを変えても同じなので、見つけた組は元の型紙にそのまま使える。
pub fn find_seams(thin: &Template) -> Vec<Seam> {
    find_joints(thin, 1.0, |panel| !panel.hidden_when_closed())
}

/// 組み上がった形から、`include` で選んだパネルどうしの、接する辺の組を見つける。
/// `closing` は開け閉め（0 が開いた状態、1 が閉じた状態）。
pub fn find_joints(thin: &Template, closing: f32, include: impl Fn(&Panel) -> bool) -> Vec<Seam> {
    const NEAR: f32 = 0.4;
    let places = thin.transforms(1.0, closing);
    let visible: Vec<usize> = (0..thin.panels.len()).filter(|&i| include(&thin.panels[i])).collect();
    let corners: Vec<Vec<V3>> = thin.panels.iter().zip(&places).map(|(panel, place)| panel.outline.iter().map(|p| place.point([p[0], p[1], 0.0])).collect()).collect();
    let mut seams = Vec::new();
    for (n, &a) in visible.iter().enumerate() {
        for &b in &visible[n + 1..] {
            for edge_a in 0..corners[a].len() {
                for edge_b in 0..corners[b].len() {
                    let (p0, p1) = edge(thin, a, edge_a);
                    let (q0, q1) = edge(thin, b, edge_b);
                    let (sheet_a, sheet_b) = (len2(sub2(p1, p0)), len2(sub2(q1, q0)));
                    if sheet_a < 1.0 || sheet_b < 1.0 {
                        continue;
                    }
                    let (ua, ub) = ([(p1[0] - p0[0]) / sheet_a, (p1[1] - p0[1]) / sheet_a], [(q1[0] - q0[0]) / sheet_b, (q1[1] - q0[1]) / sheet_b]);
                    // 展開図の上ですでに同じ線に乗っている辺は、折り線でつながった隣どうし。合わせ目ではない。
                    let off_line = |q: P2| ((q[0] - p0[0]) * ua[1] - (q[1] - p0[1]) * ua[0]).abs();
                    let along_a = |q: P2| (q[0] - p0[0]) * ua[0] + (q[1] - p0[1]) * ua[1];
                    let shared = along_a(q0).max(along_a(q1)).min(sheet_a) - along_a(q0).min(along_a(q1)).max(0.0);
                    if off_line(q0) < 0.05 && off_line(q1) < 0.05 && shared > 0.5 {
                        continue;
                    }
                    // 立体の中で、同じ線の上に、逆向きに並んでいるか。
                    let (c0, c1) = (corners[a][edge_a], corners[a][(edge_a + 1) % corners[a].len()]);
                    let (d0, d1) = (corners[b][edge_b], corners[b][(edge_b + 1) % corners[b].len()]);
                    let (la, lb) = (length(sub(c1, c0)), length(sub(d1, d0)));
                    if la < 1.0 || lb < 1.0 {
                        continue;
                    }
                    let (da, db) = (scale(sub(c1, c0), 1.0 / la), scale(sub(d1, d0), 1.0 / lb));
                    if dot(da, db) > -0.995 {
                        continue;
                    }
                    let from_line = |p: V3| length(sub(sub(p, c0), scale(da, dot(sub(p, c0), da))));
                    if from_line(d0) > NEAR || from_line(d1) > NEAR {
                        continue;
                    }
                    let (s0, s1) = (dot(sub(d0, c0), da), dot(sub(d1, c0), da));
                    let (lo, hi) = (s0.min(s1).max(0.0), s0.max(s1).min(la));
                    if hi - lo < 2.0 {
                        continue;
                    }
                    let mid = (lo + hi) * 0.5;
                    let seam = Seam { a, edge_a, b, edge_b, at_a: mid / la, at_b: dot(sub(add(c0, scale(da, mid)), d0), db) / lb, half: (hi - lo) * 0.5 };
                    // 念のため、区間の両端近くと中央で、展開図上の対応する点どうしが立体の中で重なるかを確かめる
                    // （丸めたパネルの辺は、端だけ合っていて途中が離れていることがある）。
                    let (pa, pb) = (along(p0, ua, seam.at_a * sheet_a), along(q0, ub, seam.at_b * sheet_b));
                    let touching = [-0.9 * seam.half, 0.0, 0.9 * seam.half].iter().all(|&x| {
                        let (on_a, on_b) = (along(pa, ua, x), along(pb, ub, -x));
                        length(sub(places[a].point([on_a[0], on_a[1], 0.0]), places[b].point([on_b[0], on_b[1], 0.0]))) < NEAR * 2.0
                    });
                    if touching {
                        seams.push(seam);
                    }
                }
            }
        }
    }
    seams
}

/// 合わせ目の、型紙の上での位置。
pub struct SeamParts {
    /// 接している区間の中央（a の辺の上と、b の辺の上）と、それぞれの辺の向き。
    pub pa: P2,
    pub ua: P2,
    pub pb: P2,
    pub ub: P2,
    /// 接している区間のうち、紙が折り線で別の隣へ続いていない部分。
    /// a の辺に沿った、中央からの距離の範囲で表す（b の辺は逆向きなので、符号が逆になる）。
    pub open: Vec<(f32, f32)>,
}

impl Seam {
    /// 接している区間の長さ（mm）。
    pub fn length(&self) -> f32 {
        self.half * 2.0
    }

    /// 型紙の上での位置と、開いている区間を求める。
    /// `joined` は、パネルごとの「紙が隣へ続いている線」（折り線）。その線に乗っている部分は、区間から除く。
    pub fn open_parts(&self, tpl: &Template, joined: &[Vec<[P2; 2]>]) -> SeamParts {
        let (p0, p1) = edge(tpl, self.a, self.edge_a);
        let (q0, q1) = edge(tpl, self.b, self.edge_b);
        let (la, lb) = (len2(sub2(p1, p0)).max(1e-6), len2(sub2(q1, q0)).max(1e-6));
        let (ua, ub) = ([(p1[0] - p0[0]) / la, (p1[1] - p0[1]) / la], [(q1[0] - q0[0]) / lb, (q1[1] - q0[1]) / lb]);
        let (pa, pb) = (along(p0, ua, self.at_a * la), along(q0, ub, self.at_b * lb));
        let mut blocked = on_edge(pa, ua, 1.0, &joined[self.a]);
        blocked.extend(on_edge(pb, ub, -1.0, &joined[self.b]));
        blocked.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut open: Vec<(f32, f32)> = Vec::new();
        let mut from = -self.half;
        for (lo, hi) in blocked {
            if lo - from > 2.0 {
                open.push((from, lo.min(self.half)));
            }
            from = from.max(hi);
        }
        if self.half - from > 2.0 {
            open.push((from, self.half));
        }
        SeamParts { pa, ua, pb, ub, open }
    }
}

/// `lines` のうち、点 `origin` を通る向き `dir` の線に乗っているものを、その線に沿った位置の範囲にする。
pub fn on_edge(origin: P2, dir: P2, sign: f32, lines: &[[P2; 2]]) -> Vec<(f32, f32)> {
    let off = |p: P2| ((p[0] - origin[0]) * dir[1] - (p[1] - origin[1]) * dir[0]).abs();
    let at = |p: P2| sign * ((p[0] - origin[0]) * dir[0] + (p[1] - origin[1]) * dir[1]);
    lines.iter().filter(|l| off(l[0]) < 0.05 && off(l[1]) < 0.05).map(|l| (at(l[0]).min(at(l[1])), at(l[0]).max(at(l[1])))).collect()
}

/// あるパネルから隣のパネルへの、つながり。
struct Link {
    to: usize,
    /// こちらのパネルの座標を、隣のパネルの座標へ写す。折り線なら何もしない。
    map: Aff2,
    /// 境目の線（こちらのパネルの座標）。柄がこの線にかかっていれば、隣へ続く。
    gate: Vec<[P2; 2]>,
    seam: bool,
}

/// 柄がたどれる、面のつながり（折り線と合わせ目）。
pub struct Surface {
    links: Vec<Vec<Link>>,
    pub seams: Vec<Seam>,
    /// パネルごとの「上」の向き（`Decal::angle` と同じ測り方。この角度の柄は、その面でまっすぐ立って見える）。
    /// 1 つめが紙の表（箱の外側）、2 つめが紙の裏（箱の内側）。
    up: Vec<[f32; 2]>,
}

/// 組み上がった箱での、パネルごとの「上」の向き。紙の表（箱の外側）と、裏（箱の内側）のそれぞれ。
///
/// - 立っている面と、斜めの面（屋根など）: 箱の上へ向かう向き。表も裏も同じ。
/// - 上を向いた面（天面の外側、底の内側）: 奥へ向かう向き。正面から見下ろして、まっすぐ読める。
/// - 下を向いた面（底面の外側、フタの裏）: 手前へ向かう向き。箱を手前へ起こして、まっすぐ読める。
///   フタの裏では、フタを奥へ開けて立てたときに、上になる向きでもある。
///
/// 水平な面では、表と裏で、向いている側が逆になる。だから「上」も逆になる。
/// 丸めたパネルでは、パネルの中央での向きで決める。
fn face_ups(tpl: &Template) -> Vec<[f32; 2]> {
    let world = tpl.world_at(1.0);
    let frames: Vec<Affine> = tpl
        .transforms(1.0, 1.0)
        .iter()
        .enumerate()
        .map(|(index, place)| {
            let (lo, hi) = panel_box(tpl, index);
            world.then_after(&place.at([(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5]))
        })
        .collect();
    let centers: Vec<V3> = frames.iter().enumerate().map(|(index, frame)| {
        let (lo, hi) = panel_box(tpl, index);
        frame.point([(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, 0.0])
    }).collect();
    let middle = centers.iter().fold(0.0, |sum, c| sum + c[1]) / centers.len().max(1) as f32;
    frames
        .iter()
        .zip(&centers)
        .map(|(frame, center)| {
            let (ex, ey) = (frame.vector([1.0, 0.0, 0.0]), frame.vector([0.0, 1.0, 0.0]));
            let angle = |u: P2| if len2(u) < 1e-3 { 0.0 } else { (-u[0]).atan2(u[1]) };
            // 箱の上（Y）を、面の中へ映す。ほぼ水平な面では向きが決まらないので、奥か手前を使う。
            let u = [ex[1], ey[1]];
            if len2(u) >= 0.35 {
                return [angle(u); 2];
            }
            // 箱の上半分にある水平な面は、外側が上を向き、内側が下を向いている。下半分は、その逆。
            let depth = if center[1] >= middle { -1.0 } else { 1.0 };
            [angle([ex[2] * depth, ey[2] * depth]), angle([-ex[2] * depth, -ey[2] * depth])]
        })
        .collect()
}

/// 柄の、パネル 1 枚ぶんの置かれ方。
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub panel: usize,
    /// 柄の座標（中心が原点、mm）を、展開図の座標へ写す。
    pub place: Aff2,
    /// ここへ来るまでに、合わせ目をまたいだ回数。
    pub seams: usize,
}

impl Surface {
    /// `thin` は、同じ寸法で厚みをほぼ 0 にした型紙（合わせ目を見つけるのに使う）。
    pub fn new(tpl: &Template, thin: &Template) -> Surface {
        let mut links: Vec<Vec<Link>> = (0..tpl.panels.len()).map(|_| Vec::new()).collect();
        let visible = |i: usize| !tpl.panels[i].hidden_when_closed();
        // 折り線（まっすぐなものと、曲線のもの）。展開図の上で地続きなので、座標はそのまま。
        for (i, panel) in tpl.panels.iter().enumerate() {
            let (parent, gate): (usize, Vec<[P2; 2]>) = match (&panel.hinge, &panel.curved) {
                (Some(hinge), _) => (hinge.parent, vec![[hinge.a, hinge.b]]),
                (None, Some(curved)) => (curved.parent, curved.line.windows(2).map(|w| [w[0], w[1]]).collect()),
                (None, None) => continue,
            };
            if visible(i) && visible(parent) {
                links[i].push(Link { to: parent, map: Aff2::IDENTITY, gate: gate.clone(), seam: false });
                links[parent].push(Link { to: i, map: Aff2::IDENTITY, gate, seam: false });
            }
        }
        // パネルごとの、見えている隣とつながっている線（折り線）。ここでは紙が隣へ続いているので、合わせ目にはならない。
        let mut joined: Vec<Vec<[P2; 2]>> = (0..tpl.panels.len()).map(|_| Vec::new()).collect();
        for (from, list) in links.iter().enumerate() {
            joined[from] = list.iter().flat_map(|link| link.gate.iter().copied()).collect();
        }
        // 合わせ目。辺の数が合わないときは（型紙の作りが厚みで変わる場合）、使わない。
        let same_shape = tpl.panels.len() == thin.panels.len() && tpl.panels.iter().zip(&thin.panels).all(|(p, q)| p.outline.len() == q.outline.len());
        let found = if same_shape { find_seams(thin) } else { Vec::new() };
        let mut seams = Vec::new();
        for seam in &found {
            let SeamParts { pa, ua, pb, ub, open } = seam.open_parts(tpl, &joined);
            if open.is_empty() {
                continue;
            }
            // 辺どうしは逆向きに重なる。a の辺の向きを、b の辺の逆向きへ回す。
            let turn = Aff2::new((-ub[1]).atan2(-ub[0]) - ua[1].atan2(ua[0]), [0.0, 0.0]);
            let moved = turn.vector(pa);
            let map = Aff2 { t: [pb[0] - moved[0], pb[1] - moved[1]], ..turn };
            links[seam.a].push(Link { to: seam.b, map, gate: open.iter().map(|&(lo, hi)| [along(pa, ua, lo), along(pa, ua, hi)]).collect(), seam: true });
            links[seam.b].push(Link { to: seam.a, map: map.inverse(), gate: open.iter().map(|&(lo, hi)| [along(pb, ub, -hi), along(pb, ub, -lo)]).collect(), seam: true });
            seams.push(*seam);
        }
        Surface { links, seams, up: face_ups(tpl) }
    }

    /// 柄を、かかっているパネルごとに割り振る。最初の 1 つが、柄の置かれているパネル。
    ///
    /// 置かれているパネルから、境目の線にかかっている隣へ、近い順にたどる。
    /// 同じパネルへ別の道から着いたときは、ずれて重なるので、先に着いたほうだけを使う
    /// （3 枚の面が集まる角。平らに開けないので、どこかで柄が切れる）。
    /// ただし、向きが同じで位置だけが違うときは、両方とも使う（箱を一周して戻ってきた帯）。
    pub fn layout(&self, tpl: &Template, decal: &Decal) -> Vec<Piece> {
        let half = [decal.size[0] * 0.5, decal.size[1] * 0.5];
        let mut pieces = vec![Piece { panel: decal.panel, place: decal.place(tpl), seams: 0 }];
        let mut next = 0;
        while next < pieces.len() && pieces.len() < 48 {
            let from = pieces[next];
            next += 1;
            let to_decal = from.place.inverse();
            for link in &self.links[from.panel] {
                // 合わせ目は、続けて 2 回はまたがない。合わせ目の先の面から、さらに別の合わせ目へ進むと、
                // 箱の縁にかかった長い帯が、フタを伝って側面や底へ回り込み、同じ面に何本も出てしまう。
                // （3 つの面が集まる角で、どの面のつながりを優先するかを選べるようにするまでの決まり。）
                if link.seam && from.seams >= 1 {
                    continue;
                }
                if !link.gate.iter().any(|g| segment_hits_box(to_decal.point(g[0]), to_decal.point(g[1]), half)) {
                    continue;
                }
                let place = link.map.then_after(&from.place);
                let clash = pieces.iter().filter(|p| p.panel == link.to).any(|p| {
                    let between = p.place.inverse().then_after(&place);
                    let shifted_only = between.angle().abs() < 0.01 && len2(between.t) > 1.0;
                    boxes_overlap(&between, half) && !shifted_only
                });
                if !clash {
                    pieces.push(Piece { panel: link.to, place, seams: from.seams + link.seam as usize });
                }
            }
        }
        pieces
    }

    /// 展開図上の点に、どの柄があるか。上に重なっているものを優先する。柄の番号と、柄の座標での位置を返す。
    pub fn decal_at(&self, tpl: &Template, decals: &[Decal], panel: usize, at: P2) -> Option<(usize, P2)> {
        decals.iter().enumerate().rev().find_map(|(index, decal)| {
            self.layout(tpl, decal).iter().filter(|piece| piece.panel == panel).find_map(|piece| {
                let q = piece.place.inverse().point(at);
                (q[0].abs() <= decal.size[0] * 0.5 && q[1].abs() <= decal.size[1] * 0.5).then_some((index, q))
            })
        })
    }

    /// そのパネルの「上」の向き（`Decal::angle` と同じ測り方）。`back` が真なら、紙の裏（箱の内側の面）での向き。
    pub fn up(&self, panel: usize, back: bool) -> f32 {
        self.up.get(panel).map_or(0.0, |up| up[back as usize])
    }

    /// つかんだ柄を、立体の上の別の点へ動かす。`grab` は、柄の座標でのつかんだ位置。
    ///
    /// 向きは、面ごとの「上」を基準にして保つ。正面でまっすぐ立っている柄は、側面でも天面でも、まっすぐ立つ。
    /// どの面をどの順にたどっても、同じ面に戻れば、同じ向きに戻る。
    ///
    /// 3 つの面が集まる角のまわりでは、絵をどの辺でも途切れさせずに運ぶと、一周で 90 度回ってしまう
    /// （平らに開けない角だから）。それを避けるかわりに、側面から天面へ移るときのように、
    /// 面の「上」どうしが続いていない辺では、またいだところで向きが切り替わる。
    ///
    /// `back` が真なら、紙の裏（箱の内側の面）に置いた柄。内側の面の「上」を使う。
    pub fn drag_on_solid(&self, tpl: &Template, decal: &mut Decal, back: bool, panel: usize, at: P2, grab: P2) {
        if tpl.panels[panel].hidden_when_closed() {
            return;
        }
        let angle = decal.angle - self.up(decal.panel, back) + self.up(panel, back);
        // 行ったり来たりしても値が少しずつずれないように、-180〜180 度に収める。
        let angle = (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let angle = if panel == decal.panel { decal.angle } else { angle };
        let offset = Aff2::new(angle, [0.0, 0.0]).vector(grab);
        decal.panel = panel;
        decal.angle = angle;
        decal.set_center(tpl, [at[0] - offset[0], at[1] - offset[1]]);
    }

    /// つかんだ柄を、展開図の上の別の点へ動かす。`grab` は、柄の座標でのつかんだ位置。
    ///
    /// 行き先のパネルに、柄がすでにかかっていれば、その続き方のまま（合わせ目で回った向きのまま）置き直す。
    /// 展開図の上では、動かしても絵は回らない。
    pub fn drag(&self, tpl: &Template, decal: &mut Decal, panel: usize, at: P2, grab: P2) {
        if tpl.panels[panel].hidden_when_closed() {
            return;
        }
        let pieces = self.layout(tpl, decal);
        let contains = |piece: &&Piece| {
            let q = piece.place.inverse().point(at);
            q[0].abs() <= decal.size[0] * 0.5 + 2.0 && q[1].abs() <= decal.size[1] * 0.5 + 2.0
        };
        let on_panel = || pieces.iter().filter(|piece| piece.panel == panel);
        let angle = on_panel().find(contains).or_else(|| on_panel().next()).map_or(decal.angle, |piece| piece.place.angle());
        let offset = Aff2::new(angle, [0.0, 0.0]).vector(grab);
        decal.panel = panel;
        decal.angle = angle;
        decal.set_center(tpl, [at[0] - offset[0], at[1] - offset[1]]);
    }
}

/// 線分が、原点を中心にした四角（半分の幅と高さが `half`）にかかるか。
fn segment_hits_box(a: P2, b: P2, half: [f32; 2]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..2 {
        let d = b[i] - a[i];
        if d.abs() < 1e-9 {
            if a[i].abs() > half[i] {
                return false;
            }
            continue;
        }
        let (near, far) = ((-half[i] - a[i]) / d, (half[i] - a[i]) / d);
        t0 = t0.max(near.min(far));
        t1 = t1.min(near.max(far));
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// 同じ大きさの四角 2 つ（片方は原点、もう片方は `between` で動かしたもの）が重なるか。
fn boxes_overlap(between: &Aff2, half: [f32; 2]) -> bool {
    // わずかに小さくして比べる（ちょうど端どうしが接するだけなら、重なりとしない）。
    let half = [half[0] * 0.98, half[1] * 0.98];
    let (c, s) = (between.cos.abs(), between.sin.abs());
    let reach = [half[0] * c + half[1] * s, half[0] * s + half[1] * c];
    let center = between.t;
    let turned = [center[0] * between.cos + center[1] * between.sin, -center[0] * between.sin + center[1] * between.cos];
    center[0].abs() <= half[0] + reach[0] && center[1].abs() <= half[1] + reach[1] && turned[0].abs() <= half[0] + reach[0] && turned[1].abs() <= half[1] + reach[1]
}

/// 立体の上に置く柄の、場所と大きさ。絵そのものは、ここでは持たない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decal {
    /// 置かれているパネル。
    pub panel: usize,
    /// パネルを囲む四角の中での、中心の位置（左下が 0, 0、右上が 1, 1）。
    /// 寸法を変えても、パネルの中での相対的な位置が保たれる。
    pub at: P2,
    /// 幅と高さ（mm）。寸法を変えても、大きさは変わらない。
    pub size: P2,
    /// 回転（ラジアン、反時計回り）。置かれているパネルの、展開図での向きが基準。
    pub angle: f32,
}

fn panel_box(tpl: &Template, panel: usize) -> (P2, P2) {
    let outline = &tpl.panels[panel].outline;
    let lo = outline.iter().fold([f32::INFINITY; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
    let hi = outline.iter().fold([f32::NEG_INFINITY; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
    (lo, hi)
}

impl Decal {
    /// 中心の、展開図上の位置。
    pub fn center(&self, tpl: &Template) -> P2 {
        let (lo, hi) = panel_box(tpl, self.panel);
        [lo[0] + self.at[0] * (hi[0] - lo[0]), lo[1] + self.at[1] * (hi[1] - lo[1])]
    }

    pub fn set_center(&mut self, tpl: &Template, center: P2) {
        let (lo, hi) = panel_box(tpl, self.panel);
        self.at = [(center[0] - lo[0]) / (hi[0] - lo[0]).max(1e-3), (center[1] - lo[1]) / (hi[1] - lo[1]).max(1e-3)];
    }

    fn place(&self, tpl: &Template) -> Aff2 {
        Aff2::new(self.angle, self.center(tpl))
    }
}

/// 展開図上の点が、どのパネルにあるか（組み上がると隠れるパネルは除く）。
/// 折り線のすき間を埋める面のように、輪郭のちょうど上にある点は、いちばん近いパネルにする。
pub fn panel_at(tpl: &Template, at: P2) -> Option<usize> {
    let visible = || tpl.panels.iter().enumerate().filter(|(_, panel)| !panel.hidden_when_closed());
    if let Some((index, _)) = visible().find(|(_, panel)| inside(&panel.outline, at)) {
        return Some(index);
    }
    let distance = |outline: &[P2]| {
        (0..outline.len())
            .map(|i| {
                let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
                let d = sub2(b, a);
                let s = (((at[0] - a[0]) * d[0] + (at[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-9)).clamp(0.0, 1.0);
                len2(sub2(at, along(a, d, s)))
            })
            .fold(f32::INFINITY, f32::min)
    };
    visible().map(|(index, panel)| (index, distance(&panel.outline))).filter(|&(_, d)| d < 1.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| index)
}

/// 視線（`origin` から `dir` の向き）が最初に当たる、立体の上の点。パネルの番号と、展開図上の位置を返す。
///
/// 当たったのが紙の内側や断面なら None（印刷面にしか柄は置けない）。抜き穴は素通りする。
pub fn pick(tpl: &Template, mesh: &[Vertex], origin: V3, dir: V3) -> Option<(usize, P2)> {
    pick_side(tpl, mesh, origin, dir).filter(|&(_, _, back)| !back).map(|(panel, at, _)| (panel, at))
}

/// `pick` と同じだが、紙の裏（箱の内側の面）に当たったときも返す。3 つめが、裏かどうか。
/// 断面に当たったときは None。
pub fn pick_side(tpl: &Template, mesh: &[Vertex], origin: V3, dir: V3) -> Option<(usize, P2, bool)> {
    let mut hits: Vec<(f32, f32, P2)> = Vec::new();
    for tri in mesh.chunks_exact(3) {
        let (e1, e2) = (sub(tri[1].pos, tri[0].pos), sub(tri[2].pos, tri[0].pos));
        let p = cross(dir, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-9 {
            continue;
        }
        let s = sub(origin, tri[0].pos);
        let u = dot(s, p) / det;
        let q = cross(s, e1);
        let v = dot(dir, q) / det;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            continue;
        }
        let distance = dot(e2, q) / det;
        if distance <= 1e-4 {
            continue;
        }
        let uv = [tri[0].uv[0] * (1.0 - u - v) + tri[1].uv[0] * u + tri[2].uv[0] * v, tri[0].uv[1] * (1.0 - u - v) + tri[1].uv[1] * u + tri[2].uv[1] * v];
        let at = [tpl.min[0] + uv[0] * (tpl.max[0] - tpl.min[0]), tpl.max[1] - uv[1] * (tpl.max[1] - tpl.min[1])];
        hits.push((distance, tri[0].kind, at));
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (_, kind, at) = hits.into_iter().find(|&(_, _, at)| !tpl.in_hole(at[0], at[1]))?;
    if kind > 1.5 {
        return None;
    }
    panel_at(tpl, at).map(|panel| (panel, at, kind > 0.5))
}

/// 視点とカメラの向きから、画面上の点（-1〜1、右と上が正）を通る視線の向きを求める。
pub fn ray_through(forward: V3, right: V3, up: V3, fov_y: f32, aspect: f32, x: f32, y: f32) -> V3 {
    let reach = (fov_y * 0.5).tan();
    normalize(add(forward, add(scale(right, x * reach * aspect), scale(up, y * reach))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 120.0;
    const D: f32 = 60.0;
    const H: f32 = 160.0;

    fn tuck(t: f32) -> (Template, Surface) {
        let tpl = Template::tuck_box(W, D, H, t);
        let surface = Surface::new(&tpl, &Template::tuck_box(W, D, H, 0.02));
        (tpl, surface)
    }

    fn index(tpl: &Template, name: &str) -> usize {
        tpl.panels.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("{name} がない"))
    }

    fn has_seam(tpl: &Template, surface: &Surface, a: &str, b: &str) -> bool {
        let (a, b) = (index(tpl, a), index(tpl, b));
        surface.seams.iter().any(|s| (s.a == a && s.b == b) || (s.a == b && s.b == a))
    }

    #[test]
    fn seams_of_a_tuck_box_are_found() {
        for t in [0.5f32, 3.0] {
            let (tpl, surface) = tuck(t);
            // のりしろの合わせ目と、フタのまわり 3 辺（上と底）。
            for (a, b) in [("背面", "左側面"), ("上のフタ", "正面"), ("上のフタ", "左側面"), ("上のフタ", "右側面"), ("底のフタ", "正面"), ("底のフタ", "左側面"), ("底のフタ", "右側面")] {
                assert!(has_seam(&tpl, &surface, a, b), "厚み {t}: {a} と {b} の合わせ目が見つからない");
            }
            assert_eq!(surface.seams.len(), 7, "厚み {t}: 合わせ目の数が違う");
        }
    }

    #[test]
    fn points_on_a_seam_meet_on_the_box() {
        // 合わせ目で写した先の点は、立体の中で元の点と（紙の厚みの範囲で）同じ場所に来る。
        for t in [0.5f32, 3.0] {
            let (tpl, surface) = tuck(t);
            let places = tpl.transforms(1.0, 1.0);
            for (from, links) in surface.links.iter().enumerate() {
                for link in links.iter().filter(|l| l.seam) {
                    assert_eq!(link.gate.len(), 1);
                    for s in [0.1f32, 0.5, 0.9] {
                        let on_gate = [link.gate[0][0][0] + (link.gate[0][1][0] - link.gate[0][0][0]) * s, link.gate[0][0][1] + (link.gate[0][1][1] - link.gate[0][0][1]) * s];
                        let across = link.map.point(on_gate);
                        let gap = length(sub(places[from].point([on_gate[0], on_gate[1], 0.0]), places[link.to].point([across[0], across[1], 0.0])));
                        assert!(gap < 2.5 * t + 0.6, "厚み {t}: {} から {} で {gap} mm 離れている", tpl.panels[from].name, tpl.panels[link.to].name);
                    }
                }
            }
        }
    }

    #[test]
    fn a_band_goes_around_the_box_and_meets_itself() {
        let (tpl, surface) = tuck(0.5);
        let front = index(&tpl, "正面");
        // 一周の長さと同じ帯を、正面の中央に置く。
        let band = Decal { panel: front, at: [0.5, 0.3], size: [2.0 * (W + D), 14.0], angle: 0.0 };
        let pieces = surface.layout(&tpl, &band);
        let on = |name: &str| pieces.iter().filter(|p| p.panel == index(&tpl, name)).count();
        assert_eq!((on("正面"), on("左側面"), on("右側面")), (1, 1, 1));
        // 背面には、右まわりで着いたぶんと、左まわりで合わせ目をまたいで着いたぶんの 2 つが乗る。
        assert_eq!(on("背面"), 2);
        let back: Vec<&Piece> = pieces.iter().filter(|p| p.panel == index(&tpl, "背面")).collect();
        assert_eq!(back.iter().map(|p| p.seams).sum::<usize>(), 1);
        // 2 つは向きが同じで、ほぼ一周ぶんだけ横にずれている。帯の両端が、背面の上で出会う。
        let between = back[0].place.inverse().then_after(&back[1].place);
        assert!(between.angle().abs() < 1e-3);
        assert!((len2(between.t) - 2.0 * (W + D)).abs() < 3.0, "ずれ {}", len2(between.t));
    }

    #[test]
    fn a_logo_on_a_corner_reaches_each_face_once() {
        let (tpl, surface) = tuck(0.5);
        let front = index(&tpl, "正面");
        // 正面の右上の角に置く。フタ（合わせ目）と右側面（折り線）へ続く。
        let logo = Decal { panel: front, at: [0.98, 0.98], size: [40.0, 40.0], angle: 0.0 };
        let pieces = surface.layout(&tpl, &logo);
        for name in ["正面", "右側面", "上のフタ"] {
            assert_eq!(pieces.iter().filter(|p| p.panel == index(&tpl, name)).count(), 1, "{name}");
        }
        let lid = pieces.iter().find(|p| p.panel == index(&tpl, "上のフタ")).unwrap();
        assert_eq!(lid.seams, 1);
        // フタは展開図の上で背面の先にあり、正面から見ると半回転している。
        assert!((lid.place.angle().abs() - std::f32::consts::PI).abs() < 1e-3);
    }

    #[test]
    fn decals_follow_their_panel_when_the_box_is_resized() {
        let (tpl, _) = tuck(0.5);
        let right = index(&tpl, "右側面");
        let mut logo = Decal { panel: right, at: [0.0, 0.0], size: [30.0, 30.0], angle: 0.3 };
        let (lo, hi) = panel_box(&tpl, right);
        logo.set_center(&tpl, [lo[0] + 0.25 * (hi[0] - lo[0]), lo[1] + 0.75 * (hi[1] - lo[1])]);
        // 寸法を変えても、同じパネルの、同じ割合の位置にある。
        let bigger = Template::tuck_box(200.0, 90.0, 120.0, 1.0);
        let (lo, hi) = panel_box(&bigger, right);
        let center = logo.center(&bigger);
        assert!((center[0] - (lo[0] + 0.25 * (hi[0] - lo[0]))).abs() < 1e-3);
        assert!((center[1] - (lo[1] + 0.75 * (hi[1] - lo[1]))).abs() < 1e-3);
        assert_eq!(logo.size, [30.0, 30.0]);
    }

    #[test]
    fn dragging_across_a_seam_keeps_the_picture_continuous() {
        let (tpl, surface) = tuck(0.5);
        let (front, lid) = (index(&tpl, "正面"), index(&tpl, "上のフタ"));
        let mut logo = Decal { panel: front, at: [0.5, 0.97], size: [40.0, 40.0], angle: 0.0 };
        let before = surface.layout(&tpl, &logo);
        let on_lid = *before.iter().find(|p| p.panel == lid).expect("フタにかかっていない");
        // フタの上の、柄の中心から 5 mm 上にあたる点をつかんで、同じ点へ「動かす」。置かれているパネルだけがフタに変わる。
        let target = on_lid.place.point([0.0, 5.0]);
        surface.drag(&tpl, &mut logo, lid, target, [0.0, 5.0]);
        assert_eq!(logo.panel, lid);
        let after = surface.layout(&tpl, &logo);
        // どのパネルでも、絵の位置と向きは変わらない。
        for piece in &before {
            let same = after.iter().find(|p| p.panel == piece.panel).expect("動かしたらパネルから外れた");
            let between = piece.place.inverse().then_after(&same.place);
            assert!(between.angle().abs() < 1e-3 && len2(between.t) < 0.6, "{} でずれた", tpl.panels[piece.panel].name);
        }
    }

    /// 柄の「上」が、組み上がった箱の中で向いている向き。`closing` は開け閉め（1 が閉じた状態、0 が開けた状態）。
    fn up_when(tpl: &Template, decal: &Decal, closing: f32) -> V3 {
        let frame = tpl.world_at(1.0).then_after(&tpl.transforms(1.0, closing)[decal.panel].at(decal.center(tpl)));
        frame.vector([-decal.angle.sin(), decal.angle.cos(), 0.0])
    }

    fn up_in_box(tpl: &Template, decal: &Decal) -> V3 {
        up_when(tpl, decal, 1.0)
    }

    /// 角度の差（-π〜π）。
    fn turn(a: f32, b: f32) -> f32 {
        (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    }

    #[test]
    fn 立体の上で動かすと_面ごとの上にそろって立つ() {
        let (tpl, surface) = tuck(0.5);
        let mut logo = Decal { panel: index(&tpl, "正面"), at: [0.5, 0.5], size: [30.0, 30.0], angle: 0.0 };
        // 立っている面では箱の上、天面では奥、底面では手前が「上」。
        let expected = [("正面", [0.0, 1.0, 0.0]), ("右側面", [0.0, 1.0, 0.0]), ("背面", [0.0, 1.0, 0.0]), ("左側面", [0.0, 1.0, 0.0]), ("上のフタ", [0.0, 0.0, -1.0]), ("底のフタ", [0.0, 0.0, 1.0])];
        for (name, direction) in expected {
            let panel = index(&tpl, name);
            let at = Decal { panel, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 }.center(&tpl);
            surface.drag_on_solid(&tpl, &mut logo, false, panel, at, [0.0, 0.0]);
            assert_eq!(logo.panel, panel);
            assert!(dot(up_in_box(&tpl, &logo), direction) > 0.999, "{name}: {:?}", up_in_box(&tpl, &logo));
        }
    }

    #[test]
    fn 立体の上でどの面をたどっても_同じ面では同じ向きに戻る() {
        for (tpl, thin) in [(Template::tuck_box(200.0, 160.0, 85.0, 0.5), Template::tuck_box(200.0, 160.0, 85.0, 0.02)), (Template::prism(6, 70.0, 160.0, 0.5), Template::prism(6, 70.0, 160.0, 0.02)), (Template::gable(160.0, 100.0, 110.0, 45.0, 0.5), Template::gable(160.0, 100.0, 110.0, 45.0, 0.02))] {
            let surface = Surface::new(&tpl, &thin);
            let visible: Vec<usize> = (0..tpl.panels.len()).filter(|&i| !tpl.panels[i].hidden_when_closed()).collect();
            // 斜めに回した柄で、見えている面をでたらめな順に 400 回たどる。表の柄でも、裏の柄でも。
            for back in [false, true] {
                let mut logo = Decal { panel: visible[0], at: [0.5, 0.5], size: [30.0, 30.0], angle: 0.3 };
                let mut seen: Vec<Option<f32>> = vec![None; tpl.panels.len()];
                seen[logo.panel] = Some(logo.angle);
                let mut state = 99u32;
                for _ in 0..400 {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let panel = visible[(state >> 16) as usize % visible.len()];
                    let at = Decal { panel, at: [0.3, 0.6], size: [1.0, 1.0], angle: 0.0 }.center(&tpl);
                    surface.drag_on_solid(&tpl, &mut logo, back, panel, at, [4.0, -3.0]);
                    match seen[panel] {
                        Some(first) => assert!(turn(logo.angle, first).abs() < 1e-3, "{} で向きが変わった", tpl.panels[panel].name),
                        None => seen[panel] = Some(logo.angle),
                    }
                }
            }
        }
    }

    #[test]
    fn 箱の内側でも_開けて見える向きにそろって立つ() {
        let (tpl, surface) = tuck(0.5);
        // 背面の内側に、まっすぐ立てた柄。
        let mut logo = Decal { panel: index(&tpl, "背面"), at: [0.5, 0.5], size: [30.0, 30.0], angle: 0.0 };
        assert!(dot(up_in_box(&tpl, &logo), [0.0, 1.0, 0.0]) > 0.999);
        let center = |name: &str| Decal { panel: index(&tpl, name), at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 }.center(&tpl);

        // フタの裏へ運ぶ。フタを開けて立てたとき、上を向いている（閉じた状態では、手前を向いている）。
        surface.drag_on_solid(&tpl, &mut logo, true, index(&tpl, "上のフタ"), center("上のフタ"), [0.0, 0.0]);
        assert!(dot(up_when(&tpl, &logo, 0.0), [0.0, 1.0, 0.0]) > 0.9, "開けたフタの裏: {:?}", up_when(&tpl, &logo, 0.0));
        assert!(dot(up_in_box(&tpl, &logo), [0.0, 0.0, 1.0]) > 0.999);
        // 表の柄なら、同じフタで逆（奥が上）。
        assert!(turn(surface.up(logo.panel, true), surface.up(logo.panel, false)).abs() > 3.0);

        // 底の内側へ運ぶ。上からのぞき込んで読めるように、奥が上。
        surface.drag_on_solid(&tpl, &mut logo, true, index(&tpl, "底のフタ"), center("底のフタ"), [0.0, 0.0]);
        assert!(dot(up_in_box(&tpl, &logo), [0.0, 0.0, -1.0]) > 0.999);

        // 立っている面の内側は、表と同じで、箱の上が上。背面へ戻れば、はじめの向きに戻る。
        for name in ["正面", "左側面", "右側面", "背面"] {
            surface.drag_on_solid(&tpl, &mut logo, true, index(&tpl, name), center(name), [0.0, 0.0]);
            assert!(dot(up_in_box(&tpl, &logo), [0.0, 1.0, 0.0]) > 0.999, "{name}");
            assert!(turn(surface.up(logo.panel, true), surface.up(logo.panel, false)).abs() < 1e-4);
        }
        assert!(turn(logo.angle, 0.0).abs() < 1e-4);
    }

    #[test]
    fn 立体の上で_角を一周しても向きが変わらない() {
        // 正面 → 上のフタ → 右側面 → 正面。絵を途切れさせずに運ぶと、一周で 90 度回る道。
        let (tpl, surface) = tuck(0.5);
        let mut logo = Decal { panel: index(&tpl, "正面"), at: [0.8, 0.8], size: [30.0, 30.0], angle: 0.0 };
        for _ in 0..3 {
            for name in ["上のフタ", "右側面", "正面"] {
                let panel = index(&tpl, name);
                let at = Decal { panel, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 }.center(&tpl);
                surface.drag_on_solid(&tpl, &mut logo, false, panel, at, [0.0, 0.0]);
            }
            assert!(turn(logo.angle, 0.0).abs() < 1e-4, "{}", logo.angle);
        }
    }

    #[test]
    fn 立体の上で_正面から天面へは絵が途切れずに続く() {
        let (tpl, surface) = tuck(0.5);
        let (front, lid) = (index(&tpl, "正面"), index(&tpl, "上のフタ"));
        let mut logo = Decal { panel: front, at: [0.5, 0.97], size: [40.0, 40.0], angle: 0.0 };
        let before = surface.layout(&tpl, &logo);
        let on_lid = *before.iter().find(|p| p.panel == lid).expect("フタにかかっていない");
        surface.drag_on_solid(&tpl, &mut logo, false, lid, on_lid.place.point([0.0, 5.0]), [0.0, 5.0]);
        assert_eq!(logo.panel, lid);
        let after = surface.layout(&tpl, &logo);
        for piece in &before {
            let same = after.iter().find(|p| p.panel == piece.panel).expect("動かしたらパネルから外れた");
            let between = piece.place.inverse().then_after(&same.place);
            assert!(between.angle().abs() < 1e-3 && len2(between.t) < 0.6, "{} でずれた", tpl.panels[piece.panel].name);
        }
    }

    #[test]
    fn picking_finds_the_point_under_the_cursor() {
        let (tpl, _) = tuck(0.5);
        let mesh = tpl.build_mesh(1.0, 1.0);
        let places = tpl.transforms(1.0, 1.0);
        let world = tpl.world_at(1.0);
        for name in ["正面", "右側面", "上のフタ"] {
            let panel = index(&tpl, name);
            let (lo, hi) = panel_box(&tpl, panel);
            let at = [lo[0] + 0.3 * (hi[0] - lo[0]), lo[1] + 0.6 * (hi[1] - lo[1])];
            // 印刷面の点と、その外向きの向きを求めて、外から見る。
            let point = world.point(places[panel].point([at[0], at[1], 0.0]));
            let outward = normalize(sub(point, world.point(places[panel].point([at[0], at[1], 1.0]))));
            let eye = add(point, scale(outward, 300.0));
            let (hit_panel, hit_at) = pick(&tpl, &mesh, eye, scale(outward, -1.0)).unwrap_or_else(|| panic!("{name} に当たらない"));
            assert_eq!(hit_panel, panel, "{name}");
            assert!(len2(sub2(hit_at, at)) < 0.05, "{name}: {hit_at:?} と {at:?}");
        }
        // 箱の内側から外を見ても、印刷面には当たらない。
        let inside_box = world.point(places[0].point([tpl.panels[0].outline[0][0] + 50.0, 80.0, 30.0]));
        let towards_front = normalize(sub(world.point(places[0].point([tpl.panels[0].outline[0][0] + 50.0, 80.0, 0.0])), inside_box));
        assert!(pick(&tpl, &mesh, inside_box, towards_front).is_none());
    }

    #[test]
    fn other_templates_have_sensible_seams() {
        // 角柱は、のりしろの合わせ目が 1 つと、フタのまわり。
        for n in [3usize, 5, 6] {
            let tpl = Template::prism(n, 70.0, 160.0, 0.5);
            let surface = Surface::new(&tpl, &Template::prism(n, 70.0, 160.0, 0.02));
            assert!(has_seam(&tpl, &surface, "側面 1", tpl.panels[n - 1].name), "{n} 角柱: のりしろの合わせ目がない");
        }
        // ゲーブル型は、側面ののりしろの合わせ目。
        let tpl = Template::gable(160.0, 100.0, 110.0, 45.0, 0.5);
        let surface = Surface::new(&tpl, &Template::gable(160.0, 100.0, 110.0, 45.0, 0.02));
        assert!(has_seam(&tpl, &surface, "背面", "左側面"));
        // 屋根のてっぺんでは、2 枚の屋根の辺が重なるが、どちらも持ち手へ続いているので、合わせ目ではない。
        assert!(!has_seam(&tpl, &surface, "正面の屋根", "背面の屋根"));
        // 持ち手は背中合わせに重なる。上と左右の 3 辺で、表から裏へ回り込める（屋根につながる下の辺は違う）。
        let handles = (index(&tpl, "正面の持ち手"), index(&tpl, "背面の持ち手"));
        assert_eq!(surface.seams.iter().filter(|s| (s.a, s.b) == handles || (s.b, s.a) == handles).count(), 3);
    }

    #[test]
    fn a_seam_is_open_only_where_the_paper_does_not_continue() {
        // ピロー型の貼りしろの上の端は、中央では持ち手へ続き、その左右では、背中合わせの貼りしろの縁を越えて裏へ回る。
        let (len, height) = (180.0f32, 110.0f32);
        let tpl = Template::pillow(len, height, 0.5, 1.2);
        let surface = Surface::new(&tpl, &Template::pillow(len, height, 0.02, 1.2));
        let (fin, back_fin, handle) = (index(&tpl, "正面の貼りしろ"), index(&tpl, "背面の貼りしろ"), index(&tpl, "正面の持ち手"));
        // 正面と背面は、上の端の全長が貼りしろへ続いているので、じかには合わさらない。
        assert!(!has_seam(&tpl, &surface, "正面", "背面"));
        assert!(has_seam(&tpl, &surface, "正面の貼りしろ", "背面の貼りしろ"));
        let top = tpl.panels[fin].outline.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
        let reached = |u: f32| -> Vec<usize> {
            let mut logo = Decal { panel: fin, at: [0.0, 0.0], size: [10.0, 10.0], angle: 0.0 };
            logo.set_center(&tpl, [u, top]);
            surface.layout(&tpl, &logo).iter().map(|p| p.panel).collect()
        };
        // 中央: 持ち手へ続く。裏の貼りしろへは行かない。
        let middle = reached(len * 0.5);
        assert!(middle.contains(&handle) && !middle.contains(&back_fin), "{middle:?}");
        // 端のほう: 裏の貼りしろへ回り込む。持ち手へは行かない。
        let side = reached(len * 0.12);
        assert!(side.contains(&back_fin) && !side.contains(&handle), "{side:?}");
    }

    /// 紙の表から見れば表、裏から見れば裏に当たる。
    #[test]
    fn 当たり判定は紙の表と裏を見分ける() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        // 展開図（平ら）のとき。印刷面は手前（+z）を向いている。
        let mesh = tpl.build_mesh(0.0, 1.0);
        let front = tpl.panels.iter().position(|p| p.name == "正面").unwrap();
        let outline = &tpl.panels[front].outline;
        let middle = [(outline[0][0] + outline[2][0]) * 0.5, (outline[0][1] + outline[2][1]) * 0.5];
        let on_paper = tpl.world_at(0.0).point([middle[0], middle[1], 0.0]);
        let from = |z: f32| [on_paper[0], on_paper[1], on_paper[2] + z];

        let (panel, at, back) = pick_side(&tpl, &mesh, from(500.0), [0.0, 0.0, -1.0]).unwrap();
        assert_eq!((panel, back), (front, false));
        assert!((at[0] - middle[0]).abs() < 0.5 && (at[1] - middle[1]).abs() < 0.5, "{at:?} と {middle:?}");
        assert!(pick(&tpl, &mesh, from(500.0), [0.0, 0.0, -1.0]).is_some());

        let (panel, at, back) = pick_side(&tpl, &mesh, from(-500.0), [0.0, 0.0, 1.0]).unwrap();
        assert_eq!((panel, back), (front, true));
        assert!((at[0] - middle[0]).abs() < 0.5 && (at[1] - middle[1]).abs() < 0.5, "{at:?} と {middle:?}");
        // 表だけを返す当たり判定は、裏では当たらない。
        assert!(pick(&tpl, &mesh, from(-500.0), [0.0, 0.0, 1.0]).is_none());
    }

    /// 箱の上の縁にかかった帯が、フタを伝って回り込まない。
    #[test]
    fn 縁にかかった帯は_フタを伝って回り込まない() {
        for (w, d, h) in [(200.0, 160.0, 85.0), (120.0, 60.0, 160.0)] {
            let tpl = Template::tuck_box(w, d, h, 0.5);
            let surface = Surface::new(&tpl, &Template::tuck_box(w, d, h, 0.02));
            let front = tpl.panels.iter().position(|p| p.name == "正面").unwrap();
            // 正面の上の縁をまたぐ高さに、箱を一周する帯を置く。
            let mut band = Decal { panel: front, at: [0.5, 0.5], size: [2.0 * (w + d), 14.0], angle: 0.0 };
            let top = tpl.panels[front].outline.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
            let middle = band.center(&tpl);
            band.set_center(&tpl, [middle[0], top - 4.0]);
            let pieces = surface.layout(&tpl, &band);
            let names: Vec<&str> = pieces.iter().map(|p| tpl.panels[p.panel].name).collect();
            assert_eq!(names.iter().filter(|n| **n == "正面").count(), 1, "正面に帯が 2 本出る: {names:?}");
            assert!(names.contains(&"上のフタ"), "縁をまたいだぶんは、フタに続く: {names:?}");
            assert!(!names.contains(&"底のフタ"), "底までは回り込まない: {names:?}");
            assert!(pieces.iter().all(|p| p.seams <= 1), "合わせ目を続けてまたいでいる: {names:?}");
            // 側面を 1 周はする。
            for side in ["左側面", "右側面", "背面"] {
                assert!(names.contains(&side), "{side} に帯がない: {names:?}");
            }
        }
    }
}
