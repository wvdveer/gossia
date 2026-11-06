//! The interface's texts in six languages (DESIGN.md, Web port, 0.3.0, step
//! 13f): English, French, Spanish, Portuguese, Japanese and Mandarin
//! (Simplified Chinese).
//!
//! The English text is the key. Code writes it as it always did, inside
//! [`tr!`]: `tr!("Save")`, or with named places for what changes,
//! `tr!("Export {tab} as {kind}", tab = name, kind = "PDF")`. Each other
//! language has a file in `locales/` from the English text to its own,
//! with the same named places, which it may put in another order. A text a
//! file lacks is shown in English, so a missing translation is never worse
//! than English.
//!
//! The language is one for the whole program, set at the start from the
//! preferences (or the system's language) and again when the Options change
//! it: [`set_language`]. It starts as English, so tests see English unless
//! they choose otherwise.
//!
//! A test reads the workspace's sources for every `tr!` text and checks
//! that each language's file has it, with the same places, and nothing
//! else (`tests/catalogues.rs`).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// A language of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    #[default]
    English,
    French,
    Spanish,
    Portuguese,
    Japanese,
    /// Mandarin, written in Simplified Chinese.
    Chinese,
}

impl Language {
    pub const ALL: [Language; 6] = [
        Language::English,
        Language::French,
        Language::Spanish,
        Language::Portuguese,
        Language::Japanese,
        Language::Chinese,
    ];

    /// Its BCP 47 tag, as the preferences keep it.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
            Language::Spanish => "es",
            Language::Portuguese => "pt",
            Language::Japanese => "ja",
            Language::Chinese => "zh-Hans",
        }
    }

    /// Its name in itself, as a list of languages shows it.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
            Language::Spanish => "Español",
            Language::Portuguese => "Português",
            Language::Japanese => "日本語",
            Language::Chinese => "简体中文",
        }
    }

    /// The language of a tag (`fr`, `fr-CA`, `pt_BR`, `zh-CN`), if it is
    /// one of the six. Every Chinese tag is taken as Simplified Chinese,
    /// the only Chinese there is.
    pub fn from_tag(tag: &str) -> Option<Language> {
        let primary = tag
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        Some(match primary.as_str() {
            "en" => Language::English,
            "fr" => Language::French,
            "es" => Language::Spanish,
            "pt" => Language::Portuguese,
            "ja" => Language::Japanese,
            "zh" => Language::Chinese,
            _ => return None,
        })
    }

    /// Whether its script needs a font for Chinese characters (and kana),
    /// which the bundled fonts do not have.
    pub fn needs_cjk_font(self) -> bool {
        matches!(self, Language::Japanese | Language::Chinese)
    }

    /// Its translations, from English, as the file has them.
    fn file(self) -> Option<&'static str> {
        Some(match self {
            Language::English => return None,
            Language::French => include_str!("../locales/fr.toml"),
            Language::Spanish => include_str!("../locales/es.toml"),
            Language::Portuguese => include_str!("../locales/pt.toml"),
            Language::Japanese => include_str!("../locales/ja.toml"),
            Language::Chinese => include_str!("../locales/zh-Hans.toml"),
        })
    }

    fn index(self) -> u8 {
        Language::ALL.iter().position(|&l| l == self).unwrap_or(0) as u8
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// A language for this thread only, over the program's: for tests,
    /// which run side by side in one program.
    static THREAD: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

/// The language the interface is shown in.
pub fn language() -> Language {
    THREAD.with(std::cell::Cell::get).unwrap_or_else(|| {
        Language::ALL[usize::from(CURRENT.load(Ordering::Relaxed)) % Language::ALL.len()]
    })
}

/// Shows this thread's texts in `language` (or, with none, the program's
/// language again), whatever the program's language is. For tests.
pub fn set_thread_language(language: Option<Language>) {
    THREAD.with(|t| t.set(language));
}

/// Shows the interface in `language` from now on. On a thread with a
/// language of its own ([`set_thread_language`]), only that thread's
/// changes, so that a test that switches languages leaves the others be.
pub fn set_language(language: Language) {
    let own = THREAD.with(|t| {
        let own = t.get().is_some();
        if own {
            t.set(Some(language));
        }
        own
    });
    if !own {
        CURRENT.store(language.index(), Ordering::Relaxed);
    }
}

/// The system's language, if it is one of the six; English otherwise. A
/// web view has none here: its host says the language.
pub fn system_language() -> Language {
    #[cfg(not(target_family = "wasm"))]
    {
        if let Some(language) = sys_locale::get_locales().find_map(|tag| Language::from_tag(&tag)) {
            return language;
        }
    }
    Language::English
}

/// The language a preference names: its tag, or none (or one not known)
/// for the system's.
pub fn chosen(preference: Option<&str>) -> Language {
    preference
        .and_then(Language::from_tag)
        .unwrap_or_else(system_language)
}

/// A language's translations, read from its file the first time.
pub fn catalogue(language: Language) -> &'static HashMap<String, String> {
    static CATALOGUES: [OnceLock<HashMap<String, String>>; 6] = [const { OnceLock::new() }; 6];
    CATALOGUES[usize::from(language.index())].get_or_init(|| {
        language
            .file()
            .map(|text| parse(text).unwrap_or_default())
            .unwrap_or_default()
    })
}

/// A translations file: a flat table of English texts to translations.
pub fn parse(text: &str) -> Result<HashMap<String, String>, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    table
        .into_iter()
        .map(|(key, value)| match value {
            toml::Value::String(s) => Ok((key, s)),
            other => Err(format!("{key}: a translation is a string, not {other}")),
        })
        .collect()
}

/// The text `english` in the interface's language: its translation, or the
/// English when there is none.
pub fn tr(english: &'static str) -> &'static str {
    match language() {
        Language::English => english,
        language => catalogue(language)
            .get(english)
            .map_or(english, String::as_str),
    }
}

/// A text with its named places (`{tab}`) filled in. A place not given is
/// left as it is; `{{` and `}}` are a brace.
pub fn fill(template: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(end) = tail.find('}').filter(|_| tail.starts_with('{'))
            && let Some((_, value)) = values.iter().find(|(name, _)| *name == &tail[1..end])
        {
            out.push_str(&value.to_string());
            rest = &tail[end + 1..];
            continue;
        }
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// The named places of a text, in order: `{tab}` gives `tab`.
pub fn places(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{{") {
            rest = after;
            continue;
        }
        match tail.find('}') {
            Some(end) => {
                out.push(&tail[1..end]);
                rest = &tail[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// A text of the interface in its language: `tr!("Save")` gives a
/// `&'static str`; with named places, `tr!("Open {name}", name = file)`
/// gives a `String` with them filled in.
#[macro_export]
macro_rules! tr {
    ($english:literal) => {
        $crate::tr($english)
    };
    ($english:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::fill(
            $crate::tr($english),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// Marks a text for the catalogues where it cannot be translated yet, as in
/// a constant table: `tr_noop!("The sum.")` is the English itself, and
/// `tr(text)` translates it where it is shown.
#[macro_export]
macro_rules! tr_noop {
    ($english:literal) => {
        $english
    };
}

/// A count with its noun as a whole phrase, so that each language words it
/// its own way: `one` for 1, as `tr!("1 value")`, and `many` otherwise, as
/// `tr!("{n} values")`, with `{n}` filled in.
pub fn counted(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_owned()
    } else {
        fill(many, &[("n", &n)])
    }
}

/// Texts joined as a list, "a, b and c" in English.
pub fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => tr!(
            "{list} and {last}",
            list = rest.join(tr!(", ")),
            last = last
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_lists_are_whole_phrases() {
        assert_eq!(counted(1, "1 value", "{n} values"), "1 value");
        assert_eq!(counted(3, "1 value", "{n} values"), "3 values");
        let items = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(and_list(&items), "a, b and c");
        assert_eq!(and_list(&items[..1]), "a");
    }

    #[test]
    fn tags_name_the_six_languages() {
        assert_eq!(Language::from_tag("fr-CA"), Some(Language::French));
        assert_eq!(
            Language::from_tag("pt_BR.UTF-8"),
            Some(Language::Portuguese)
        );
        assert_eq!(Language::from_tag("zh-TW"), Some(Language::Chinese));
        assert_eq!(Language::from_tag("ZH-Hans-CN"), Some(Language::Chinese));
        assert_eq!(Language::from_tag("de-DE"), None);
        for language in Language::ALL {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
        }
        assert_eq!(chosen(Some("ja")), Language::Japanese);
    }

    #[test]
    fn places_are_filled_by_name_in_any_order() {
        let name = "Summary";
        assert_eq!(
            fill("{kind} : {tab}", &[("tab", &name), ("kind", &"PDF")]),
            "PDF : Summary"
        );
        assert_eq!(fill("{{literal}} {n}", &[("n", &3)]), "{literal} 3");
        assert_eq!(fill("{missing}", &[]), "{missing}");
        assert_eq!(places("Export {tab} as {kind} {{not}}"), ["tab", "kind"]);
    }

    #[test]
    fn every_file_is_a_table_of_texts() {
        for language in Language::ALL {
            if let Some(text) = language.file() {
                parse(text).unwrap_or_else(|e| panic!("{}: {e}", language.tag()));
            }
        }
    }
}
