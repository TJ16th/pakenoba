//! コードの中の t!("…") と tr("…") の文が、すべての訳の表にあるか。訳の表に、使っていない文が残っていないか。
//!
//! 英語は基準の言葉なので、英語の表（en.tsv）は、パネルの名前などの別の文脈だけを持つ。それ以外の言語の表は、
//! 画面の文字をすべて持つ。訳を足し忘れると、この試験が、どのファイルのどの文かを知らせる。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    let mut stack = vec![root.join("pakenoba-app/src"), root.join("pakenoba-core/src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}

/// 文字列の字句の中身（`"` のあとから）を読んで、値と、閉じた `"` のあとの位置を返す。
fn literal(text: &str) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, i + 1)),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '0' => out.push('\0'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            c => out.push(c),
        }
    }
    None
}

/// テストのモジュールより前のコード（試験の中の文字は、訳さない）。
fn code_part(text: &str) -> &str {
    text.find("\nmod tests {").map_or(text, |at| &text[..at])
}

/// t!("…") と tr("…") の文と、そのほかの文字列の字句すべて。
fn scan() -> (BTreeSet<(String, String)>, BTreeSet<String>) {
    let mut wanted = BTreeSet::new();
    let mut all = BTreeSet::new();
    for path in sources() {
        // 文字の字句の '"' と、生の文字列 r"…" は、読み方を狂わせるので、先に除く。
        let text = std::fs::read_to_string(&path).unwrap().replace("'\"'", "' '").replace("r\"\\\\?\\\"", "\"\"");
        let code = code_part(&text);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut rest = code;
        while let Some(at) = rest.find('"') {
            let before = &rest[..at];
            let Some((value, end)) = literal(&rest[at + 1..]) else { break };
            // `format!(` や `str(` などの、名前の終わりが同じものは除く。
            let call = |name: &str| before.strip_suffix(name).is_some_and(|head| !head.ends_with(|c: char| c.is_alphanumeric() || c == '_'));
            if call("t!(") || call("tr(") {
                wanted.insert((value.clone(), name.clone()));
            }
            all.insert(value);
            rest = &rest[at + 1 + end..];
        }
    }
    (wanted, all)
}

#[test]
fn コードの文は_すべての言語の表にある() {
    let (wanted, _) = scan();
    assert!(wanted.len() > 300, "文が少なすぎる（読み方がおかしい）: {}", wanted.len());
    for lang in pakenoba_i18n::LANGS.iter().filter(|l| l.code != "en") {
        pakenoba_i18n::set(lang.code);
        let missing: Vec<_> = wanted.iter().filter(|(source, _)| pakenoba_i18n::every_has(lang.code, source).not()).collect();
        assert!(missing.is_empty(), "{} の表にない文 {} 個:\n{}", lang.code, missing.len(), missing.iter().map(|(s, f)| format!("  {f}: {s:?}")).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn 表の文は_どれもコードで使っている() {
    let (_, all) = scan();
    for lang in pakenoba_i18n::LANGS.iter().filter(|l| l.code != "en") {
        let unused: Vec<String> = pakenoba_i18n::sources_of(lang.code).into_iter().filter(|s| !all.contains(s)).collect();
        assert!(unused.is_empty(), "{} の表で、使っていない文 {} 個:\n{}", lang.code, unused.len(), unused.join("\n"));
    }
}

trait Not {
    fn not(self) -> bool;
}
impl Not for bool {
    fn not(self) -> bool {
        !self
    }
}
