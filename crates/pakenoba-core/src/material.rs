//! 紙とコート（箱全体の素材）。
//!
//! 見た目の数値（色、つや）は描画の側が持つ。ここは種類と、案件のファイルに書く名前だけ。
//! くわしくは `docs/finishes.md`。

use pakenoba_i18n::t;

/// 紙の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Paper {
    /// コートボール（表が白いコート、裏がグレーの板紙）。紙箱でいちばん多い。
    #[default]
    CoatedBoard,
    /// コート紙（両面が白くつやのある紙）。
    Coated,
    /// マットコート紙。
    MatteCoated,
    /// 上質紙（コートのない白い紙）。
    Uncoated,
    /// クラフト紙（茶色の紙）。
    Kraft,
    /// 段ボール（茶色のライナー）。
    Corrugated,
}

impl Paper {
    pub const ALL: [Paper; 6] = [Paper::CoatedBoard, Paper::Coated, Paper::MatteCoated, Paper::Uncoated, Paper::Kraft, Paper::Corrugated];

    /// 案件のファイルに書く名前。
    pub fn key(self) -> &'static str {
        match self {
            Paper::CoatedBoard => "coated_board",
            Paper::Coated => "coated",
            Paper::MatteCoated => "matte_coated",
            Paper::Uncoated => "uncoated",
            Paper::Kraft => "kraft",
            Paper::Corrugated => "corrugated",
        }
    }

    pub fn from_key(key: &str) -> Option<Paper> {
        Paper::ALL.into_iter().find(|p| p.key() == key)
    }

    /// 画面に出す名前。
    pub fn label(self) -> &'static str {
        match self {
            Paper::CoatedBoard => t!("Coated board (gray back)"),
            Paper::Coated => t!("Coated paper"),
            Paper::MatteCoated => t!("Matte coated paper"),
            Paper::Uncoated => t!("Uncoated paper"),
            Paper::Kraft => t!("Kraft paper"),
            Paper::Corrugated => t!("Corrugated board"),
        }
    }
}

/// 表に掛けるコート（ニス、PP）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Coat {
    #[default]
    None,
    MatteVarnish,
    GlossVarnish,
    MattePp,
    GlossPp,
    /// ソフトタッチ（ベルベット）PP。
    SoftTouchPp,
}

impl Coat {
    pub const ALL: [Coat; 6] = [Coat::None, Coat::MatteVarnish, Coat::GlossVarnish, Coat::MattePp, Coat::GlossPp, Coat::SoftTouchPp];

    pub fn key(self) -> &'static str {
        match self {
            Coat::None => "none",
            Coat::MatteVarnish => "matte_varnish",
            Coat::GlossVarnish => "gloss_varnish",
            Coat::MattePp => "matte_pp",
            Coat::GlossPp => "gloss_pp",
            Coat::SoftTouchPp => "soft_touch_pp",
        }
    }

    pub fn from_key(key: &str) -> Option<Coat> {
        Coat::ALL.into_iter().find(|c| c.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Coat::None => t!("No coating"),
            Coat::MatteVarnish => t!("Matte varnish"),
            Coat::GlossVarnish => t!("Gloss varnish"),
            Coat::MattePp => t!("Matte PP lamination"),
            Coat::GlossPp => t!("Gloss PP lamination"),
            Coat::SoftTouchPp => t!("Soft-touch PP lamination"),
        }
    }
}

/// 箱全体の素材。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Material {
    pub paper: Paper,
    /// 紙の表（箱の外側）に掛けるコート。裏には掛けない。
    pub coat: Coat,
}

/// 箔の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Foil {
    #[default]
    Gold,
    Silver,
    Copper,
    /// 赤金（赤みの強い金）。
    RedGold,
    MatteGold,
    MatteSilver,
    /// ホログラム（見る向きで虹色に変わる）。
    Hologram,
}

impl Foil {
    pub const ALL: [Foil; 7] = [Foil::Gold, Foil::Silver, Foil::Copper, Foil::RedGold, Foil::MatteGold, Foil::MatteSilver, Foil::Hologram];

    pub fn key(self) -> &'static str {
        match self {
            Foil::Gold => "gold",
            Foil::Silver => "silver",
            Foil::Copper => "copper",
            Foil::RedGold => "red_gold",
            Foil::MatteGold => "matte_gold",
            Foil::MatteSilver => "matte_silver",
            Foil::Hologram => "hologram",
        }
    }

    pub fn from_key(key: &str) -> Option<Foil> {
        Foil::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Foil::Gold => t!("Gold foil"),
            Foil::Silver => t!("Silver foil"),
            Foil::Copper => t!("Copper foil"),
            Foil::RedGold => t!("Red gold foil"),
            Foil::MatteGold => t!("Matte gold foil"),
            Foil::MatteSilver => t!("Matte silver foil"),
            Foil::Hologram => t!("Hologram foil"),
        }
    }

    /// 並びの番号（描くときに、箔の種類を絵の 1 チャンネルに入れるため）。
    pub fn index(self) -> usize {
        Foil::ALL.iter().position(|&f| f == self).unwrap_or(0)
    }
}

/// 加工の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum FinishKind {
    /// 箔押し。
    #[default]
    Foil,
    /// エンボス（浮き出し）かデボス（へこみ）。
    Emboss,
    /// 部分 UV ニス（その形だけに、つやの膜を足す）。
    SpotUv,
    /// 白版（色の下に白いインキを敷く。クラフト紙や色の紙で、色を鮮やかに出す）。
    White,
}

impl FinishKind {
    pub fn key(self) -> &'static str {
        match self {
            FinishKind::Foil => "foil",
            FinishKind::Emboss => "emboss",
            FinishKind::SpotUv => "spot_uv",
            FinishKind::White => "white",
        }
    }

    pub fn from_key(key: &str) -> Option<FinishKind> {
        [FinishKind::Foil, FinishKind::Emboss, FinishKind::SpotUv, FinishKind::White].into_iter().find(|k| k.key() == key)
    }
}

/// エンボスの高さ（mm）の範囲と、はじめの値。
pub const EMBOSS_HEIGHT: std::ops::RangeInclusive<f32> = 0.05..=2.0;
pub const DEFAULT_EMBOSS_HEIGHT: f32 = 0.4;
/// エンボスの縁の坂の幅（mm）の範囲と、はじめの値。
pub const EMBOSS_EDGE: std::ops::RangeInclusive<f32> = 0.1..=3.0;
pub const DEFAULT_EMBOSS_EDGE: f32 = 0.6;

/// 加工のレイヤー（特殊版）。箔押しと、エンボス・デボス。
///
/// 形は、つないだ柄（`link` のレイヤー）の形をそのまま使う。柄を動かすと、加工も付いてくる。
/// 柄を隠しても、加工の形には使う（インキを刷らずに箔だけを押す、空押しする、を表すため）。
/// 同じ柄に箔とエンボスをつなぐと、浮き出し箔になる。
#[derive(Clone, Debug, PartialEq)]
pub struct Finish {
    /// 加工ごとの決まった印（柄のレイヤーの印と同じ数の並びから付ける）。0 はまだ付けていない。
    pub id: u32,
    pub kind: FinishKind,
    /// 箔の種類（箔押しのとき）。
    pub foil: Foil,
    /// 形を借りる柄のレイヤーの印。
    pub link: u32,
    /// 使う人が付けた名前。
    pub name: Option<String>,
    pub hidden: bool,
    /// 押した箔の小さなうねり（0〜100%）。0 は鏡のように平ら（箔押しのとき）。
    pub wave: u8,
    /// 浮き出しの高さ（mm。エンボスのとき）。
    pub height: f32,
    /// 真ならへこませる（デボス）。
    pub deboss: bool,
    /// 縁の坂の幅（mm）。小さいほど、縁が切り立つ。
    pub edge: f32,
    /// つや消しの部分 UV ニス（偽ならグロス）。
    pub matte: bool,
}

impl Finish {
    /// 柄 `link` に押す箔。
    pub fn foil(foil: Foil, link: u32) -> Finish {
        Finish { id: 0, kind: FinishKind::Foil, foil, link, name: None, hidden: false, wave: DEFAULT_WAVE, height: DEFAULT_EMBOSS_HEIGHT, deboss: false, edge: DEFAULT_EMBOSS_EDGE, matte: false }
    }

    /// 柄 `link` の形のエンボス（`deboss` が真ならデボス）。
    pub fn emboss(link: u32, deboss: bool) -> Finish {
        Finish { kind: FinishKind::Emboss, deboss, ..Finish::foil(Foil::Gold, link) }
    }

    /// 柄 `link` の形の部分 UV ニス（`matte` が真ならつや消し）。
    pub fn spot_uv(link: u32, matte: bool) -> Finish {
        Finish { kind: FinishKind::SpotUv, matte, ..Finish::foil(Foil::Gold, link) }
    }

    /// 柄 `link` の形の白版。
    pub fn white(link: u32) -> Finish {
        Finish { kind: FinishKind::White, ..Finish::foil(Foil::Gold, link) }
    }
}

/// 箔のうねりの、はじめの値（%）。
pub const DEFAULT_WAVE: u8 = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 名前から戻せる() {
        for p in Paper::ALL {
            assert_eq!(Paper::from_key(p.key()), Some(p));
        }
        for c in Coat::ALL {
            assert_eq!(Coat::from_key(c.key()), Some(c));
        }
        assert_eq!(Paper::from_key("washi"), None);
        for f in Foil::ALL {
            assert_eq!(Foil::from_key(f.key()), Some(f));
        }
    }
}
