//! 見本パック。アプリ本体とは別に配る、素材の詰め合わせ（架空のロゴ、箱の全面に使える模様、旧パケノバの見本）。
//!
//! 中身は ZIP 1 つ（`tools/make_sample_pack.py` で作る）。`pakenoba-samples.json` に、入っているファイルの一覧がある。
//! 入れると、使う人のデータのフォルダの「PAKENOBA sample pack」に広げ、素材の画面のコレクションに登録する
//! （ほかの登録したフォルダと同じく、そこから案件へ取り込む）。
//! 配る場所（GitHub のリリース）が決まるまでは、ZIP のファイルから入れる。

use pakenoba_i18n::t;
use serde_json::Value;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::App;

/// 見本パックを取ってくる場所。公開するリポジトリが決まったら、リリースの ZIP の URL を書く。
pub(super) const PACK_URL: Option<&str> = None;

/// 見本パックを広げるフォルダの名前（素材の画面には、この名前で出る）。
const PACK_FOLDER: &str = "PAKENOBA sample pack";

/// 見本パックを広げる場所。
pub(super) fn pack_dir() -> Option<PathBuf> {
    Some(super::settings::data_dir()?.join(PACK_FOLDER))
}

/// ZIP の中の名前が、広げる先の外を指していないか（`..` や、絶対の場所）。
fn safe_name(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().all(|c| matches!(c, Component::Normal(_))) { Some(path.to_path_buf()) } else { None }
}

/// 見本パックの ZIP を `dir` に広げる。前に入れたものは、広げ終わってから置き換える。入れた素材の数を返す。
/// 一覧（`pakenoba-samples.json`）にあるファイルだけを広げる。
pub(super) fn install(bytes: &[u8], dir: &Path) -> Result<usize, String> {
    let bad = |e: &dyn std::fmt::Display| t!("Could not read the sample pack: {e}");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(&e))?;
    let manifest: Value = {
        let mut text = String::new();
        zip.by_name("pakenoba-samples.json").map_err(|_| t!("This ZIP is not a PAKENOBA sample pack (pakenoba-samples.json is missing).").to_owned())?.read_to_string(&mut text).map_err(|e| bad(&e))?;
        serde_json::from_str(&text).map_err(|e| bad(&e))?
    };
    if manifest.get("format").and_then(Value::as_str) != Some("pakenoba-sample-pack") {
        return Err(t!("This ZIP is not a PAKENOBA sample pack (pakenoba-samples.json is missing).").to_owned());
    }
    let files: Vec<String> = manifest.get("items").and_then(Value::as_array).map(|items| items.iter().filter_map(|i| i.get("file")?.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let parent = dir.parent().ok_or_else(|| bad(&"folder"))?;
    std::fs::create_dir_all(parent).map_err(|e| bad(&e))?;
    let staging = parent.join(format!(".{PACK_FOLDER}.new"));
    let _ = std::fs::remove_dir_all(&staging);
    let mut count = 0;
    for name in files.iter().map(String::as_str).chain(["pakenoba-samples.json", "README.txt"]) {
        let Some(rel) = safe_name(name) else { return Err(bad(&format!("unsafe name {name}"))) };
        let Ok(mut entry) = zip.by_name(name) else {
            if name == "README.txt" {
                continue;
            }
            return Err(bad(&format!("{name} is missing")));
        };
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| bad(&e))?;
        let out = staging.join(&rel);
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| bad(&e))?;
        }
        std::fs::write(&out, data).map_err(|e| bad(&e))?;
        if files.iter().any(|f| f == name) {
            count += 1;
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&staging, dir).map_err(|e| bad(&e))?;
    Ok(count)
}

impl App {
    /// 見本パックが入っているか。
    pub(super) fn pack_installed(&self) -> bool {
        pack_dir().is_some_and(|d| d.join("pakenoba-samples.json").exists())
    }

    /// 見本パックの ZIP を選んでもらい、入れる。
    pub(super) fn install_pack_file(&mut self) {
        let answer = match self.library.answer_folder.take() {
            Some(answer) => Some(answer),
            None => rfd::FileDialog::new().set_title(t!("Choose the sample pack (ZIP)")).add_filter("ZIP", &["zip"]).pick_file(),
        };
        let Some(path) = answer else { return };
        match std::fs::read(&path).map_err(|e| t!("Could not read {}: {e}", path.display())).and_then(|bytes| self.install_pack_bytes(&bytes)) {
            Ok(count) => self.say(t!("Installed the sample pack ({count} assets). It is listed under \"Collections\".")),
            Err(e) => self.say(e),
        }
    }

    /// 見本パックを広げ、コレクションに登録して、そこを開く。
    pub(super) fn install_pack_bytes(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let dir = pack_dir().ok_or_else(|| t!("The data folder was not found, so the sample pack cannot be installed.").to_owned())?;
        let count = install(bytes, &dir)?;
        self.settings.add(dir.clone());
        self.settings.save()?;
        let index = self.settings.collections.iter().position(|c| c.path == dir).unwrap_or(0);
        self.open_collection(index);
        Ok(count)
    }

    /// 見本パックを、配る場所から取ってくる（別のスレッドで。終わったら知らせる）。
    pub(super) fn download_pack(&mut self) {
        let Some(url) = PACK_URL else { return };
        if self.pack_job.is_some() {
            return;
        }
        let (send, recv) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = ureq::get(url).call().map_err(|e| e.to_string()).and_then(|mut r| r.body_mut().with_config().limit(200 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string()));
            let _ = send.send(result);
        });
        self.pack_job = Some(recv);
        self.say(t!("Downloading the sample pack…"));
    }

    /// 取ってくるのが終わっていたら、入れる。
    pub(super) fn poll_pack(&mut self, ctx: &eframe::egui::Context) {
        let Some(recv) = &self.pack_job else { return };
        match recv.try_recv() {
            Ok(result) => {
                self.pack_job = None;
                match result.map_err(|e| t!("Could not download the sample pack: {e}")).and_then(|bytes| self.install_pack_bytes(&bytes)) {
                    Ok(count) => self.say(t!("Installed the sample pack ({count} assets). It is listed under \"Collections\".")),
                    Err(e) => self.say(e),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(300)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pack_job = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn pack(files: &[(&str, &[u8])], listed: &[&str]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default();
            let items: Vec<Value> = listed.iter().map(|f| serde_json::json!({ "file": f, "name": f })).collect();
            z.start_file("pakenoba-samples.json", opts).unwrap();
            z.write_all(serde_json::json!({ "format": "pakenoba-sample-pack", "version": 1, "items": items }).to_string().as_bytes()).unwrap();
            for (name, data) in files {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn 見本パックは_一覧のファイルだけを広げ_入れ直すと置き換わる() {
        let root = std::env::temp_dir().join(format!("pakenoba-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(PACK_FOLDER);
        let first = pack(&[("logos/a.png", b"a"), ("patterns/b.svg", b"<svg/>"), ("extra.exe", b"x")], &["logos/a.png", "patterns/b.svg"]);
        assert_eq!(install(&first, &dir).unwrap(), 2);
        assert!(dir.join("logos/a.png").exists() && dir.join("patterns/b.svg").exists());
        assert!(!dir.join("extra.exe").exists(), "一覧にないものは広げない");
        let second = pack(&[("logos/c.png", b"c")], &["logos/c.png"]);
        assert_eq!(install(&second, &dir).unwrap(), 1);
        assert!(dir.join("logos/c.png").exists() && !dir.join("logos/a.png").exists(), "前のものは置き換わる");
        // 外を指す名前は断る。
        let evil = pack(&[("../evil.png", b"x")], &["../evil.png"]);
        assert!(install(&evil, &dir).is_err());
        assert!(!root.join("evil.png").exists());
        // 見本パックではない ZIP も断る。
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            z.start_file("a.txt", zip::write::SimpleFileOptions::default()).unwrap();
            z.finish().unwrap();
        }
        assert!(install(&out.into_inner(), &dir).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
