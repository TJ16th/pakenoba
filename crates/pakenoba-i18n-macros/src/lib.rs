//! `t!` マクロ。画面の文字を、英語の文で書いて、いまの言語の訳に置き換える。
//!
//! ```ignore
//! t!("Save")                          // 埋め込む値がなければ &'static str
//! t!("Saved {name}.")                 // まわりの変数をそのまま使える（format! と同じ）
//! t!("{} of {}", done, total)         // 位置で渡す値
//! t!("Width {w:.1} mm", w = size[0])  // 名前で渡す値と、書式
//! ```
//!
//! 埋め込む値があるときは、英語の文と同じ書き方の `format!` で値を 1 つずつ文字にしてから、訳の文の
//! 同じ名前の場所（`{name}`）に入れる。位置で渡す値（`{}`）は、訳の文でも現れる順に入る。
//! 訳の文では、書式（`:.1` など）は書いても書かなくてもよい（値はもう文字になっている）。

use proc_macro::{Delimiter, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// 値どうしの区切り。値の中に出てこない文字（情報分離記号）。
const SEP: char = '\u{1f}';

#[proc_macro]
pub fn t(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(out) => out,
        Err(message) => error(&message),
    }
}

fn error(message: &str) -> TokenStream {
    format!("::core::compile_error!({message:?})").parse().unwrap_or_default()
}

fn expand(input: TokenStream) -> Result<TokenStream, String> {
    let mut tokens: Vec<TokenTree> = input.into_iter().collect();
    // マクロの中から渡されると、文字列が見えない括弧に包まれていることがある。
    if let Some(TokenTree::Group(g)) = tokens.first()
        && g.delimiter() == Delimiter::None
    {
        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
        tokens.splice(0..1, inner);
    }
    let Some(TokenTree::Literal(lit)) = tokens.first().cloned() else { return Err("t! の最初には、文字列を書いてください".into()) };
    let text = string_value(&lit.to_string()).ok_or("t! の最初には、文字列を書いてください")?;
    let rest: Vec<TokenTree> = tokens.into_iter().skip(1).collect();
    let pieces = placeholders(&text)?;

    let path = |name: &str| format!("::pakenoba_i18n::{name}");
    if pieces.is_empty() {
        if !rest.is_empty() {
            return Err("t! の文に {…} がないのに、値が渡されています".into());
        }
        // 訳を引くだけ。
        return Ok(wrap_call(&path("tr"), TokenStream::from(TokenTree::Literal(lit))));
    }

    // 名前で渡された値の名前（`name = 式`）。
    let explicit = named_args(&rest);
    // 値をまとめて 1 つの format! で文字にする。まわりの変数は、名前で渡す形に直す。
    let mut joined = String::new();
    let mut implicit = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            joined.push(SEP);
        }
        joined.push('{');
        joined.push_str(&piece.inner);
        joined.push('}');
        if let Some(name) = &piece.name
            && is_ident(name)
            && !explicit.contains(name)
            && !implicit.contains(name)
        {
            implicit.push(name.clone());
        }
    }
    let mut format_args: Vec<TokenTree> = vec![TokenTree::Literal(Literal::string(&joined))];
    if !rest.is_empty() {
        let mut rest = rest;
        if !matches!(rest.first(), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
            return Err("t! の文のあとは、カンマで区切って値を書いてください".into());
        }
        // 終わりのカンマは取る（あとで名前の値を足すため）。
        if matches!(rest.last(), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
            rest.pop();
        }
        format_args.extend(rest);
    }
    for name in &implicit {
        format_args.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
        format_args.push(TokenTree::Ident(Ident::new(name, Span::call_site())));
        format_args.push(TokenTree::Punct(Punct::new('=', Spacing::Alone)));
        format_args.push(TokenTree::Ident(Ident::new(name, Span::call_site())));
    }
    let format_call = wrap_call("::std::format!", format_args.into_iter().collect());
    // fill(英語の文, 訳の文, 値)
    let mut args: Vec<TokenTree> = vec![TokenTree::Literal(lit.clone()), TokenTree::Punct(Punct::new(',', Spacing::Alone))];
    args.extend(wrap_call(&path("tr"), TokenStream::from(TokenTree::Literal(lit))));
    args.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
    args.push(TokenTree::Punct(Punct::new('&', Spacing::Alone)));
    args.extend(format_call);
    Ok(wrap_call(&path("fill"), args.into_iter().collect()))
}

/// `callee(中身)` の呼び出し（`callee` は関数かマクロの道筋）。
fn wrap_call(callee: &str, inner: TokenStream) -> TokenStream {
    let mut out: TokenStream = callee.parse().unwrap_or_default();
    out.extend([TokenTree::Group(proc_macro::Group::new(Delimiter::Parenthesis, inner))]);
    out
}

/// 値の場所 1 つ。`inner` は `{` と `}` の間（`name:spec` など）、`name` は名前（位置で渡すものは None）。
struct Piece {
    inner: String,
    name: Option<String>,
}

/// 文の中の値の場所を、現れる順に。`{{` と `}}` は文字の `{` `}`。
fn placeholders(text: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '{' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => inner.push(c),
                        None => return Err(format!("t! の文の {{ が閉じていません: {text:?}")),
                    }
                }
                let name = inner.split(':').next().unwrap_or("").trim().to_owned();
                out.push(Piece { name: (!name.is_empty() && !name.chars().all(|c| c.is_ascii_digit())).then_some(name), inner });
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            _ => {}
        }
    }
    Ok(out)
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic()) && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// 値の並びのうち、`name = 式` の形で渡されている名前。
fn named_args(rest: &[TokenTree]) -> Vec<String> {
    let mut names = Vec::new();
    let mut at_start = true;
    let mut i = 0;
    while i < rest.len() {
        match &rest[i] {
            TokenTree::Punct(p) if p.as_char() == ',' => at_start = true,
            TokenTree::Ident(id) if at_start => {
                if let Some(TokenTree::Punct(eq)) = rest.get(i + 1)
                    && eq.as_char() == '='
                    && eq.spacing() == Spacing::Alone
                {
                    names.push(id.to_string());
                }
                at_start = false;
            }
            _ => at_start = false,
        }
        i += 1;
    }
    names
}

/// 文字列の字句（`"…"` か `r"…"`、`r#"…"#`）の中身。
fn string_value(token: &str) -> Option<String> {
    if let Some(raw) = token.strip_prefix('r') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let body = raw.get(hashes..)?.strip_prefix('"')?;
        let body = body.get(..body.len().checked_sub(1 + hashes)?)?;
        return Some(body.to_owned());
    }
    let body = token.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex.replace('_', ""), 16).ok()?)?);
            }
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
            }
            '\n' => {
                // 行の終わりの \ は、次の行の頭の空白ごと飛ばす。
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}
