//! 付属の見本の案件（箔押し、エンボス、その組み合わせ）。ホーム画面から開く。
//!
//! 見本は、絵の素材を使わない案件（図形と付属の見本の柄だけ）なので、設定のテキストだけを同梱する。
//! 開くと、名前のない新しい案件になる（見本そのものは変わらない）。

use pakenoba_i18n::t;

use pakenoba_core::project::Project;

use super::{App, Workspace};

/// 見本の名前（英語。画面では訳す）と、設定のテキスト。
pub(super) const SAMPLES: &[(&str, &str)] = &[
    ("Foil sampler (7 foils on navy, matte PP)", include_str!("../../assets/samples/foil.toml")),
    ("Embossing sampler (heights, deboss, raised foil)", include_str!("../../assets/samples/emboss.toml")),
    ("Kraft with white ink, hologram band and copper foil", include_str!("../../assets/samples/kraft.toml")),
    ("Black matte PP with spot UV and raised gold foil", include_str!("../../assets/samples/black.toml")),
];

// 訳の表に載せる文（`SAMPLES` の名前）。
#[allow(dead_code)]
fn names() -> [&'static str; 4] {
    [
        t!("Foil sampler (7 foils on navy, matte PP)"),
        t!("Embossing sampler (heights, deboss, raised foil)"),
        t!("Kraft with white ink, hologram band and copper foil"),
        t!("Black matte PP with spot UV and raised gold foil"),
    ]
}

impl App {
    /// 見本を、新しい案件として開く（いまの案件に変更があれば、新しいタブで）。
    pub(super) fn open_sample(&mut self, index: usize) {
        let Some(&(name, text)) = SAMPLES.get(index) else { return };
        if !(self.pristine() || self.open_tab()) {
            return;
        }
        match Project::from_text(text) {
            Ok(opened) => {
                let problems = self.apply_project(opened.project);
                self.file.path = None;
                self.file.suggest = pakenoba_i18n::tr(name).to_owned();
                self.mark_saved();
                self.workspace = Workspace::Design;
                self.home = false;
                if problems.is_empty() {
                    self.say(t!("Opened the sample \"{}\". Save it to keep your changes.", pakenoba_i18n::tr(name)));
                } else {
                    self.say(problems.join(" "));
                }
            }
            Err(e) => self.say(t!("Could not open the sample: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 見本は_どれも傷なく開ける() {
        for (name, text) in SAMPLES {
            let opened = Project::from_text(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(opened.problems.is_empty(), "{name}: {:?}", opened.problems);
            assert!(!opened.project.finishes.is_empty(), "{name}");
            for f in &opened.project.finishes {
                assert!(opened.project.decals.iter().any(|d| d.layer.id == f.link), "{name}: 加工 {} のつなぐ柄がない", f.id);
            }
        }
    }
}
