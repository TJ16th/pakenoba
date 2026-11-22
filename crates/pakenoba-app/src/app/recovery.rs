//! 落ちたときの控え。
//!
//! 決まり（`docs/project-file.md` の 6 章の 4）: 保存は使う人が行う。自動では、案件のファイルを書き換えない。
//! そのかわり、保存していない変更があるあいだは、決まった間隔で、別の場所に控えを書く。
//!
//! - 控えの場所は、使う人のデータのフォルダの中（Windows なら `%LOCALAPPDATA%\pakenoba\recovery`）。
//!   案件のフォルダには書かない（同期フォルダを汚さない、案件のファイルと取り違えない）。
//! - 控えは、案件のファイルと同じ形（`.pknb`）。横に、元の場所と日時を書いた小さなテキストを置く。
//! - 起動している間は、自分の控えに鍵をかけておく。次に起動したとき、鍵の外れた控えは、
//!   前回落ちたときに残ったものとみなして、開くか、捨てるかを聞く。鍵がかかっている控えは、
//!   ほかのウィンドウで動いているパケノバのものなので、触らない。
//! - 保存したら、または変更がなくなったら（元に戻した、保存しないで閉じた）、控えは消す。
//!   ふつうに閉じたときも消す。

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;

use pakenoba_core::project::{self, Project};

use super::{App, file};

/// 控えのテキストの、1 行目の印。
const MARK: &str = "pakenoba-recovery 1";

/// 前回、落ちたときに残った控え。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Orphan {
    id: String,
    /// 控えの案件のファイル。
    pub backup: PathBuf,
    /// 元の案件のファイルの場所。保存したことのない案件なら None。
    pub original: Option<PathBuf>,
    /// 控えを書いた日時。
    pub when: String,
}

impl Orphan {
    /// 画面に出す名前。
    pub fn name(&self) -> String {
        self.original.as_deref().and_then(Path::file_name).map_or_else(|| "名前なし".to_owned(), |n| n.to_string_lossy().into_owned())
    }
}

/// 落ちたときの控えにかかわる状態。
pub(super) struct Recovery {
    /// 控えを置くフォルダ。作れなかったときは None（控えを取らない）。
    dir: Option<PathBuf>,
    /// この起動の印。控えのファイルの名前に使う。
    id: String,
    /// 起動している間、かけておく鍵。
    _lock: Option<File>,
    /// 控えを書く間隔（秒）。
    interval: f64,
    /// 最後に控えを書いた（書き始めた）ときの、状態の印と時刻。
    last: Option<(u64, f64)>,
    /// 控えがあるか。
    present: bool,
    /// 書いている途中の控え。
    writing: Option<Receiver<Result<(), String>>>,
    /// 前回、落ちたときに残った控え。起動したときに見つけたもの。
    pub orphans: Vec<Orphan>,
}

/// 使う人のデータのフォルダの中の、控えの場所。
fn default_dir() -> Option<PathBuf> {
    Some(super::settings::data_dir()?.join("recovery"))
}

impl Recovery {
    /// 控えを使わない（自動のテストのとき。使う人のフォルダに書かないため）。
    pub fn off() -> Recovery {
        Recovery { dir: None, id: String::new(), _lock: None, interval: 30.0, last: None, present: false, writing: None, orphans: Vec::new() }
    }

    /// 起動したときに作る。控えのフォルダを用意し、自分の鍵をかけ、前回の控えを探す。
    pub fn start(dir: Option<PathBuf>, interval: f64) -> Recovery {
        let dir = dir.or_else(default_dir).filter(|dir| fs::create_dir_all(dir).is_ok());
        let id = format!("{}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S%3f"), std::process::id());
        let mut recovery = Recovery { dir, id, _lock: None, interval: interval.max(1.0), last: None, present: false, writing: None, orphans: Vec::new() };
        let Some(dir) = recovery.dir.clone() else { return recovery };
        // 前回の控えを、先に探す（自分の鍵を作る前に）。
        recovery.orphans = find_orphans(&dir);
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{}.lock", recovery.id)));
        match lock.map(|file| file.try_lock().map(|()| file)) {
            Ok(Ok(file)) => recovery._lock = Some(file),
            // 鍵をかけられない場所では、控えを取らない（ほかの起動と取り違えないため）。
            _ => recovery.dir = None,
        }
        recovery
    }

    fn path(&self, ext: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{}.{ext}", self.id)))
    }

    /// 自分の控えを消す。
    fn remove(&mut self) {
        for ext in [project::EXTENSION, "txt"] {
            if let Some(path) = self.path(ext) {
                let _ = fs::remove_file(path);
            }
        }
        self.present = false;
        self.last = None;
    }

    /// ふつうに終わるとき。控えと鍵を消す。
    pub fn finish(&mut self) {
        // 書いている途中の控えは、待ってから消す（消したあとに書き上がらないように）。
        if let Some(writing) = self.writing.take() {
            let _ = writing.recv_timeout(Duration::from_secs(10));
        }
        self.remove();
        if let Some(path) = self.path("lock") {
            self._lock = None;
            let _ = fs::remove_file(path);
        }
    }

    /// 前回の控えを、この起動のものとして引き継ぐ（開いた控えを、次に落ちたときにも残すため）。
    fn adopt(&mut self, orphan: &Orphan, print: u64, now: f64) {
        let (Some(backup), Some(note), Some(dir)) = (self.path(project::EXTENSION), self.path("txt"), self.dir.clone()) else { return };
        if fs::rename(&orphan.backup, &backup).is_ok() {
            let _ = fs::rename(dir.join(format!("{}.txt", orphan.id)), &note);
            self.present = true;
            self.last = Some((print, now));
        }
        let _ = fs::remove_file(dir.join(format!("{}.lock", orphan.id)));
    }

    /// 前回の控えを捨てる。
    pub fn discard(&mut self, orphan: &Orphan) {
        let Some(dir) = &self.dir else {
            let _ = fs::remove_file(&orphan.backup);
            return;
        };
        for ext in [project::EXTENSION, "txt", "lock"] {
            let _ = fs::remove_file(dir.join(format!("{}.{ext}", orphan.id)));
        }
    }
}

/// 鍵の外れた控えを探す。控えのない鍵だけが残っていれば、片付ける。
fn find_orphans(dir: &Path) -> Vec<Orphan> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut orphans = Vec::new();
    let mut ids: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            (ext == project::EXTENSION || ext == "lock").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        let lock = dir.join(format!("{id}.lock"));
        // ほかのパケノバが動いていて、鍵がかかっている控えは、触らない。
        if lock.exists() {
            match OpenOptions::new().write(true).open(&lock).map(|file| file.try_lock()) {
                Ok(Ok(())) => {}
                _ => continue,
            }
        }
        let backup = dir.join(format!("{id}.{}", project::EXTENSION));
        if !backup.exists() {
            // 控えを書く前に落ちたか、ふつうに終わったあとの鍵だけ。片付ける。
            let _ = fs::remove_file(&lock);
            continue;
        }
        let note = fs::read_to_string(dir.join(format!("{id}.txt"))).unwrap_or_default();
        let mut lines = note.lines();
        let (original, when) = if lines.next() == Some(MARK) {
            let original = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
            (original, lines.next().unwrap_or_default().to_owned())
        } else {
            (None, String::new())
        };
        orphans.push(Orphan { id, backup, original, when });
    }
    // 新しいものから。
    orphans.sort_by(|a, b| b.when.cmp(&a.when));
    orphans
}

/// 控えを書く（裏の仕事から呼ぶ）。案件と、横のテキスト。どちらも、書ききってから入れ替える。
fn write_backup(project: &Project, backup: &Path, note: &Path, original: Option<&Path>) -> Result<(), String> {
    project.save(backup).map_err(|e| e.to_string())?;
    let text = format!("{MARK}\n{}\n{}\n", original.map(|p| p.display().to_string()).unwrap_or_default(), file::now_local());
    project::replace_file(note, |out| {
        use std::io::Write;
        out.write_all(text.as_bytes())?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

impl App {
    /// 毎回、画面を描く前に行う。保存していない変更があれば、決まった間隔で控えを書く。なければ、控えを消す。
    pub(super) fn recovery_frame(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if let Some(writing) = &self.recovery.writing {
            match writing.try_recv() {
                Ok(result) => {
                    self.recovery.writing = None;
                    if result.is_ok() {
                        self.recovery.present = true;
                    }
                }
                Err(TryRecvError::Disconnected) => self.recovery.writing = None,
                Err(TryRecvError::Empty) => return,
            }
        }
        if self.recovery.dir.is_none() {
            return;
        }
        if !self.unsaved() {
            if self.recovery.present || self.recovery.last.is_some() {
                self.recovery.remove();
            }
            return;
        }
        let print = self.fingerprint();
        let since = match self.recovery.last {
            // 前の控えから変わっていなければ、書かない。
            Some((last, _)) if last == print => return,
            Some((_, at)) => now - at,
            // はじめての変更。間隔が過ぎてから書く。
            None => {
                self.recovery.last = Some((0, now));
                0.0
            }
        };
        let wait = self.recovery.interval - since;
        if wait > 0.0 {
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
            return;
        }
        self.write_recovery(Some(ctx), now);
    }

    /// 控えを書く。`ctx` があれば裏で書き、なければ（テスト）その場で書ききる。
    fn write_recovery(&mut self, ctx: Option<&egui::Context>, now: f64) {
        let (Some(backup), Some(note)) = (self.recovery.path(project::EXTENSION), self.recovery.path("txt")) else { return };
        let Ok(project) = self.project() else { return };
        self.recovery.last = Some((self.fingerprint(), now));
        let original = self.file.path.clone();
        match ctx {
            Some(ctx) => {
                let (sender, receiver) = channel();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = sender.send(write_backup(&project, &backup, &note, original.as_deref()));
                    ctx.request_repaint();
                });
                self.recovery.writing = Some(receiver);
            }
            None => {
                if write_backup(&project, &backup, &note, original.as_deref()).is_ok() {
                    self.recovery.present = true;
                }
            }
        }
    }

    /// 起動したときに、前回の控えがあれば、開くか捨てるかを聞く。
    pub(super) fn recovery_dialog(&mut self, ctx: &egui::Context) {
        if self.recovery.orphans.is_empty() {
            return;
        }
        let orphans = self.recovery.orphans.clone();
        let mut chosen: Option<(usize, bool)> = None;
        let response = egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.strong("前回、保存しないまま終わった案件があります");
            ui.add_space(4.0);
            ui.label("パケノバが途中で終わったときに残した控えです。開いて、保存し直せます。");
            ui.add_space(6.0);
            for (index, orphan) in orphans.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(orphan.name());
                        let when = file::show_time(&orphan.when).map_or_else(String::new, |t| format!("控えの日時: {t}"));
                        let place = orphan.original.as_ref().map_or_else(|| "まだ保存したことのない案件".to_owned(), |p| p.display().to_string());
                        ui.small(format!("{place}\n{when}"));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("開く").clicked() {
                            chosen = Some((index, true));
                        }
                        if ui.button("捨てる").on_hover_text("控えを消します。元に戻せません").clicked() {
                            chosen = Some((index, false));
                        }
                    });
                });
                ui.separator();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.button("あとで決める").on_hover_text("控えは残しておき、次に起動したときにまた聞きます").clicked()).inner
        });
        let later = response.inner || response.should_close();
        match chosen {
            Some((index, open)) => {
                let orphan = self.recovery.orphans.remove(index);
                if open {
                    // 開けるのは 1 つだけ。ほかの控えは、次に起動したときにまた聞く。
                    self.recovery.orphans.clear();
                    self.open_recovered(&orphan, ctx.input(|i| i.time));
                } else {
                    self.recovery.discard(&orphan);
                }
            }
            None if later => self.recovery.orphans.clear(),
            None => {}
        }
    }

    /// 控えを開く。元の案件の場所につなぎ、保存していない状態にする（保存すれば、元のファイルが新しくなる）。
    fn open_recovered(&mut self, orphan: &Orphan, now: f64) {
        if self.open_backup(&orphan.backup, orphan.original.clone(), &orphan.name()) {
            let print = self.fingerprint();
            self.recovery.adopt(orphan, print, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pakenoba-recovery-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// 柄を 1 つ置いた（保存していない変更がある）アプリ。
    fn app_with_change(dir: &Path) -> App {
        let mut app = App::new(&Initial::default());
        app.recovery = Recovery::start(Some(dir.to_owned()), 30.0);
        app.add_decal(crate::paint::Picture::Logo, [46.0, 46.0]);
        assert!(app.unsaved());
        app
    }

    #[test]
    fn 動いている間の控えは_ほかの起動から見えない() {
        let dir = scratch("live");
        let mut first = app_with_change(&dir);
        first.write_recovery(None, 0.0);
        assert!(files(&dir).iter().any(|n| n.ends_with(".pknb")));
        // 同じ時に、もう 1 つ起動しても、動いている方の控えは「落ちた」と扱わない。
        let second = Recovery::start(Some(dir.clone()), 30.0);
        assert!(second.orphans.is_empty());
        // ふつうに終わると、控えも鍵も消える。
        first.recovery.finish();
        drop(second);
        let third = Recovery::start(Some(dir.clone()), 30.0);
        assert!(third.orphans.is_empty());
        drop(third);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 落ちたあとに起動すると_控えを開いて元に戻せる() {
        let dir = scratch("crash");
        let mut before = app_with_change(&dir);
        before.decals[0].decal.at = [0.3, 0.7];
        before.file.path = Some(dir.join("箱.pknb"));
        before.write_recovery(None, 0.0);
        let decals = before.decals.iter().map(|d| (d.decal, d.back)).collect::<Vec<_>>();
        // 落ちた（後片付けをしないまま、鍵が外れた）。
        drop(before);

        let mut after = App::new(&Initial::default());
        after.recovery = Recovery::start(Some(dir.clone()), 30.0);
        assert_eq!(after.recovery.orphans.len(), 1);
        let orphan = after.recovery.orphans[0].clone();
        assert_eq!(orphan.original.as_deref(), Some(dir.join("箱.pknb").as_path()));
        assert_eq!(orphan.name(), "箱.pknb");
        assert!(file::show_time(&orphan.when).is_some());

        after.open_recovered(&orphan, 0.0);
        assert_eq!(after.decals.iter().map(|d| (d.decal, d.back)).collect::<Vec<_>>(), decals);
        // 元の場所につながり、保存していない状態。
        assert_eq!(after.file.path.as_deref(), Some(dir.join("箱.pknb").as_path()));
        assert!(after.unsaved());
        // 控えは、この起動のものとして引き継いだ。次に起動しても、落ちた控えとは扱わない。
        assert_eq!(files(&dir).iter().filter(|n| n.ends_with(".pknb")).count(), 1);
        assert!(Recovery::start(Some(dir.clone()), 30.0).orphans.is_empty());

        // 変更がなくなると（ここでは、開き直した状態を保存したことにする）、控えは消える。
        after.mark_saved();
        after.recovery.remove();
        assert_eq!(files(&dir).iter().filter(|n| n.ends_with(".pknb")).count(), 0);
        after.recovery.finish();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 捨てた控えは_消える() {
        let dir = scratch("discard");
        let mut before = app_with_change(&dir);
        before.write_recovery(None, 0.0);
        drop(before);
        let mut after = Recovery::start(Some(dir.clone()), 30.0);
        assert_eq!(after.orphans.len(), 1);
        assert_eq!(after.orphans[0].name(), "名前なし");
        let orphan = after.orphans.remove(0);
        after.discard(&orphan);
        after.finish();
        assert!(files(&dir).is_empty(), "{:?}", files(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 控えのない鍵だけが残っていたら_片付ける() {
        let dir = scratch("stale");
        fs::write(dir.join("20260101-000000000-1.lock"), "").unwrap();
        let recovery = Recovery::start(Some(dir.clone()), 30.0);
        assert!(recovery.orphans.is_empty());
        assert!(!dir.join("20260101-000000000-1.lock").exists());
        drop(recovery);
        fs::remove_dir_all(&dir).unwrap();
    }
}
