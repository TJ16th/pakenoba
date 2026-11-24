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
    }
}
