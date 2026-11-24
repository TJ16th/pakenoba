//! 型の線のファイル（DXF など）から、型紙を作る。
//!
//! 1. 切り線と折り線を、端どうしをつなぎ、交わる所で分けて、1 枚の網にする。
//! 2. 網で囲まれた区画がパネル。行き止まりの線（切り込み、途中で終わる折り線）は除く。
//! 3. 2 枚のパネルが折り線を共有していれば、そこが折り線（まっすぐなものだけ）。
//! 4. いちばん大きなパネルを根にして、長い折り線を優先して木にする。木に入らなかったパネルは
//!    捨て紙（抜きかす）として除く。
//! 5. 折る角度は、はじめは谷折り 90 度。筒（のりしろで閉じる側面の並び）なら、側面の数で割った角度。
//!    折る順番は、筒の側面が先、そこから離れるほどあと。
//! 6. 役割（面、のりしろ、フラップ、差し込み）を、形と並びから当てる。
//!
//! 書き出す型紙の線は、ファイルの線をそのまま使う（`Template::drawn`）。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use pakenoba_i18n::t;

use super::{DrawnLines, Hinge, Hold, Motion, Opening, P2, Panel, Role, Template, centroid, signed_area, upright_world};
use crate::dieline::{Drawing, LineKind};

/// 読み込みの設定。同じファイルと同じ設定なら、いつも同じ型紙になる。
#[derive(Clone, Debug, PartialEq)]
pub struct ImportSettings {
    /// 線のまとまりごとの種類（`Drawing::groups` と同じ並び）。
    pub kinds: Vec<LineKind>,
    /// 1 単位あたりの mm。
    pub unit_mm: f32,
    /// 左右を裏返す（ファイルが紙の裏、内側から見た図のとき）。
    pub mirror: bool,
    /// 折り線ごとに決めた角度（度）。キーは、折ったときに動くパネルの番号。負なら山折り。
    pub folds: BTreeMap<usize, f32>,
}

impl ImportSettings {
    /// ファイルから当てた、はじめの設定。
    pub fn guess(drawing: &Drawing) -> ImportSettings {
        ImportSettings { kinds: drawing.groups.iter().map(|g| g.guess).collect(), unit_mm: drawing.guess_unit_mm(), mirror: false, folds: BTreeMap::new() }
    }
}

/// 折り線 1 本の情報（画面で角度を直すときに使う）。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldInfo {
    /// 折ったときに動くパネル（子）と、つながる先（親）。
    pub panel: usize,
    pub parent: usize,
    pub a: P2,
    pub b: P2,
    /// 当てた角度と、いまの角度（度。負なら山折り）。
    pub guessed: f32,
    pub angle: f32,
}

/// 読み込みの結果のまとめ。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub panels: usize,
    pub folds: Vec<FoldInfo>,
    /// 折り線でつながらず、除いたパネル（抜きかす、穴の中など）。
    pub dropped: usize,
    /// まっすぐでない折り線（いまは折れない）。
    pub curved: usize,
    /// 展開図の大きさ（mm）。
    pub size: P2,
    /// のりしろと貼る相手を見つけたか。
    pub glued: bool,
}

/// フタを開けたときに戻る角度と、小さいフラップが開く角度（組み込みの差し込み箱と同じ）。
const LID_LIFT: f32 = 110.0 * std::f32::consts::PI / 180.0;
const FLAP_LIFT: f32 = 100.0 * std::f32::consts::PI / 180.0;

/// 点どうしを同じとみなす距離（mm）。
const SNAP: f32 = 0.05;

/// 点に番号を付ける（近い点は同じ番号）。
struct Points {
    list: Vec<P2>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

impl Points {
    fn new() -> Points {
        Points { list: Vec::new(), grid: HashMap::new() }
    }

    fn cell(p: P2) -> (i64, i64) {
        ((p[0] / (SNAP * 2.0)).floor() as i64, (p[1] / (SNAP * 2.0)).floor() as i64)
    }

    fn id(&mut self, p: P2) -> usize {
        let (cx, cy) = Points::cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = self.grid.get(&(cx + dx, cy + dy)) {
                    for &i in ids {
                        let q = self.list[i];
                        if (q[0] - p[0]).abs() <= SNAP && (q[1] - p[1]).abs() <= SNAP {
                            return i;
                        }
                    }
                }
            }
        }
        self.list.push(p);
        self.grid.entry((cx, cy)).or_default().push(self.list.len() - 1);
        self.list.len() - 1
    }
}

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: P2, b: P2) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn dot(a: P2, b: P2) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: P2) -> f32 {
    dot(a, a).sqrt()
}

/// 線分 p-q の上で、点 x がどこにあるか（0〜1）。線から SNAP より離れていれば None。
fn on_segment(p: P2, q: P2, x: P2) -> Option<f32> {
    let d = sub(q, p);
    let l2 = dot(d, d);
    if l2 < 1e-12 {
        return None;
    }
    let s = dot(sub(x, p), d) / l2;
    let l = l2.sqrt();
    if s * l < -SNAP || (s - 1.0) * l > SNAP {
        return None;
    }
    let dist = cross(d, sub(x, p)).abs() / l;
    (dist <= SNAP).then_some(s.clamp(0.0, 1.0))
}

/// 2 本の線分が、端以外で交わる所（それぞれの上の位置）。
fn crossing(p: P2, q: P2, r: P2, s: P2) -> Option<(f32, f32)> {
    let (d, e) = (sub(q, p), sub(s, r));
    let denom = cross(d, e);
    if denom.abs() < 1e-9 {
        return None;
    }
    let w = sub(r, p);
    let (a, b) = (cross(w, e) / denom, cross(w, d) / denom);
    let (ld, le) = (len(d), len(e));
    let inside = |x: f32, l: f32| x * l > SNAP && (1.0 - x) * l > SNAP;
    (inside(a, ld) && inside(b, le)).then_some((a, b))
}

/// 名前（`&'static str`）を、番号ごとに 1 回だけ作る。
fn panel_name(index: usize) -> &'static str {
    static NAMES: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap_or_else(|e| e.into_inner());
    while names.len() <= index {
        let n = names.len() + 1;
        names.push(Box::leak(format!("パネル {n}").into_boxed_str()));
    }
    names[index]
}

/// 輪郭から、まっすぐ並んだ途中の点を除く。
fn simplify(points: Vec<P2>) -> Vec<P2> {
    let mut pts = points;
    loop {
        let n = pts.len();
        if n <= 3 {
            return pts;
        }
        let mut removed = false;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let (u, v) = (sub(b, a), sub(c, b));
            let straight = cross(u, v).abs() <= 1e-3 * len(u).max(1e-6) * len(v).max(1e-6) && dot(u, v) > 0.0;
            if straight && !removed {
                removed = true;
                continue;
            }
            out.push(b);
        }
        if !removed {
            return out;
        }
        pts = out;
    }
}

/// 折り線の端が、ほかの線の少し手前で止まっていれば（抜き型では、よくある）、まっすぐ延ばしてつなぐ。
const EXTEND: f32 = 2.0;

fn extend_crease_ends(segments: &mut [(P2, P2, LineKind)]) {
    let touches = |segments: &[(P2, P2, LineKind)], skip: usize, x: P2| segments.iter().enumerate().any(|(k, &(p, q, _))| k != skip && on_segment(p, q, x).is_some());
    for i in 0..segments.len() {
        if segments[i].2 != LineKind::Crease {
            continue;
        }
        for end in 0..2 {
            let (p, q, _) = segments[i];
            let (tip, from) = if end == 0 { (p, q) } else { (q, p) };
            if touches(segments, i, tip) {
                continue;
            }
            let d = sub(tip, from);
            let l = len(d);
            if l < 1e-6 {
                continue;
            }
            let dir = [d[0] / l, d[1] / l];
            // 延ばした先で、いちばん近くで交わる線。
            let far = [tip[0] + dir[0] * EXTEND, tip[1] + dir[1] * EXTEND];
            let mut best: Option<f32> = None;
            for (k, &(r, s, _)) in segments.iter().enumerate() {
                if k == i {
                    continue;
                }
                let (e, f) = (sub(far, tip), sub(s, r));
                let denom = cross(e, f);
                if denom.abs() < 1e-9 {
                    continue;
                }
                let w = sub(r, tip);
                let (a, b) = (cross(w, f) / denom, cross(w, e) / denom);
                if (0.0..=1.0).contains(&a) && (-1e-4..=1.0 + 1e-4).contains(&b) && best.is_none_or(|x| a < x) {
                    best = Some(a);
                }
            }
            if let Some(a) = best {
                let hit = [tip[0] + (far[0] - tip[0]) * a, tip[1] + (far[1] - tip[1]) * a];
                if end == 0 {
                    segments[i].0 = hit;
                } else {
                    segments[i].1 = hit;
                }
            }
        }
    }
}

/// 線分の並び（向きのないもの）。
struct Net {
    points: Vec<P2>,
    /// 辺（点の番号の組。小さい方が先）と、その種類。
    edges: BTreeMap<(usize, usize), LineKind>,
}

/// 切り線と折り線から、網を作る。端をつなぎ、交わる所とＴ字の所で分け、行き止まりを除く。
fn build_net(segments: &[(P2, P2, LineKind)]) -> Net {
    // 交わる所と、ほかの線の端が乗る所で、線分を分ける。
    let mut cuts: Vec<Vec<f32>> = vec![vec![0.0, 1.0]; segments.len()];
    let boxes: Vec<(P2, P2)> = segments.iter().map(|(p, q, _)| ([p[0].min(q[0]) - SNAP, p[1].min(q[1]) - SNAP], [p[0].max(q[0]) + SNAP, p[1].max(q[1]) + SNAP])).collect();
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| boxes[a].0[0].total_cmp(&boxes[b].0[0]));
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if boxes[j].0[0] > boxes[i].1[0] {
                break;
            }
            if boxes[j].0[1] > boxes[i].1[1] || boxes[j].1[1] < boxes[i].0[1] {
                continue;
            }
            let (p, q, _) = segments[i];
            let (r, s, _) = segments[j];
            if let Some((a, b)) = crossing(p, q, r, s) {
                cuts[i].push(a);
                cuts[j].push(b);
            }
            for (x, (from, to), list) in [(r, (p, q), i), (s, (p, q), i), (p, (r, s), j), (q, (r, s), j)] {
                if let Some(at) = on_segment(from, to, x) {
                    cuts[list].push(at);
                }
            }
        }
    }
    let mut ids = Points::new();
    let mut edges: BTreeMap<(usize, usize), LineKind> = BTreeMap::new();
    for (i, (p, q, kind)) in segments.iter().enumerate() {
        let mut at = std::mem::take(&mut cuts[i]);
        at.sort_by(f32::total_cmp);
        let mut prev = ids.id(*p);
        for &x in &at[1..] {
            let point = [p[0] + (q[0] - p[0]) * x, p[1] + (q[1] - p[1]) * x];
            let id = ids.id(point);
            if id != prev {
                let key = (prev.min(id), prev.max(id));
                // 切り線と折り線が重なっていれば、切り線。
                let entry = edges.entry(key).or_insert(*kind);
                if *kind == LineKind::Cut {
                    *entry = LineKind::Cut;
                }
            }
            prev = id;
        }
    }
    // 行き止まり（1 本の線しか来ない点）を、なくなるまで除く。
    loop {
        let mut degree: HashMap<usize, usize> = HashMap::new();
        for &(a, b) in edges.keys() {
            *degree.entry(a).or_default() += 1;
            *degree.entry(b).or_default() += 1;
        }
        let before = edges.len();
        edges.retain(|&(a, b), _| degree[&a] > 1 && degree[&b] > 1);
        if edges.len() == before {
            break;
        }
    }
    Net { points: ids.list, edges }
}

/// 向きのある辺（点の番号の組）から、その左側の区画の番号へ。
type Owners = HashMap<(usize, usize), usize>;

/// 網の区画（左回りの輪郭）と、向きのある辺がどの区画に属するか。
fn faces(net: &Net) -> (Vec<Vec<usize>>, Owners) {
    let mut around: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(a, b) in net.edges.keys() {
        around.entry(a).or_default().push(b);
        around.entry(b).or_default().push(a);
    }
    let angle = |from: usize, to: usize| {
        let d = sub(net.points[to], net.points[from]);
        d[1].atan2(d[0])
    };
    for (&v, list) in around.iter_mut() {
        list.sort_by(|&a, &b| angle(v, a).total_cmp(&angle(v, b)));
    }
    let mut owner: HashMap<(usize, usize), usize> = HashMap::new();
    let mut loops: Vec<Vec<usize>> = Vec::new();
    let mut areas: Vec<f32> = Vec::new();
    let directed: Vec<(usize, usize)> = net.edges.keys().flat_map(|&(a, b)| [(a, b), (b, a)]).collect();
    for start in directed {
        if owner.contains_key(&start) {
            continue;
        }
        let id = loops.len();
        let mut ring = Vec::new();
        let (mut u, mut v) = start;
        loop {
            owner.insert((u, v), id);
            ring.push(u);
            let list = &around[&v];
            let k = list.iter().position(|&w| w == u).unwrap_or(0);
            let w = list[(k + list.len() - 1) % list.len()];
            (u, v) = (v, w);
            if (u, v) == start || ring.len() > net.edges.len() * 2 + 2 {
                break;
            }
        }
        let pts: Vec<P2> = ring.iter().map(|&i| net.points[i]).collect();
        areas.push(signed_area(&pts));
        loops.push(ring);
    }
    // 外回り（面積が負）は、区画ではない。
    let keep: Vec<Option<usize>> = {
        let mut next = 0;
        areas.iter().map(|&a| if a > 1.0 { next += 1; Some(next - 1) } else { None }).collect()
    };
    let faces: Vec<Vec<usize>> = loops.into_iter().zip(&keep).filter_map(|(l, k)| k.map(|_| l)).collect();
    let owner = owner.into_iter().filter_map(|(e, f)| keep[f].map(|k| (e, k))).collect();
    (faces, owner)
}

/// 2 枚のパネルが共有する折り線。
struct Join {
    a: usize,
    b: usize,
    from: P2,
    to: P2,
}

impl Template {
    /// 型の線のファイルから、型紙を作る。`t` は紙の厚み（mm）。
    pub fn imported(drawing: &Drawing, settings: &ImportSettings, t: f32) -> Result<(Template, ImportReport), String> {
        let unit = if settings.unit_mm.is_finite() && settings.unit_mm > 0.0 { settings.unit_mm } else { 1.0 };
        let (bmin, bmax) = drawing.bounds();
        // 紙の座標: mm にし、左下を原点に寄せる（裏返すときは左右を入れ替える）。
        let place = |p: P2| -> P2 {
            let x = if settings.mirror { bmax[0] - p[0] } else { p[0] - bmin[0] };
            [x * unit, (p[1] - bmin[1]) * unit]
        };
        let kind_of = |group: usize| settings.kinds.get(group).copied().unwrap_or(LineKind::Ignore);

        // 書き出す線（そのまま）。
        let mut drawn = DrawnLines::default();
        let mut segments: Vec<(P2, P2, LineKind)> = Vec::new();
        for path in &drawing.paths {
            let kind = kind_of(path.group);
            for pair in path.points.windows(2) {
                let (a, b) = (place(pair[0]), place(pair[1]));
                if len(sub(a, b)) < 1e-4 {
                    continue;
                }
                match kind {
                    LineKind::Cut => drawn.cuts.push((a, b)),
                    LineKind::Crease | LineKind::Perforation => drawn.folds.push((a, b)),
                    LineKind::Ignore => {}
                }
                if matches!(kind, LineKind::Cut | LineKind::Crease) {
                    segments.push((a, b, kind));
                }
            }
        }
        extend_crease_ends(&mut segments);
        if !segments.iter().any(|s| s.2 == LineKind::Cut) {
            return Err(t!("No cut lines were found. Choose which lines are cut lines.").to_owned());
        }

        let net = build_net(&segments);
        let (rings, owner) = faces(&net);
        if rings.is_empty() {
            return Err(t!("The cut lines do not close into a shape. Check that the outline has no gaps.").to_owned());
        }
        let outlines: Vec<Vec<P2>> = rings.iter().map(|r| simplify(r.iter().map(|&i| net.points[i]).collect())).collect();
        let areas: Vec<f32> = outlines.iter().map(|o| signed_area(o)).collect();

        // 折り線で隣り合うパネルの組と、共有する線。
        let mut shared: BTreeMap<(usize, usize), Vec<(P2, P2)>> = BTreeMap::new();
        for (&(u, v), &kind) in &net.edges {
            if kind != LineKind::Crease {
                continue;
            }
            if let (Some(&f), Some(&g)) = (owner.get(&(u, v)), owner.get(&(v, u)))
                && f != g
            {
                shared.entry((f.min(g), f.max(g))).or_default().push((net.points[u], net.points[v]));
            }
        }
        let mut joins: Vec<Join> = Vec::new();
        let mut curved = 0;
        for ((a, b), list) in shared {
            // いちばん長い線分の向きに、全部が乗っていれば、まっすぐな折り線。
            let longest = list.iter().copied().max_by(|x, y| len(sub(x.1, x.0)).total_cmp(&len(sub(y.1, y.0)))).unwrap();
            let dir = sub(longest.1, longest.0);
            let l = len(dir);
            let dir = [dir[0] / l, dir[1] / l];
            let straight = list.iter().all(|&(p, q)| cross(dir, sub(p, longest.0)).abs() < 0.2 && cross(dir, sub(q, longest.0)).abs() < 0.2);
            if !straight {
                curved += 1;
                continue;
            }
            let along: Vec<f32> = list.iter().flat_map(|&(p, q)| [dot(sub(p, longest.0), dir), dot(sub(q, longest.0), dir)]).collect();
            let (lo, hi) = (along.iter().copied().fold(f32::INFINITY, f32::min), along.iter().copied().fold(f32::NEG_INFINITY, f32::max));
            let at = |s: f32| [longest.0[0] + dir[0] * s, longest.0[1] + dir[1] * s];
            joins.push(Join { a, b, from: at(lo), to: at(hi) });
        }

        // 根は、いちばん大きなパネル。長い折り線から順に、木を広げる。
        let root = (0..outlines.len()).max_by(|&a, &b| areas[a].total_cmp(&areas[b])).unwrap();
        let mut parent: Vec<Option<(usize, usize)>> = vec![None; outlines.len()];
        let mut reached = vec![false; outlines.len()];
        reached[root] = true;
        let mut order = vec![root];
        loop {
            let best = joins
                .iter()
                .enumerate()
                .filter(|(_, j)| reached[j.a] != reached[j.b])
                .max_by(|(_, x), (_, y)| len(sub(x.to, x.from)).total_cmp(&len(sub(y.to, y.from))));
            let Some((k, j)) = best else { break };
            let (from, to) = if reached[j.a] { (j.a, j.b) } else { (j.b, j.a) };
            parent[to] = Some((from, k));
            reached[to] = true;
            order.push(to);
        }
        let dropped = outlines.len() - order.len();

        // 親が先に来る並び（深さの順）にする。
        let mut depth = vec![0usize; outlines.len()];
        for &i in &order[1..] {
            depth[i] = depth[parent[i].unwrap().0] + 1;
        }
        let mut sorted = order.clone();
        sorted.sort_by_key(|&i| (depth[i], order.iter().position(|&x| x == i)));
        let index_of: HashMap<usize, usize> = sorted.iter().enumerate().map(|(n, &i)| (i, n)).collect();

        // 筒（根から、同じ向きの折り線でつながる面の並び）。いちばん多くつながる向きを選ぶ。
        let hinge_dir = |i: usize| -> Option<P2> {
            let (_, k) = parent[i]?;
            let d = sub(joins[k].to, joins[k].from);
            let l = len(d);
            Some([d[0] / l, d[1] / l])
        };
        let children = |i: usize| -> Vec<usize> { order.iter().copied().filter(|&c| parent[c].map(|p| p.0) == Some(i)).collect() };
        let chain_along = |dir: P2| -> Vec<usize> {
            let mut chain = vec![root];
            let mut queue = VecDeque::from([root]);
            while let Some(i) = queue.pop_front() {
                for c in children(i) {
                    if hinge_dir(c).is_some_and(|d| cross(d, dir).abs() < 0.02) {
                        chain.push(c);
                        queue.push_back(c);
                    }
                }
            }
            chain
        };
        let chain = children(root).into_iter().filter_map(hinge_dir).map(chain_along).max_by_key(|c| c.len()).unwrap_or_else(|| vec![root]);
        let wall_dir = chain.get(1).and_then(|&c| hinge_dir(c));
        // 折り線に垂直な向きの、パネルの幅。
        let extent = |i: usize, dir: P2| -> f32 {
            let normal = [-dir[1], dir[0]];
            let s: Vec<f32> = outlines[i].iter().map(|&p| dot(p, normal)).collect();
            s.iter().copied().fold(f32::NEG_INFINITY, f32::max) - s.iter().copied().fold(f32::INFINITY, f32::min)
        };

        // のりしろ: 筒の端の、子のない細いパネル。貼る相手は、筒の反対の端。
        let mut tab = None;
        let mut partner = None;
        if let Some(dir) = wall_dir
            && chain.len() >= 4
        {
            let normal = [-dir[1], dir[0]];
            let mut along: Vec<usize> = chain.clone();
            along.sort_by(|&a, &b| dot(centroid(&outlines[a]), normal).total_cmp(&dot(centroid(&outlines[b]), normal)));
            let widths: Vec<f32> = along.iter().map(|&i| extent(i, dir)).collect();
            let mut sorted_widths = widths.clone();
            sorted_widths.sort_by(f32::total_cmp);
            let median = sorted_widths[sorted_widths.len() / 2];
            for (end, other) in [(0, along.len() - 1), (along.len() - 1, 0)] {
                let i = along[end];
                if i != root && widths[end] < 0.45 * median && children(i).iter().all(|&c| !chain.contains(&c)) {
                    tab = Some(i);
                    partner = Some(along[other]);
                    break;
                }
            }
        }
        let walls = chain.len() - usize::from(tab.is_some());
        // 閉じた筒なら、側面の数で一周を割る。
        let wall_angle = if tab.is_some() && walls >= 3 { std::f32::consts::TAU / walls as f32 } else { std::f32::consts::FRAC_PI_2 };
        let wall_width = chain.iter().filter(|&&i| Some(i) != tab).filter_map(|&i| wall_dir.map(|d| extent(i, d))).fold(f32::INFINITY, f32::min);

        // 筒から離れる段（筒の面は 0）。
        let mut level = vec![0usize; outlines.len()];
        for &i in &sorted[1..] {
            let p = parent[i].unwrap().0;
            level[i] = if chain.contains(&i) { 0 } else { level[p] + 1 };
        }
        let levels = level.iter().copied().max().unwrap_or(0).max(1);

        let mut panels: Vec<Panel> = Vec::new();
        let mut folds: Vec<FoldInfo> = Vec::new();
        for (n, &i) in sorted.iter().enumerate() {
            let outline = super::ccw(outlines[i].clone());
            let hinge = parent[i].map(|(p, k)| {
                let j = &joins[k];
                let guessed = if chain.contains(&i) { wall_angle } else { std::f32::consts::FRAC_PI_2 };
                let chosen = settings.folds.get(&n).map(|d| d.to_radians()).unwrap_or(guessed);
                folds.push(FoldInfo { panel: n, parent: index_of[&p], a: j.from, b: j.to, guessed: guessed.to_degrees(), angle: chosen.to_degrees() });
                let window = if level[i] == 0 {
                    (0.0, 0.45)
                } else {
                    let span = 0.55 / levels as f32;
                    let start = 0.45 + span * (level[i] - 1) as f32;
                    (start, (start + span).min(1.0))
                };
                Hinge { parent: index_of[&p], a: j.from, b: j.to, angle: chosen, window, motion: Motion::Fixed }
            });
            let role = if Some(i) == tab {
                Role::GlueTab
            } else if chain.contains(&i) {
                Role::Face
            } else {
                let (dir, hinge_len) = match parent[i] {
                    Some((_, k)) => {
                        let d = sub(joins[k].to, joins[k].from);
                        (d, len(d))
                    }
                    None => ([1.0, 0.0], 1.0),
                };
                let l = len(dir).max(1e-6);
                let e = extent(i, [dir[0] / l, dir[1] / l]);
                if wall_width.is_finite() && e >= 0.9 * wall_width {
                    Role::Face
                } else if level[i] >= 2 && e < 0.5 * hinge_len {
                    Role::Tuck
                } else {
                    Role::Flap
                }
            };
            let name = if i == root { "正面" } else if Some(i) == tab { "のりしろ" } else { panel_name(n) };
            panels.push(Panel { name, outline, hinge, bend: None, curved: None, slices: 0, role, opening: 0 });
        }

        // 折る順番: 筒の側面 → 小さいフラップ → フタなどの面 → その先（差し込みなど）。
        for (n, &i) in sorted.iter().enumerate() {
            let role = panels[n].role;
            if let Some(h) = panels[n].hinge.as_mut()
                && level[i] > 0
            {
                h.window = match (level[i], role) {
                    (1, Role::Flap) => (0.45, 0.65),
                    (1, _) => (0.6, 0.85),
                    _ => (0.8, 1.0),
                };
            }
        }

        // 開け口: 筒の上か下のフタ（面）で、先に差し込みがあるもの。フタは起き、差し込みは抜け、
        // 同じ側の小さいフラップは、フタが起きたあとに開く（組み込みの差し込み箱と同じ動き）。
        let mut openings: Vec<Opening> = Vec::new();
        if let Some(dir) = wall_dir {
            let centers: Vec<f32> = panels.iter().map(|p| dot(centroid(&p.outline), dir)).collect();
            let along = |n: usize| centers[n];
            let middle = chain.iter().map(|&i| dot(centroid(&outlines[i]), dir)).sum::<f32>() / chain.len() as f32;
            for n in 0..panels.len() {
                if level[sorted[n]] != 1 || panels[n].role != Role::Face {
                    continue;
                }
                let Some(tuck) = (0..panels.len()).find(|&c| panels[c].role == Role::Tuck && panels[c].hinge.as_ref().is_some_and(|h| h.parent == n)) else { continue };
                let top = along(n) >= middle;
                let index = if top { 0 } else { 1 };
                if openings.iter().any(|o| panels[o.lid].opening == index) {
                    continue;
                }
                let lid_len = extent(sorted[n], panels[n].hinge.as_ref().map_or(dir, |h| {
                    let d = sub(h.b, h.a);
                    let l = len(d).max(1e-6);
                    [d[0] / l, d[1] / l]
                }));
                let tuck_len = panels[tuck].hinge.as_ref().map_or(0.0, |h| {
                    let d = sub(h.b, h.a);
                    let l = len(d).max(1e-6);
                    extent(sorted[tuck], [d[0] / l, d[1] / l])
                });
                let clear = (tuck_len / lid_len.max(1e-3)).min(1.0).asin();
                if let Some(h) = panels[n].hinge.as_mut() {
                    h.motion = Motion::Lift(LID_LIFT);
                }
                if let Some(h) = panels[tuck].hinge.as_mut() {
                    h.motion = Motion::Slide { lift: LID_LIFT, clear };
                }
                panels[n].opening = index;
                panels[tuck].opening = index;
                for m in 0..panels.len() {
                    if level[sorted[m]] == 1 && panels[m].role == Role::Flap && (along(m) >= middle) == top {
                        if let Some(h) = panels[m].hinge.as_mut() {
                            h.motion = Motion::Flap(FLAP_LIFT);
                        }
                        panels[m].opening = index;
                    }
                }
                openings.push(Opening { lid: n, hold: Hold::Tuck(tuck) });
            }
        }

        let (min, max) = super::bounds(&panels);
        let depth = chain.get(1).filter(|&&c| Some(c) != tab).and_then(|&c| wall_dir.map(|d| extent(c, d))).unwrap_or(0.0);
        let center = centroid(&panels[0].outline);
        let glued: Vec<(usize, usize)> = tab.zip(partner).map(|(a, b)| (index_of[&a], index_of[&b])).into_iter().collect();
        let report = ImportReport { panels: panels.len(), folds, dropped, curved, size: [max[0] - min[0], max[1] - min[1]], glued: !glued.is_empty() };
        let template = Template {
            panels,
            thickness: t,
            min,
            max,
            world: upright_world(center, depth, t),
            tilt: 0.0,
            tilt_window: (0.0, 0.5),
            marks: Vec::new(),
            holes: Vec::new(),
            open_angle: 0.0,
            openings,
            glued,
            drawn: Some(drawn),
        };
        Ok((template, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dieline::{Group, Path};

    /// 差し込み箱の、のりしろつきの筒（4 面 + のりしろ）に、上下のフタをつけた単純な展開図。
    fn tube() -> Drawing {
        let (w, d, h, g) = (100.0f32, 60.0f32, 150.0f32, 15.0f32);
        let xs = [0.0, g, g + d, g + d + w, g + 2.0 * d + w, g + 2.0 * d + 2.0 * w];
        let mut paths = Vec::new();
        let mut line = |group: usize, a: P2, b: P2| paths.push(Path { group, points: vec![a, b] });
        // 外形: 筒の帯と、正面（3 番目の面）の上下につくフタ。
        let (l0, l1) = (xs[3], xs[4]);
        let outline = [[xs[0], 10.0], [xs[1], 0.0], [l0, 0.0], [l0, -d], [l1, -d], [l1, 0.0], [xs[5], 0.0], [xs[5], h], [l1, h], [l1, h + d], [l0, h + d], [l0, h], [xs[1], h], [xs[0], h - 10.0]];
        for k in 0..outline.len() {
            line(0, outline[k], outline[(k + 1) % outline.len()]);
        }
        for &x in &xs[1..5] {
            line(1, [x, 0.0], [x, h]);
        }
        line(1, [l0, 0.0], [l1, 0.0]);
        line(1, [l0, h], [l1, h]);
        // 切り込み（行き止まり）と、関係のない文字の線。
        line(0, [xs[2] + 10.0, 50.0], [xs[2] + 20.0, 50.0]);
        line(2, [0.0, -50.0], [10.0, -50.0]);
        let groups = vec![
            Group { name: "CUT".into(), color: 1, count: 0, guess: LineKind::Cut },
            Group { name: "CREASE".into(), color: 3, count: 0, guess: LineKind::Crease },
            Group { name: "TEXT".into(), color: 7, count: 0, guess: LineKind::Ignore },
        ];
        Drawing { groups, paths, unit_mm: Some(1.0), skipped: 0 }
    }

    #[test]
    fn tube_becomes_a_closed_box() {
        let drawing = tube();
        let settings = ImportSettings::guess(&drawing);
        let (tpl, report) = Template::imported(&drawing, &settings, 0.5).unwrap();
        assert_eq!(report.panels, 7, "{report:?}");
        assert_eq!(report.dropped, 0);
        assert!(report.glued);
        let tab = tpl.panels.iter().position(|p| p.role == Role::GlueTab).expect("glue tab");
        assert!(tpl.glued.contains(&(tab, tpl.glued[0].1)));
        // 筒の折りは 90 度、親は子より先。
        for (i, p) in tpl.panels.iter().enumerate() {
            if let Some(h) = &p.hinge {
                assert!(h.parent < i);
                assert!((h.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
            }
        }
        // 書き出す線は、ファイルのまま（切り込みも入り、文字の線は入らない）。
        assert_eq!(tpl.drawn.as_ref().unwrap().cuts.len(), 15);
        assert_eq!(tpl.drawn.as_ref().unwrap().folds.len(), 6);
        // 組み上がると、幅 100、奥行き 60、高さ 150 の箱になる。
        let mesh = tpl.build_mesh(1.0, 1.0);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for v in &mesh {
            for k in 0..3 {
                lo[k] = lo[k].min(v.pos[k]);
                hi[k] = hi[k].max(v.pos[k]);
            }
        }
        let mut size: Vec<f32> = (0..3).map(|k| hi[k] - lo[k]).collect();
        size.sort_by(f32::total_cmp);
        for (got, want) in size.iter().zip([60.0, 100.0, 150.0]) {
            assert!((got - want).abs() < 4.0, "{size:?}");
        }
    }

    #[test]
    fn fold_angles_can_be_changed_and_mirrored() {
        let drawing = tube();
        let mut settings = ImportSettings::guess(&drawing);
        let (_, report) = Template::imported(&drawing, &settings, 0.5).unwrap();
        let fold = report.folds[0].panel;
        settings.folds.insert(fold, -45.0);
        settings.mirror = true;
        let (tpl, report) = Template::imported(&drawing, &settings, 0.5).unwrap();
        assert!((tpl.panels[fold].hinge.as_ref().unwrap().angle + 45f32.to_radians()).abs() < 1e-5);
        assert_eq!(report.folds.iter().find(|f| f.panel == fold).unwrap().angle, -45.0);
    }

    #[test]
    fn no_cut_lines_is_an_error() {
        let drawing = tube();
        let settings = ImportSettings { kinds: vec![LineKind::Crease; 3], ..ImportSettings::guess(&drawing) };
        assert!(Template::imported(&drawing, &settings, 0.5).is_err());
    }
}

/// 見本の型（`spikes/dielines/pacxio`）を読んで、結果を並べる。`cargo test -p pakenoba-core samples -- --ignored --nocapture`
#[cfg(test)]
mod samples {
    use super::*;

    #[test]
    #[ignore]
    fn samples() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/dielines/pacxio");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "dxf")).collect();
        files.sort();
        for f in files {
            let drawing = crate::dieline::read_dxf(&std::fs::read(&f).unwrap()).unwrap();
            let settings = ImportSettings::guess(&drawing);
            let start = std::time::Instant::now();
            match Template::imported(&drawing, &settings, 0.5) {
                Ok((tpl, r)) => {
                    if let Ok(dir) = std::env::var("PKN_FOLD_DUMP") {
                        for (tag, fold) in [("half", 0.6f32), ("done", 1.0)] {
                            let mesh = tpl.build_mesh(fold, 1.0);
                            let mut text = String::new();
                            for v in mesh.iter().filter(|v| v.kind == 0.0) {
                                text += &format!("{} {} {}\n", v.pos[0], v.pos[1], v.pos[2]);
                            }
                            std::fs::write(format!("{dir}/{}_{tag}.txt", f.file_stem().unwrap().to_string_lossy()), text).unwrap();
                        }
                    }
                    for (k, p) in tpl.panels.iter().enumerate() {
                        let area: f32 = crate::template::triangles(&p.outline).iter().map(|t| signed_area(t).abs()).sum();
                        let want = signed_area(&p.outline).abs();
                        if (area - want).abs() > want * 0.001 + 0.5 {
                            println!("   panel {k}: triangles cover {area:.1} of {want:.1} ({} points)", p.outline.len());
                        }
                    }
                    let thin = Template::imported(&drawing, &settings, 0.02).unwrap().0;
                    let problems: Vec<String> = crate::check::check(&tpl, &thin).iter().map(|p| p.to_string()).collect();
                    println!("   openings {} problems {:?}", tpl.openings.len(), problems);
                    let roles: Vec<String> = tpl.panels.iter().map(|p| format!("{:?}", p.role).chars().take(2).collect()).collect();
                    let angles: Vec<i32> = r.folds.iter().map(|f| f.angle.round() as i32).collect();
                    println!("{:28} panels {:2} dropped {} curved {} glued {} size {:.0}x{:.0} {:?} roles {} angles {:?} ({:?})", f.file_name().unwrap().to_string_lossy(), r.panels, r.dropped, r.curved, r.glued, r.size[0], r.size[1], drawing.groups.iter().map(|g| (&g.name, g.guess)).collect::<Vec<_>>(), roles.join(","), angles, start.elapsed());
                }
                Err(e) => println!("{:28} ERROR {e}", f.file_name().unwrap().to_string_lossy()),
            }
        }
    }
}
