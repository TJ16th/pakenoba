//! 型の線のファイル（DXF）を読む。
//!
//! 抜き型の受け渡しでいちばん多い DXF から、線をレイヤーごとに読み出す。
//! 曲線（円弧、ふくらみのある折れ線）は、細かい折れ線にする。
//! 読んだ線を、切り線・折り線などに分けて、型紙にするのは `template::imported`。
//!
//! 読むもの: LINE、ARC、CIRCLE、LWPOLYLINE、POLYLINE（VERTEX）、INSERT（ブロックを展開）、ELLIPSE。
//! 読まないもの: 文字、寸法、ハッチなど（型の線ではない）。SPLINE は制御点を結んだ折れ線で近似する。

use std::collections::HashMap;

use pakenoba_i18n::t;

pub type P2 = [f32; 2];

/// 線の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// 切り線（抜き）。
    Cut,
    /// 折り線（スジ、罫線）。
    Crease,
    /// ミシン目。折りの形には使わず、型紙の線としてだけ出す。
    Perforation,
    /// 型の線ではない（塗り足し、寸法、文字、のりの位置など）。
    Ignore,
}

impl LineKind {
    pub const ALL: [LineKind; 4] = [LineKind::Cut, LineKind::Crease, LineKind::Perforation, LineKind::Ignore];

    pub fn key(self) -> &'static str {
        match self {
            LineKind::Cut => "cut",
            LineKind::Crease => "crease",
            LineKind::Perforation => "perforation",
            LineKind::Ignore => "ignore",
        }
    }

    pub fn from_key(key: &str) -> Option<LineKind> {
        LineKind::ALL.into_iter().find(|k| k.key() == key)
    }

    /// 画面に出す名前（英語。訳は表から引く）。
    pub fn label(self) -> &'static str {
        match self {
            LineKind::Cut => "Cut",
            LineKind::Crease => "Crease",
            LineKind::Perforation => "Perforation",
            LineKind::Ignore => "Not used",
        }
    }
}

/// 線のまとまり（レイヤー。1 つのレイヤーに色が混ざっているときは、色ごとに分ける）。
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// 分けるときの名前（レイヤー名。色で分けたときは「レイヤー名 / 色の番号」）。
    pub name: String,
    /// AutoCAD の色の番号（1 が赤、3 が緑、5 が青、7 が白／黒）。
    pub color: i32,
    /// 線の本数。
    pub count: usize,
    /// 名前と色から当てた種類。
    pub guess: LineKind,
}

/// 1 本の線（折れ線）。
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub group: usize,
    pub points: Vec<P2>,
}

/// 読んだ図面。座標はファイルの単位のまま。
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub groups: Vec<Group>,
    pub paths: Vec<Path>,
    /// ファイルに書いてある単位（1 単位あたりの mm）。書いていなければ None。
    pub unit_mm: Option<f32>,
    /// 読まなかった図形の数（文字、寸法など）。
    pub skipped: usize,
}

impl Drawing {
    /// 全体を囲む四角（ファイルの単位）。
    pub fn bounds(&self) -> (P2, P2) {
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for p in self.paths.iter().flat_map(|p| &p.points) {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        (min, max)
    }

    /// 単位の見当（1 単位あたりの mm）。ファイルに書いてあればそれ、なければ大きさから当てる
    /// （箱の展開図は、mm なら 60 より大きい。インチなら、ふつうは 60 より小さい）。
    pub fn guess_unit_mm(&self) -> f32 {
        if let Some(unit) = self.unit_mm {
            return unit;
        }
        let (min, max) = self.bounds();
        let size = (max[0] - min[0]).max(max[1] - min[1]);
        if size.is_finite() && size < 60.0 { 25.4 } else { 1.0 }
    }
}

/// 名前と色から、線の種類を当てる。
pub fn guess_kind(name: &str, color: i32) -> LineKind {
    let lower = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    // 先に「使わないもの」を見る（「cut」を含む「cutcontour_bleed」のような名前もあるので、塗り足しなどを先に）。
    if has(&["bleed", "dim", "text", "safe", "glue", "note", "label", "legend", "artwork", "info", "塗り足し", "寸法", "文字", "のり", "糊", "注"]) {
        return LineKind::Ignore;
    }
    if has(&["perf", "zipper", "tear", "ミシン", "ジッパー"]) {
        return LineKind::Perforation;
    }
    if has(&["crease", "fold", "score", "rule", "bend", "折", "スジ", "すじ", "罫", "筋"]) {
        return LineKind::Crease;
    }
    if has(&["cut", "knife", "die", "trim", "contour", "outline", "切", "抜", "カット"]) {
        return LineKind::Cut;
    }
    // 名前で分からなければ、色。赤は切り、緑と青は折り（よくある決まり）。
    match color {
        1 => LineKind::Cut,
        3..=5 => LineKind::Crease,
        _ => LineKind::Cut,
    }
}

/// 曲線を折れ線にするときの、弦と弧のずれの上限（ファイルの単位ではなく、mm で決めたいが、
/// 単位がまだ分からないので、半径に対する割合で決める）。
const ARC_SEGMENT_ANGLE: f32 = std::f32::consts::PI / 90.0;

fn arc_points(center: P2, radius: f32, start_deg: f32, end_deg: f32) -> Vec<P2> {
    let mut sweep = (end_deg - start_deg).rem_euclid(360.0);
    if sweep < 1e-6 {
        sweep = 360.0;
    }
    let (a0, sweep) = (start_deg.to_radians(), sweep.to_radians());
    // 小さい円は少なく、大きい円は細かく（弦と弧のずれが半径の 0.03% 程度）。
    let count = ((sweep / ARC_SEGMENT_ANGLE).ceil() as usize).max(2);
    (0..=count)
        .map(|i| {
            let a = a0 + sweep * i as f32 / count as f32;
            [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
        })
        .collect()
}

/// ふくらみ（bulge）のある辺を、折れ線にする。bulge は、弧の中心角の 1/4 の tan。
fn bulge_points(a: P2, b: P2, bulge: f32) -> Vec<P2> {
    if bulge.abs() < 1e-9 {
        return vec![a, b];
    }
    let theta = 4.0 * bulge.atan();
    let chord = [b[0] - a[0], b[1] - a[1]];
    let len = (chord[0] * chord[0] + chord[1] * chord[1]).sqrt();
    if len < 1e-9 {
        return vec![a, b];
    }
    let radius = len / (2.0 * (theta / 2.0).sin());
    // 中心は、弦の中点から、弦に垂直な向きへ。
    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let h = radius * (theta / 2.0).cos();
    let normal = [-chord[1] / len, chord[0] / len];
    let center = [mid[0] + normal[0] * h, mid[1] + normal[1] * h];
    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
    let count = ((theta.abs() / ARC_SEGMENT_ANGLE).ceil() as usize).max(2);
    let r = radius.abs();
    let mut out: Vec<P2> = (0..=count)
        .map(|i| {
            let ang = start + theta * i as f32 / count as f32;
            [center[0] + r * ang.cos(), center[1] + r * ang.sin()]
        })
        .collect();
    // 端は、元の点そのもの（つなぎ目がずれないように）。
    out[0] = a;
    *out.last_mut().unwrap() = b;
    out
}

/// 図形 1 つ（グループの番号と値の組の並び）。
struct Entity {
    kind: String,
    pairs: Vec<(i32, String)>,
}

impl Entity {
    fn get(&self, code: i32) -> Option<&str> {
        self.pairs.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
    }
    fn num(&self, code: i32) -> Option<f32> {
        self.get(code)?.trim().parse().ok()
    }
    fn int(&self, code: i32) -> Option<i32> {
        self.get(code)?.trim().parse().ok()
    }
}

/// 位置の変換（ブロックの展開に使う）。
#[derive(Clone, Copy)]
struct Place {
    origin: P2,
    scale: P2,
    cos: f32,
    sin: f32,
    base: P2,
}

impl Place {
    const IDENTITY: Place = Place { origin: [0.0, 0.0], scale: [1.0, 1.0], cos: 1.0, sin: 0.0, base: [0.0, 0.0] };

    fn apply(&self, p: P2) -> P2 {
        let x = (p[0] - self.base[0]) * self.scale[0];
        let y = (p[1] - self.base[1]) * self.scale[1];
        [self.origin[0] + x * self.cos - y * self.sin, self.origin[1] + x * self.sin + y * self.cos]
    }
}

/// DXF を読む。テキスト形式だけ（バイナリ DXF は読まない）。
pub fn read_dxf(bytes: &[u8]) -> Result<Drawing, String> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err(t!("Binary DXF files are not supported. Save it as an ASCII (text) DXF.").to_owned());
    }
    // 文字コードは、古いものは Shift_JIS のこともある。型の線を読むだけなので、読めない文字は置き換える。
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let mut pairs: Vec<(i32, String)> = Vec::new();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let Ok(code) = code.trim().parse::<i32>() else {
            return Err(t!("This does not look like a DXF file.").to_owned());
        };
        pairs.push((code, value.trim_end_matches('\r').to_owned()));
    }
    if !pairs.iter().any(|(c, v)| *c == 0 && v == "SECTION") {
        return Err(t!("This does not look like a DXF file.").to_owned());
    }

    // 節ごとに分ける。
    let mut unit_mm = None;
    let mut layer_colors: HashMap<String, i32> = HashMap::new();
    let mut blocks: HashMap<String, (P2, Vec<Entity>)> = HashMap::new();
    let mut entities: Vec<Entity> = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        if pairs[i] != (0, "SECTION".to_owned()) {
            i += 1;
            continue;
        }
        let name = pairs.get(i + 1).map(|(_, v)| v.clone()).unwrap_or_default();
        let end = (i..pairs.len()).find(|&j| pairs[j].0 == 0 && pairs[j].1 == "ENDSEC").unwrap_or(pairs.len());
        let body = &pairs[i + 2..end];
        match name.as_str() {
            "HEADER" => {
                if let Some(k) = body.iter().position(|(c, v)| *c == 9 && v == "$INSUNITS")
                    && let Some((_, v)) = body.get(k + 1)
                {
                    unit_mm = match v.trim() {
                        "1" => Some(25.4),
                        "2" => Some(304.8),
                        "4" => Some(1.0),
                        "5" => Some(10.0),
                        "6" => Some(1000.0),
                        _ => None,
                    };
                }
            }
            "TABLES" => {
                for e in split_entities(body) {
                    if e.kind == "LAYER"
                        && let Some(name) = e.get(2)
                    {
                        layer_colors.insert(name.to_owned(), e.int(62).unwrap_or(7).abs());
                    }
                }
            }
            "BLOCKS" => {
                let list = split_entities(body);
                let mut k = 0;
                while k < list.len() {
                    if list[k].kind == "BLOCK" {
                        let name = list[k].get(2).unwrap_or_default().to_owned();
                        let base = [list[k].num(10).unwrap_or(0.0), list[k].num(20).unwrap_or(0.0)];
                        let mut inner = Vec::new();
                        k += 1;
                        while k < list.len() && list[k].kind != "ENDBLK" {
                            inner.push(Entity { kind: list[k].kind.clone(), pairs: list[k].pairs.clone() });
                            k += 1;
                        }
                        blocks.insert(name, (base, inner));
                    }
                    k += 1;
                }
            }
            "ENTITIES" => entities = split_entities(body),
            _ => {}
        }
        i = end + 1;
    }

    let mut out = Collector { layer_colors: &layer_colors, keys: HashMap::new(), raw: Vec::new(), skipped: 0 };
    out.walk(&entities, &blocks, &[Place::IDENTITY], None, 0);
    let Collector { keys, raw, skipped, .. } = out;
    if raw.is_empty() {
        return Err(t!("No lines were found in this DXF file.").to_owned());
    }

    // レイヤーに色が混ざっていれば、色ごとに分ける。
    let mut colors_of: HashMap<&str, Vec<i32>> = HashMap::new();
    for (layer, color) in keys.keys() {
        colors_of.entry(layer.as_str()).or_default().push(*color);
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut index_of: HashMap<(String, i32), usize> = HashMap::new();
    let mut ordered: Vec<&(String, i32)> = keys.keys().collect();
    ordered.sort();
    for key in ordered {
        let mixed = colors_of[key.0.as_str()].len() > 1;
        let name = if mixed { format!("{} / {}", key.0, key.1) } else { key.0.clone() };
        let found = groups.iter().position(|g| g.name == name);
        let index = match found {
            Some(index) => index,
            None => {
                groups.push(Group { name: name.clone(), color: key.1, count: 0, guess: guess_kind(&key.0, key.1) });
                groups.len() - 1
            }
        };
        index_of.insert(key.clone(), index);
    }
    let paths: Vec<Path> = raw
        .into_iter()
        .map(|(key, points)| {
            let group = index_of[&key];
            groups[group].count += 1;
            Path { group, points }
        })
        .collect();
    Ok(Drawing { groups, paths, unit_mm, skipped })
}

/// 番号 0 の組ごとに、図形に分ける。
fn split_entities(body: &[(i32, String)]) -> Vec<Entity> {
    let mut out: Vec<Entity> = Vec::new();
    for (code, value) in body {
        if *code == 0 {
            out.push(Entity { kind: value.clone(), pairs: Vec::new() });
        } else if let Some(last) = out.last_mut() {
            last.pairs.push((*code, value.clone()));
        }
    }
    out
}

struct Collector<'a> {
    layer_colors: &'a HashMap<String, i32>,
    keys: HashMap<(String, i32), ()>,
    raw: Vec<((String, i32), Vec<P2>)>,
    skipped: usize,
}

impl Collector<'_> {
    fn push(&mut self, layer: &str, color: i32, points: Vec<P2>, places: &[Place]) {
        if points.len() < 2 || points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return;
        }
        let points = points.into_iter().map(|p| places.iter().rev().fold(p, |q, place| place.apply(q))).collect();
        let key = (layer.to_owned(), color);
        self.keys.insert(key.clone(), ());
        self.raw.push((key, points));
    }

    /// 図形をたどる。`inherited` は、ブロックの中のレイヤー「0」と色「ブロックによる」が引き継ぐ、INSERT のレイヤーと色。
    fn walk(&mut self, list: &[Entity], blocks: &HashMap<String, (P2, Vec<Entity>)>, places: &[Place], inherited: Option<(&str, i32)>, depth: usize) {
        let mut k = 0;
        while k < list.len() {
            let e = &list[k];
            k += 1;
            let mut layer = e.get(8).unwrap_or("0").to_owned();
            let mut color = e.int(62).unwrap_or(256);
            if let Some((parent_layer, parent_color)) = inherited {
                if layer == "0" {
                    layer = parent_layer.to_owned();
                }
                if color == 0 {
                    color = parent_color;
                }
            }
            if color == 256 || color == 0 {
                color = self.layer_colors.get(&layer).copied().unwrap_or(7);
            }
            let color = color.abs();
            let point = |x: i32, y: i32| -> Option<P2> { Some([e.num(x)?, e.num(y)?]) };
            match e.kind.as_str() {
                "LINE" => {
                    if let (Some(a), Some(b)) = (point(10, 20), point(11, 21)) {
                        self.push(&layer, color, vec![a, b], places);
                    }
                }
                "ARC" | "CIRCLE" => {
                    if let (Some(c), Some(r)) = (point(10, 20), e.num(40)) {
                        let (s, t) = if e.kind == "ARC" { (e.num(50).unwrap_or(0.0), e.num(51).unwrap_or(360.0)) } else { (0.0, 360.0) };
                        // 押し出しの向きが裏（-Z）なら、左右が逆になる。
                        let flipped = e.num(230).is_some_and(|z| z < 0.0);
                        let mut pts = arc_points(c, r, s, t);
                        if flipped {
                            pts.iter_mut().for_each(|p| p[0] = -p[0]);
                        }
                        self.push(&layer, color, pts, places);
                    }
                }
                "ELLIPSE" => {
                    if let (Some(c), Some(major)) = (point(10, 20), point(11, 21)) {
                        let ratio = e.num(40).unwrap_or(1.0);
                        let (s, t) = (e.num(41).unwrap_or(0.0), e.num(42).unwrap_or(std::f32::consts::TAU));
                        let mut sweep = (t - s).rem_euclid(std::f32::consts::TAU);
                        if sweep < 1e-6 {
                            sweep = std::f32::consts::TAU;
                        }
                        let count = ((sweep / ARC_SEGMENT_ANGLE).ceil() as usize).max(2);
                        let minor = [-major[1] * ratio, major[0] * ratio];
                        let pts = (0..=count)
                            .map(|i| {
                                let a = s + sweep * i as f32 / count as f32;
                                [c[0] + major[0] * a.cos() + minor[0] * a.sin(), c[1] + major[1] * a.cos() + minor[1] * a.sin()]
                            })
                            .collect();
                        self.push(&layer, color, pts, places);
                    }
                }
                "LWPOLYLINE" => {
                    let closed = e.int(70).unwrap_or(0) & 1 == 1;
                    // 点と、その点から次の点への辺のふくらみ。
                    let mut vertices: Vec<(P2, f32)> = Vec::new();
                    let mut x = None;
                    for (code, value) in &e.pairs {
                        let v: f32 = value.trim().parse().unwrap_or(0.0);
                        match code {
                            10 => x = Some(v),
                            20 => {
                                if let Some(x) = x.take() {
                                    vertices.push(([x, v], 0.0));
                                }
                            }
                            42 => {
                                if let Some(last) = vertices.last_mut() {
                                    last.1 = v;
                                }
                            }
                            _ => {}
                        }
                    }
                    self.push(&layer, color, poly_points(&vertices, closed), places);
                }
                "POLYLINE" => {
                    let closed = e.int(70).unwrap_or(0) & 1 == 1;
                    let mut vertices: Vec<(P2, f32)> = Vec::new();
                    while k < list.len() && list[k].kind == "VERTEX" {
                        let v = &list[k];
                        if let (Some(x), Some(y)) = (v.num(10), v.num(20)) {
                            vertices.push(([x, y], v.num(42).unwrap_or(0.0)));
                        }
                        k += 1;
                    }
                    if k < list.len() && list[k].kind == "SEQEND" {
                        k += 1;
                    }
                    self.push(&layer, color, poly_points(&vertices, closed), places);
                }
                "SPLINE" => {
                    // 通る点（11, 21）があればそれ、なければ制御点（10, 20）を結ぶ。
                    let collect = |cx: i32, cy: i32| -> Vec<P2> {
                        let xs: Vec<f32> = e.pairs.iter().filter(|(c, _)| *c == cx).filter_map(|(_, v)| v.trim().parse().ok()).collect();
                        let ys: Vec<f32> = e.pairs.iter().filter(|(c, _)| *c == cy).filter_map(|(_, v)| v.trim().parse().ok()).collect();
                        xs.into_iter().zip(ys).map(|(x, y)| [x, y]).collect()
                    };
                    let fit = collect(11, 21);
                    let pts = if fit.len() >= 2 { fit } else { collect(10, 20) };
                    self.push(&layer, color, pts, places);
                }
                "INSERT" => {
                    let name = e.get(2).unwrap_or_default();
                    if depth < 8
                        && let Some((base, inner)) = blocks.get(name)
                    {
                        let rot = e.num(50).unwrap_or(0.0).to_radians();
                        let place = Place {
                            origin: point(10, 20).unwrap_or([0.0, 0.0]),
                            scale: [e.num(41).unwrap_or(1.0), e.num(42).unwrap_or(1.0)],
                            cos: rot.cos(),
                            sin: rot.sin(),
                            base: *base,
                        };
                        let mut nested = places.to_vec();
                        nested.push(place);
                        self.walk(inner, blocks, &nested, Some((&layer, color)), depth + 1);
                    }
                }
                "VERTEX" | "SEQEND" => {}
                _ => self.skipped += 1,
            }
        }
    }
}

/// 折れ線（ふくらみのある辺は弧にする）。
fn poly_points(vertices: &[(P2, f32)], closed: bool) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::new();
    let n = vertices.len();
    if n < 2 {
        return out;
    }
    let edges = if closed { n } else { n - 1 };
    for i in 0..edges {
        let (a, bulge) = vertices[i];
        let b = vertices[(i + 1) % n].0;
        let piece = bulge_points(a, b, bulge);
        if out.is_empty() {
            out.extend(piece);
        } else {
            out.extend(piece.into_iter().skip(1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(body: &str) -> Vec<u8> {
        format!("0\nSECTION\n2\nENTITIES\n{body}0\nENDSEC\n0\nEOF\n").into_bytes()
    }

    #[test]
    fn reads_lines_arcs_and_polylines() {
        let text = "0\nLINE\n8\nCUT\n10\n0\n20\n0\n11\n100\n21\n0\n\
                    0\nARC\n8\nCUT\n10\n0\n20\n0\n40\n10\n50\n0\n51\n90\n\
                    0\nLWPOLYLINE\n8\nCREASE\n90\n3\n70\n1\n10\n0\n20\n0\n10\n10\n20\n0\n42\n1\n10\n10\n20\n10\n\
                    0\nTEXT\n8\nNOTE\n";
        let d = read_dxf(&dxf(text)).unwrap();
        assert_eq!(d.paths.len(), 3);
        assert_eq!(d.skipped, 1);
        let names: Vec<_> = d.groups.iter().map(|g| (g.name.as_str(), g.guess)).collect();
        assert_eq!(names, [("CREASE", LineKind::Crease), ("CUT", LineKind::Cut)]);
        // 円弧の端は、半径の上。
        let arc = &d.paths[1].points;
        assert!((arc[0][0] - 10.0).abs() < 1e-4 && arc.last().unwrap()[1] > 9.99);
        // ふくらみ 1 は半円。閉じた折れ線は、はじめの点に戻る。
        let poly = &d.paths[2].points;
        assert_eq!(poly.first(), poly.last());
        assert!(poly.iter().any(|p| p[0] > 14.9));
    }

    #[test]
    fn expands_blocks_and_guesses_units() {
        let text = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n\
                    0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nB\n10\n0\n20\n0\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDBLK\n0\nENDSEC\n\
                    0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nB\n8\nKNIFE\n10\n5\n20\n5\n50\n90\n0\nENDSEC\n0\nEOF\n";
        let d = read_dxf(text.as_bytes()).unwrap();
        assert_eq!(d.unit_mm, Some(25.4));
        assert_eq!(d.groups[0].name, "KNIFE");
        let p = &d.paths[0].points;
        assert!((p[1][0] - 5.0).abs() < 1e-5 && (p[1][1] - 6.0).abs() < 1e-5, "{p:?}");
    }

    #[test]
    fn guesses_kinds_from_names_and_colors() {
        assert_eq!(guess_kind("Die Cut", 7), LineKind::Cut);
        assert_eq!(guess_kind("FOLD", 7), LineKind::Crease);
        assert_eq!(guess_kind("スジ", 7), LineKind::Crease);
        assert_eq!(guess_kind("BLEED", 1), LineKind::Ignore);
        assert_eq!(guess_kind("PERFORATION", 7), LineKind::Perforation);
        assert_eq!(guess_kind("0", 3), LineKind::Crease);
        assert!(read_dxf(b"hello").is_err());
    }
}

/// 読み込んだ型の線のファイルと、読み方の設定。型（`project::Kind::Imported`）は、これを番号で指す。
#[derive(Debug)]
pub struct Source {
    /// 元のファイルの名前（場所は含まない）。
    pub name: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub drawing: Drawing,
    pub settings: crate::template::ImportSettings,
}

fn registry() -> &'static std::sync::Mutex<HashMap<u64, std::sync::Arc<Source>>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, std::sync::Arc<Source>>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// 中身と設定から決まる番号（同じファイルと同じ設定なら、同じ番号）。
fn key_of(bytes: &[u8], settings: &crate::template::ImportSettings) -> u64 {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.update(format!("{settings:?}").as_bytes());
    let digest = hash.finalize();
    u64::from_le_bytes(digest[..8].try_into().unwrap_or_default()).max(1)
}

/// 読み込んだファイルを覚えて、番号を返す。読めなければ、理由を返す。
pub fn register(name: &str, bytes: std::sync::Arc<Vec<u8>>, settings: Option<crate::template::ImportSettings>) -> Result<u64, String> {
    let drawing = read_dxf(&bytes)?;
    let settings = settings.unwrap_or_else(|| crate::template::ImportSettings::guess(&drawing));
    let key = key_of(&bytes, &settings);
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key).or_insert_with(|| std::sync::Arc::new(Source { name: name.to_owned(), bytes, drawing, settings }));
    Ok(key)
}

/// 同じファイルで、設定だけを変えた番号。
pub fn with_settings(key: u64, settings: crate::template::ImportSettings) -> Option<u64> {
    let old = source(key)?;
    let new_key = key_of(&old.bytes, &settings);
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(new_key).or_insert_with(|| std::sync::Arc::new(Source { name: old.name.clone(), bytes: old.bytes.clone(), drawing: old.drawing.clone(), settings }));
    Some(new_key)
}

/// 番号から、読み込んだファイルを引く。
pub fn source(key: u64) -> Option<std::sync::Arc<Source>> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned()
}

/// 2 つの番号が、同じパネルの並びになるか（同じファイルで、線の分け方、単位、裏返しが同じ。折る角度は違ってよい）。
pub fn same_layout(a: u64, b: u64) -> bool {
    if a == b {
        return true;
    }
    match (source(a), source(b)) {
        (Some(x), Some(y)) => x.bytes == y.bytes && x.settings.kinds == y.settings.kinds && x.settings.unit_mm == y.settings.unit_mm && x.settings.mirror == y.settings.mirror,
        _ => false,
    }
}
