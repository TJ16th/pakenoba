//! パケノバの多言語化。
//!
//! 画面の文字は、コードの中では英語で書き、[`t!`] で包む。いまの言語の訳は、言語ごとの訳の表
//! （`catalogs/<言語>.tsv`）から引く。訳がなければ英語のまま出る。どこも失敗しない。
//!
//! 訳の表の形は、VectorCraft と同じ（1 行に `文脈<TAB>英語<TAB>訳`）。文脈は、ふつうの画面の文字なら空。
//! 同じ英語でも意味が違うときに、文脈で分ける。パネルの名前と型の名前は、文脈 `panel` と `shape` で、
//! 日本語の名前（案件のファイルに書く名前）から各言語へ訳す（[`trc`]）。
//! 表の中の `\t`、`\n`、`\\` は、タブ、改行、`\`。`#` で始まる行と空の行は読まない。
//!
//! 言語を足すには、`catalogs/` に表を置き、[`LANGS`] に 1 行足す。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use pakenoba_i18n_macros::t;

// t! が作るコードは ::pakenoba_i18n を指すので、この中の試験でも同じ名前で引けるようにする。
extern crate self as pakenoba_i18n;

/// 言語の一覧。コード（BCP 47 の言語の部分）、その言語での名前、訳の表。
pub const LANGS: [Lang; 2] = [
    Lang { code: "en", name: "English", catalog: include_str!("../catalogs/en.tsv") },
    Lang { code: "ja", name: "日本語", catalog: include_str!("../catalogs/ja.tsv") },
];

/// 言語 1 つ。
#[derive(Debug)]
pub struct Lang {
    pub code: &'static str,
    pub name: &'static str,
    catalog: &'static str,
}

/// いまの言語（[`LANGS`] の番号）。はじめは英語。アプリが起動したときに決める。
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// いまの言語のコード。
pub fn current() -> &'static str {
    LANGS[CURRENT.load(Ordering::Relaxed).min(LANGS.len() - 1)].code
}

/// 言語を変える。知らないコードなら英語。`ja-JP` のような地域つきのコードも受ける。
pub fn set(code: &str) {
    CURRENT.store(find(code).unwrap_or(0), Ordering::Relaxed);
}

/// コード（`ja`、`ja-JP`、`ja_JP.UTF-8` など）に合う言語の番号。
fn find(code: &str) -> Option<usize> {
    let base = code.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    LANGS.iter().position(|l| l.code == base)
}

/// OS の言語のうち、パケノバにあるもの。なければ英語。
pub fn detect() -> &'static str {
    sys_locale::get_locales().find_map(|locale| find(&locale)).map_or("en", |i| LANGS[i].code)
}

type Table = HashMap<(String, String), String>;

/// 言語ごとの訳の表。はじめて使うときに読む。
fn table(index: usize) -> &'static Table {
    static TABLES: [OnceLock<Table>; LANGS.len()] = [const { OnceLock::new() }; LANGS.len()];
    TABLES[index].get_or_init(|| parse(LANGS[index].catalog).0)
}

/// 訳の表を読む。読めなかった行は、行の番号と理由を返す（試験で見る）。
pub fn parse(text: &str) -> (Table, Vec<String>) {
    let mut table = Table::new();
    let mut problems = Vec::new();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (i, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        let [context, source, translation] = columns[..] else {
            problems.push(format!("{} 行目: 3 つの列（文脈、英語、訳）になっていません", i + 1));
            continue;
        };
        let key = (unescape(context), unescape(source));
        if table.contains_key(&key) {
            problems.push(format!("{} 行目: 同じ文が 2 回あります（はじめのものを使います）", i + 1));
            continue;
        }
        table.insert(key, unescape(translation));
    }
    (table, problems)
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// 英語の文 `source` の、いまの言語の訳。訳がなければ `source`。
pub fn tr(source: &str) -> &str {
    trc("", source)
}

/// 文脈 `context` の中での、`source` の訳。パネルの名前（`panel`）などに使う。
pub fn trc<'a>(context: &str, source: &'a str) -> &'a str {
    let index = CURRENT.load(Ordering::Relaxed).min(LANGS.len() - 1);
    // 表を引くために、キーを作る。数百の文なので、毎回作っても軽い。
    match table(index).get(&(context.to_owned(), source.to_owned())) {
        Some(found) if !found.is_empty() => found.as_str(),
        _ => source,
    }
}

/// 言語 `code` の表に、文脈なしの `source` があるか（試験で、訳の抜けを見るため）。
#[doc(hidden)]
pub fn every_has(code: &str, source: &str) -> bool {
    find(code).is_some_and(|i| table(i).contains_key(&(String::new(), source.to_owned())))
}

/// 言語 `code` の表の、文脈なしの英語の文すべて（試験で、使っていない文を見るため）。
#[doc(hidden)]
pub fn sources_of(code: &str) -> Vec<String> {
    find(code).map(|i| table(i).keys().filter(|(c, _)| c.is_empty()).map(|(_, s)| s.clone()).collect()).unwrap_or_default()
}

/// `source` と、そのすべての言語での訳。ファイルに書いた名前（PSD のレイヤーの名前など）を、
/// どの言語で書かれていても見分けるために使う。
pub fn every(source: &'static str) -> Vec<&'static str> {
    let mut out = vec![source];
    for index in 0..LANGS.len() {
        if let Some(found) = table(index).get(&(String::new(), source.to_owned()))
            && !found.is_empty()
            && !out.contains(&found.as_str())
        {
            out.push(found.as_str());
        }
    }
    out
}

/// `t!` の値の入れ方。`source` は英語の文、`translated` は訳の文、`values` は、英語の文の値の場所に
/// 現れる順に、区切り（U+001F）でつないだ値。訳の文の `{name}` に同じ名前の値を、`{}` に位置の値を入れる。
#[doc(hidden)]
pub fn fill(source: &str, translated: &str, values: &str) -> String {
    let mut values = values.split('\u{1f}');
    let mut named: HashMap<&str, &str> = HashMap::new();
    let mut positional: Vec<&str> = Vec::new();
    walk(source, |piece| {
        if let Piece::Slot(key) = piece {
            let value = values.next().unwrap_or("");
            match key {
                Key::Name(name) => {
                    named.insert(name, value);
                }
                Key::Position(None) => positional.push(value),
                Key::Position(Some(n)) => {
                    if positional.len() <= n {
                        positional.resize(n + 1, "");
                    }
                    positional[n] = value;
                }
            }
        }
    });
    let mut out = String::with_capacity(translated.len() + 16);
    let mut next = 0;
    walk(translated, |piece| match piece {
        Piece::Text(text) => out.push_str(text),
        Piece::Slot(Key::Name(name)) => match named.get(name) {
            Some(value) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        },
        Piece::Slot(Key::Position(at)) => {
            let n = at.unwrap_or_else(|| {
                next += 1;
                next - 1
            });
            out.push_str(positional.get(n).copied().unwrap_or(""));
        }
    });
    out
}

enum Key<'a> {
    Name(&'a str),
    Position(Option<usize>),
}

enum Piece<'a> {
    Text(&'a str),
    Slot(Key<'a>),
}

/// 文を、ふつうの文字と値の場所に分けて、順に `each` へ渡す。`{{` と `}}` は文字の `{` と `}`。
fn walk<'a>(text: &'a str, mut each: impl FnMut(Piece<'a>)) {
    let mut rest = text;
    while !rest.is_empty() {
        let Some(at) = rest.find(['{', '}']) else {
            each(Piece::Text(rest));
            return;
        };
        if at > 0 {
            each(Piece::Text(&rest[..at]));
        }
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            each(Piece::Text(&tail[..1]));
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            each(Piece::Text("}"));
            rest = after;
            continue;
        }
        let Some(close) = tail.find('}') else {
            each(Piece::Text(tail));
            return;
        };
        let inner = &tail[1..close];
        let name = inner.split(':').next().unwrap_or("").trim();
        let key = if name.is_empty() {
            Key::Position(None)
        } else if let Ok(n) = name.parse() {
            Key::Position(Some(n))
        } else {
            Key::Name(name)
        };
        each(Piece::Slot(key));
        rest = &tail[close + 1..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 値を名前と位置で入れ替えられる() {
        assert_eq!(fill("Saved {name} ({} of {})", "{name} を保存（{} 件中 {} 件目）", "a.pknb\u{1f}2\u{1f}5"), "a.pknb を保存（2 件中 5 件目）");
        // 訳の文で順番を入れ替える。
        assert_eq!(fill("{a} then {b}", "{b} のあとに {a}", "x\u{1f}y"), "y のあとに x");
        // 書式を書いた訳の文でも、値はそのまま入る。
        assert_eq!(fill("Width {w:.1} mm", "幅 {w:.1} mm", "12.5"), "幅 12.5 mm");
        // 波かっこそのもの。
        assert_eq!(fill("{{x}} = {x}", "{{x}} は {x}", "3"), "{x} は 3");
    }

    #[test]
    fn 訳の表は全部読めて_同じ文が2回ない() {
        for lang in &LANGS {
            let (_, problems) = parse(lang.catalog);
            assert!(problems.is_empty(), "{}: {problems:?}", lang.code);
        }
    }

    #[test]
    fn 言語のコードは地域つきでも受ける() {
        assert_eq!(find("ja-JP"), Some(1));
        assert_eq!(find("ja_JP.UTF-8"), Some(1));
        assert_eq!(find("EN-us"), Some(0));
        assert_eq!(find("xx"), None);
        assert_eq!(unescape(r"a\tb\nc\\d"), "a\tb\nc\\d");
    }
}
