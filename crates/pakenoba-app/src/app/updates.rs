//! アップデートを確かめる（「ヘルプ」→「アップデートを確かめる」と、起動したとき）。
//!
//! PhotoCraft と同じ部品（craft-libs の craft-update）を使う。GitHub の Releases（[`REPO`]）から新しい版を探し、
//! あれば知らせる。署名した Windows の ZIP 版なら、その場で入れ替えて開き直す。署名していないもの（はじめのうちの配布物）と、
//! 自分でビルドしたものには、ダウンロードの場所を出す。
//!
//! 配布物の名前は `pakenoba-<版>-windows-x64-portable.zip` と `SHA256SUMS.txt`（`.github/workflows/release.yml` が作る）。

use eframe::egui;
use pakenoba_i18n::t;

/// 配布する GitHub のリポジトリ。
pub const REPO: &str = "TJ16th/pakenoba";

/// craft-update に渡す、パケノバの名前。
pub static APP: craft_update::App = craft_update::App {
    id: "pakenoba",
    name: "PAKENOBA",
    tagline: "Package design",
    repo: REPO,
    former_ids: &[],
    former_names: &[],
    icon_png: Some(include_bytes!("../../assets/pakenoba-icon-256.png")),
    accent: Some([0x37, 0x8e, 0xf0]),
    developer: "PAKENOBA",
    bundle_id_override: Some("io.github.tj16th.pakenoba"),
};

/// アップデートの状態。
#[derive(Default)]
pub(super) struct Updates {
    updater: Option<craft_update::updater::Updater>,
    /// 「アップデートを確かめる」の結果の窓を出している。
    result_open: bool,
    /// 起動したときの確かめを、もう始めたか。
    started: bool,
}

impl super::App {
    /// 毎回の描き直しで: 起動したときに 1 度だけ確かめ、知らせの窓を出し、答えに従う。
    pub(super) fn updates_frame(&mut self, ctx: &egui::Context) {
        if !self.updates.started {
            self.updates.started = true;
            if self.settings.check_updates {
                self.updater().check_at_start(ctx);
            }
        }
        let can_start = !self.unsaved();
        let Some(updater) = self.updates.updater.as_mut() else { return };
        updater.tick(ctx);
        match updater.show_if(ctx, can_start) {
            craft_update::updater::Action::None => {}
            craft_update::updater::Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            craft_update::updater::Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
        // 起動したときの確かめで、前のアップデートの結果があれば、それも出す。
        let status = updater.status().clone();
        if self.updates.result_open || matches!(status, craft_update::updater::Status::Updated(_) | craft_update::updater::Status::InstallFailed(_)) {
            let dialog = craft_update::dialog::CheckResultDialog::new(&APP, env!("CARGO_PKG_VERSION"));
            match dialog.show(ctx, &status) {
                craft_update::dialog::CheckResultResponse::None => {}
                craft_update::dialog::CheckResultResponse::Close => {
                    updater.dismiss_status();
                    self.updates.result_open = false;
                }
                craft_update::dialog::CheckResultResponse::CheckAgain => updater.check_now(ctx),
                craft_update::dialog::CheckResultResponse::RestartToUpdate => updater.restart_to_update(),
            }
            // 新しい版が見つかったら、知らせの窓（起動のときと同じもの）に任せる。
            if updater.prompt_open() {
                self.updates.result_open = false;
            }
        }
    }

    fn updater(&mut self) -> &mut craft_update::updater::Updater {
        self.updates.updater.get_or_insert_with(|| craft_update::updater::Updater::new(&APP, env!("CARGO_PKG_VERSION")))
    }

    /// 「ヘルプ」→「アップデートを確かめる」。
    pub(super) fn check_updates_now(&mut self, ctx: &egui::Context) {
        self.updater().check_now(ctx);
        self.updates.result_open = true;
    }

    /// 終わるとき: 用意できたアップデートがあれば、入れ替える係を動かす。
    pub(super) fn updates_exit(&mut self) {
        if let Some(updater) = self.updates.updater.as_mut() {
            updater.start_pending_update();
        }
    }

    /// 「編集」→「環境設定」の、起動したときに確かめるかどうか。
    pub(super) fn updates_menu(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.settings.check_updates, t!("Check for updates at start")).on_hover_text(t!("Asks GitHub whether a newer version of PAKENOBA has been released. Nothing about you or your work is sent")).changed()
            && let Err(e) = self.settings.save()
        {
            self.say(e);
        }
    }
}

/// アップデートの係として起動されたとき（Windows で、入れ替えのために自分を起動し直したとき）。終わったら `true`。
pub(crate) fn finish_helper() -> bool {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match craft_update::self_update::finish(&APP, &args) {
        Some(Err(e)) => {
            eprintln!("PAKENOBA update helper failed: {e}");
            true
        }
        Some(Ok(())) => true,
        None => false,
    }
}
