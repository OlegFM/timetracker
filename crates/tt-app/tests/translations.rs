//! The hand-maintained gettext catalogs stay in sync with the `@tr(...)` strings in `ui/`.
//!
//! Supported `@tr` forms: `@tr("msg")`, `@tr("ctx" => "msg")`, `@tr("one" | "many" % n)`
//! (and their combination), with optional format arguments after the strings.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const LANGS: [&str; 3] = ["ru", "zh_CN", "es"];

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Msg {
    ctx: String,
    id: String,
    plural: String,
}

#[derive(Debug, Default)]
struct PoEntry {
    msg: Msg,
    strs: Vec<String>,
    fuzzy: bool,
}

impl PoEntry {
    fn translated(&self) -> bool {
        !self.fuzzy && !self.strs.is_empty() && self.strs.iter().all(|s| !s.is_empty())
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn translations_dir() -> PathBuf {
    manifest_dir().join("..").join("..").join("translations")
}

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A leading `"…"` literal of `s` (after whitespace): its unescaped text and the rest.
fn string_literal(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start().strip_prefix('"')?;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some((unescape(&s[..i]), &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Every `@tr(...)` message in a `.slint` source.
fn tr_messages(src: &str) -> Vec<Msg> {
    src.match_indices("@tr(")
        .map(|(i, m)| {
            let rest = &src[i + m.len()..];
            let (first, rest) = string_literal(rest).expect("@tr( must start with a string");
            let (ctx, id, rest) = match rest.trim_start().strip_prefix("=>") {
                Some(after) => {
                    let (id, rest) = string_literal(after).expect("msgid after =>");
                    (first, id, rest)
                }
                None => (String::new(), first, rest),
            };
            let plural = match rest.trim_start().strip_prefix('|') {
                Some(after) => string_literal(after).expect("plural after |").0,
                None => String::new(),
            };
            Msg { ctx, id, plural }
        })
        .collect()
}

fn ui_messages() -> BTreeSet<Msg> {
    let mut files = Vec::new();
    slint_files(&manifest_dir().join("ui"), &mut files);
    files
        .iter()
        .flat_map(|f| tr_messages(&fs::read_to_string(f).unwrap()))
        .collect()
}

/// Minimal `.po` reader: entries separated by blank lines, multi-line strings, `#, fuzzy`.
fn parse_po(text: &str) -> Vec<PoEntry> {
    let mut entries = Vec::new();
    let mut cur = PoEntry::default();
    let mut last_kw = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            entries.push(std::mem::take(&mut cur));
            last_kw.clear();
            continue;
        }
        if line.starts_with('#') {
            cur.fuzzy |= line.starts_with("#,") && line.contains("fuzzy");
            continue;
        }
        let (kw, value) = if line.starts_with('"') {
            (last_kw.clone(), line)
        } else {
            let (kw, value) = line.split_once(' ').expect("keyword and value");
            (kw.to_owned(), value)
        };
        let (value, _) = string_literal(value).expect("quoted po value");
        match kw.as_str() {
            "msgctxt" => cur.msg.ctx += &value,
            "msgid" => cur.msg.id += &value,
            "msgid_plural" => cur.msg.plural += &value,
            _ => {
                let idx = match kw.strip_prefix("msgstr[") {
                    Some(n) => n
                        .trim_end_matches(']')
                        .parse::<usize>()
                        .expect("plural index"),
                    None if kw == "msgstr" => 0,
                    None => panic!("unknown po keyword {kw}"),
                };
                if cur.strs.len() <= idx {
                    cur.strs.resize(idx + 1, String::new());
                }
                cur.strs[idx] += &value;
            }
        }
        last_kw = kw;
    }
    entries.push(cur);
    // Drop the empty separators and the header entry (empty msgid).
    entries.retain(|e| !e.msg.id.is_empty());
    entries
}

/// `nplurals` from the `Plural-Forms` header (1 if absent, as in the template).
fn nplurals(text: &str) -> usize {
    text.split("nplurals=")
        .nth(1)
        .map(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().expect("nplurals value")
        })
        .unwrap_or(1)
}

/// Format placeholders (`{}`, `{0}`, `{n}`), sorted, with repeats.
fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end)
                if after[..end]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                out.push(format!("{{{}}}", &after[..end]));
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out.sort();
    out
}

/// Wrong number of msgstr forms, or placeholders that differ from the msgid's
/// (from msgid_plural for plural entries). Untranslated (empty) forms are skipped.
fn form_problems(e: &PoEntry, nplurals: usize) -> Vec<String> {
    let mut problems = Vec::new();
    let forms = if e.msg.plural.is_empty() { 1 } else { nplurals };
    if e.strs.len() != forms {
        problems.push(format!(
            "{:?}: {} msgstr, want {forms}",
            e.msg,
            e.strs.len()
        ));
    }
    let source = if e.msg.plural.is_empty() {
        &e.msg.id
    } else {
        &e.msg.plural
    };
    let want = placeholders(source);
    for (i, s) in e.strs.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        if placeholders(s) != want {
            problems.push(format!("{:?}: msgstr[{i}] placeholders differ", e.msg));
        }
    }
    problems
}

fn catalog(path: &Path) -> Vec<PoEntry> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse_po(&text)
}

fn po_path(lang: &str) -> PathBuf {
    translations_dir()
        .join(lang)
        .join("LC_MESSAGES")
        .join("tt-app.po")
}

fn pot_path() -> PathBuf {
    translations_dir().join("tt-app.pot")
}

#[test]
fn every_tr_string_is_translated() {
    let ui = ui_messages();
    assert!(!ui.is_empty(), "no @tr strings found");

    let mut problems = Vec::new();
    let pot: BTreeSet<Msg> = catalog(&pot_path()).into_iter().map(|e| e.msg).collect();
    for m in ui.iter().filter(|m| !pot.contains(m)) {
        problems.push(format!("tt-app.pot: missing {m:?}"));
    }
    for lang in LANGS {
        let po = catalog(&po_path(lang));
        for m in &ui {
            match po.iter().find(|e| &e.msg == m) {
                None => problems.push(format!("{lang}: missing {m:?}")),
                Some(e) if !e.translated() => problems.push(format!("{lang}: untranslated {m:?}")),
                Some(_) => {}
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn plural_forms_and_placeholders_match() {
    let mut problems = Vec::new();
    for lang in LANGS {
        let path = po_path(lang);
        let text = fs::read_to_string(&path).unwrap();
        let n = nplurals(&text);
        for e in parse_po(&text) {
            problems.extend(
                form_problems(&e, n)
                    .into_iter()
                    .map(|p| format!("{lang}: {p}")),
            );
        }
    }
    assert!(
        problems.is_empty(),
        "
{}",
        problems.join(
            "
"
        )
    );
}

#[test]
fn form_checks_catch_bad_entries() {
    assert_eq!(nplurals("\"Plural-Forms: nplurals=3; plural=0;\n\""), 3);
    assert_eq!(nplurals(""), 1);
    assert_eq!(
        placeholders("Create \"{}\" in {} ({n}, {0}) {not a placeholder"),
        ["{0}", "{n}", "{}", "{}"]
    );

    let po = parse_po(concat!(
        "msgid \"Create \\\"{}\\\"\"
msgstr \"Создать\"

",
        "msgid \"Fine {}\"
msgstr \"Хорошо {}\"

",
        "msgid \"{n} hour\"
msgid_plural \"{n} hours\"
msgstr[0] \"{n} час\"
msgstr[1] \"{n} часа\"
",
    ));
    assert_eq!(po.len(), 3);
    assert_eq!(form_problems(&po[0], 3).len(), 1, "missing placeholder");
    assert!(form_problems(&po[1], 3).is_empty());
    assert_eq!(form_problems(&po[2], 3).len(), 1, "2 of 3 plural forms");
    assert!(form_problems(&po[2], 2).is_empty());
}

#[test]
fn catalogs_have_no_stale_entries() {
    let ui = ui_messages();
    let mut stale = Vec::new();
    let paths = std::iter::once(pot_path()).chain(LANGS.map(po_path));
    for path in paths {
        for e in catalog(&path).iter().filter(|e| !ui.contains(&e.msg)) {
            stale.push(format!("{}: {:?}", path.display(), e.msg));
        }
    }
    assert!(stale.is_empty(), "\n{}", stale.join("\n"));
}

#[test]
fn parser_handles_context_and_plural_forms() {
    let src = r#"
        a: @tr("Pause");
        b: @tr("menu" => "Open");
        c: @tr("{n} hour" | "{n} hours" % n);
        d: @tr("Say \"{}\"", name);
    "#;
    let msgs = tr_messages(src);
    let m = |ctx: &str, id: &str, plural: &str| Msg {
        ctx: ctx.into(),
        id: id.into(),
        plural: plural.into(),
    };
    assert_eq!(
        msgs,
        [
            m("", "Pause", ""),
            m("menu", "Open", ""),
            m("", "{n} hour", "{n} hours"),
            m("", "Say \"{}\"", ""),
        ]
    );

    let po = parse_po(concat!(
        "msgid \"\"\nmsgstr \"\"\n\"Language: ru\\n\"\n\n",
        "msgctxt \"menu\"\nmsgid \"Open\"\nmsgstr \"\"\n\"Откр\"\n\"ыть\"\n\n",
        "#, fuzzy\nmsgid \"Pause\"\nmsgstr \"Пауза\"\n\n",
        "msgid \"{n} hour\"\nmsgid_plural \"{n} hours\"\nmsgstr[0] \"{n} час\"\nmsgstr[1] \"\"\n",
    ));
    assert_eq!(po.len(), 3);
    assert_eq!(
        (po[0].msg.clone(), po[0].strs.clone()),
        (m("menu", "Open", ""), vec!["Открыть".to_owned()])
    );
    assert!(po[0].translated());
    assert!(po[1].fuzzy && !po[1].translated());
    assert_eq!(po[2].msg, m("", "{n} hour", "{n} hours"));
    assert!(!po[2].translated());
}
