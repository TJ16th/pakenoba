//! 案件をまたいで覚えておく設定。コレクション（登録したフォルダ）、最近使った案件、最近取り込んだ画像。
//!
//! 使う人のデータのフォルダの `pakenoba/settings.toml` に、人が読める形で書く。
//! 読めないとき（壊れた、新しい版で書かれた）は、空から始めて、元のファイルは上書きしない。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 使う人のデータのフォルダの中の、パケノバの場所。
pub(super) fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    }?;
    Some(base.join("pakenoba"))
}

/// 最近取り込んだ画像を、いくつ覚えておくか。
const RECENT_ASSETS: usize = 20;

/// 登録したフォルダ 1 つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Collection {
    pub path: PathBuf,
}

impl Collection {
    /// 画面に出す名前（フォルダの名前）。
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    /// 最近使った案件（新しい順）。
    #[serde(default)]
    recent: Vec<PathBuf>,
    /// 最近取り込んだ画像（新しい順）。どの案件でも、素材の欄に出す。
    #[serde(default)]
    recent_assets: Vec<PathBuf>,
    #[serde(default)]
    collection: Vec<Collection>,
    /// 柄の素材を直すアプリ（なければ、OS で決まっているアプリ）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    editor: Option<PathBuf>,
}

pub(super) struct Settings {
    /// 書く場所。None なら覚えない（自動のテストのとき）。
    file: Option<PathBuf>,
    /// 読めなかったときは、書かない（読めなかったファイルを消さないため）。
    writable: bool,
    pub collections: Vec<Collection>,
    pub recent: Vec<PathBuf>,
    pub recent_assets: Vec<PathBuf>,
    pub editor: Option<PathBuf>,
}

impl Settings {
    /// 覚えない（自動のテストのとき。使う人のフォルダに書かないため）。
    pub fn off() -> Settings {
        Settings { file: None, writable: false, collections: Vec::new(), recent: Vec::new(), recent_assets: Vec::new(), editor: None }
    }

    /// 起動したときに読む。`file` がなければ、いつもの場所。
    pub fn load(file: Option<PathBuf>) -> Settings {
        let Some(file) = file.or_else(|| data_dir().map(|dir| dir.join("settings.toml"))) else { return Settings::off() };
        match fs::read_to_string(&file) {
            Ok(text) => match toml::from_str::<Stored>(&text) {
                Ok(stored) => Settings { file: Some(file), writable: true, collections: stored.collection, recent: stored.recent, recent_assets: stored.recent_assets, editor: stored.editor },
                Err(e) => {
                    eprintln!("設定（{}）を読めませんでした。今回は覚えません: {e}", file.display());
                    Settings { file: Some(file), writable: false, collections: Vec::new(), recent: Vec::new(), recent_assets: Vec::new(), editor: None }
                }
            },
            // まだない。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings { file: Some(file), writable: true, collections: Vec::new(), recent: Vec::new(), recent_assets: Vec::new(), editor: None },
            Err(e) => {
                eprintln!("設定（{}）を読めませんでした。今回は覚えません: {e}", file.display());
                Settings { file: Some(file), writable: false, collections: Vec::new(), recent: Vec::new(), recent_assets: Vec::new(), editor: None }
            }
        }
    }

    /// いまの設定を書く。書けなかったら、そのわけを返す。
    pub fn save(&self) -> Result<(), String> {
        let (Some(file), true) = (&self.file, self.writable) else { return Ok(()) };
        let text = toml::to_string(&Stored { recent: self.recent.clone(), recent_assets: self.recent_assets.clone(), collection: self.collections.clone(), editor: self.editor.clone() }).map_err(|e| e.to_string())?;
        write_text(file, &format!("# パケノバの設定。アプリが書き換えます。\n{text}")).map_err(|e| format!("設定を保存できませんでした: {e}"))
    }

    /// 取り込んだ画像を、最近使ったものの先頭に入れる（20 件まで）。
    pub fn used_asset(&mut self, path: &Path) {
        let path = fs::canonicalize(path).map(|p| super::file::simplify(&p)).unwrap_or_else(|_| path.to_owned());
        self.recent_assets.retain(|p| *p != path);
        self.recent_assets.insert(0, path);
        self.recent_assets.truncate(RECENT_ASSETS);
    }

    /// フォルダを登録する。すでにあれば何もしない（false）。
    pub fn add(&mut self, path: PathBuf) -> bool {
        if self.collections.iter().any(|c| same_path(&c.path, &path)) {
            return false;
        }
        self.collections.push(Collection { path });
        true
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 書きかけで落ちても壊れないように、別の名前で書いてから置き換える。
fn write_text(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = file.with_extension(format!("toml.tmp-{}", std::process::id()));
    let result = (|| {
        let mut out = fs::File::create(&temp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()
    })();
    match result.and_then(|()| fs::rename(&temp, file)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pakenoba-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 登録したフォルダを_次に起動したときも覚えている() {
        let dir = temp_dir("roundtrip");
        let file = dir.join("settings.toml");
        let mut settings = Settings::load(Some(file.clone()));
        assert!(settings.collections.is_empty());
        assert!(settings.add(dir.join("ロゴ集")));
        assert!(!settings.add(dir.join("ロゴ集")), "同じフォルダは二重に登録しない");
        settings.recent = vec![dir.join("箱.pknb")];
        for i in 0..25 {
            settings.used_asset(&dir.join(format!("絵{i}.png")));
        }
        settings.used_asset(&dir.join("絵3.png"));
        settings.save().unwrap();
        let again = Settings::load(Some(file));
        assert_eq!(again.recent, vec![dir.join("箱.pknb")]);
        assert_eq!(again.recent_assets.len(), 20);
        assert_eq!(again.recent_assets[0], dir.join("絵3.png"));
        assert_eq!(again.recent_assets[1], dir.join("絵24.png"));
        assert_eq!(again.collections, vec![Collection { path: dir.join("ロゴ集") }]);
        assert_eq!(again.collections[0].name(), "ロゴ集");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 読めない設定は_上書きしない() {
        let dir = temp_dir("broken");
        let file = dir.join("settings.toml");
        fs::write(&file, "collection = 3 これは壊れている").unwrap();
        let mut settings = Settings::load(Some(file.clone()));
        assert!(settings.collections.is_empty());
        settings.add(dir.clone());
        settings.save().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "collection = 3 これは壊れている");
        fs::remove_dir_all(dir).unwrap();
    }
}
