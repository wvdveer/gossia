//! The interface's language in the window (DESIGN.md, step 13f): which one,
//! and the font Japanese and Chinese need.
//!
//! The texts themselves are `gossia-i18n`'s. The language is set before the
//! window opens (from the preferences, or the system's) and again when the
//! Options change it (`apply_appearance`). Japanese and Chinese need a font
//! with their characters, which the bundled fonts lack: on the desktop the
//! system's own is used (owner, 2026-11-06), so the program grows by
//! nothing; it is added behind each family's own fonts, for the window's
//! texts and for data in those scripts alike. A web view cannot read the
//! system's fonts, so VS Code ships Noto Sans JP and SC beside the wasm and
//! loads the one wanted when it is first wanted (`Host::load_font`).

use gossia_i18n::tr;
use std::path::PathBuf;

use gossia_i18n::Language;

use crate::app::GossiaApp;

/// A font file for Japanese or Chinese, the system's or the host's: its
/// bytes, and which face of a collection.
#[derive(Clone)]
pub struct SystemFont {
    pub bytes: Vec<u8>,
    pub index: u32,
}

impl std::fmt::Debug for SystemFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SystemFont({} bytes, face {})",
            self.bytes.len(),
            self.index
        )
    }
}

impl GossiaApp {
    /// The language a preference means: the one chosen, or the system's
    /// (the host's, in a host that says one).
    pub fn language_for(&self, preference: Option<Language>) -> Language {
        preference.unwrap_or_else(|| {
            crate::host::current()
                .and_then(|h| h.language())
                .and_then(|tag| Language::from_tag(&tag))
                .unwrap_or_else(gossia_i18n::system_language)
        })
    }

    /// Gives the window the font the language needs, or takes it away when
    /// it no longer needs one. Each frame; it changes the fonts only when
    /// the language's need changes.
    pub(crate) fn apply_cjk_font(&mut self, ctx: &egui::Context) {
        let language = gossia_i18n::language();
        let want = language.needs_cjk_font().then_some(language);
        if self.cjk_font_for == Some(want) {
            return;
        }
        let font = match want {
            None => None,
            Some(language) => match font_for(language) {
                // Tried again next frame; the host repaints when it comes.
                Found::Waiting => return,
                Found::Font(font) => Some(font),
                Found::Missing(why) => {
                    self.notice = Some(why);
                    None
                }
            },
        };
        self.cjk_font_for = Some(want);
        crate::style::set_fonts_with(ctx, font);
    }
}

/// Where the font for a language stands.
enum Found {
    Font(SystemFont),
    /// The host is loading it.
    Waiting,
    /// There is none: the notice that says so.
    Missing(String),
}

/// The font for Japanese or Chinese: in a host that ships one (VS Code,
/// whose web view cannot read the system's fonts), the host's, loaded the
/// first time it is wanted; otherwise the system's.
fn font_for(language: Language) -> Found {
    if let Some(host) = crate::host::current() {
        match crate::host::font(&*host, language) {
            crate::host::HostFont::None => {}
            crate::host::HostFont::Waiting => return Found::Waiting,
            crate::host::HostFont::Loaded(bytes) => {
                return Found::Font(SystemFont { bytes, index: 0 });
            }
            crate::host::HostFont::Failed(why) => {
                return Found::Missing(tr!(
                    "The font for Japanese or Chinese could not be loaded ({why}), so their \
                     characters show as boxes",
                    why = why
                ));
            }
        }
    }
    match system_font(language) {
        Some(font) => Found::Font(font),
        None => Found::Missing(
            tr!(
                "No font for Japanese or Chinese was found on this system, so their \
                 characters show as boxes"
            )
            .to_owned(),
        ),
    }
}

/// The files to try for a language, first to last, with the face of a
/// collection: on Windows its own fonts (Yu Gothic and Meiryo for Japanese,
/// Microsoft YaHei and SimSun for Chinese), on macOS Hiragino and PingFang.
/// Elsewhere fontconfig is asked ([`fontconfig`]).
fn candidates(language: Language) -> Vec<(PathBuf, u32)> {
    let japanese = language == Language::Japanese;
    let mut out = Vec::new();
    if cfg!(windows) {
        let fonts = std::env::var_os("WINDIR")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        let names: &[&str] = if japanese {
            &["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"]
        } else {
            &["msyh.ttc", "msyh.ttf", "simsun.ttc"]
        };
        out.extend(names.iter().map(|n| (fonts.join(n), 0)));
    } else if cfg!(target_os = "macos") {
        let names: &[&str] = if japanese {
            &[
                "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
            ]
        } else {
            &[
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
            ]
        };
        out.extend(names.iter().map(|n| (PathBuf::from(n), 0)));
    } else {
        out.extend(fontconfig(language));
    }
    out
}

/// The font fontconfig chooses for the language's script, as file and face
/// (`fc-match -f '%{file}|%{index}' :lang=ja`), if it chooses one that has
/// the language.
fn fontconfig(language: Language) -> Option<(PathBuf, u32)> {
    let lang = if language == Language::Japanese {
        "ja"
    } else {
        "zh-cn"
    };
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}|%{index}|%{lang}", &format!(":lang={lang}")])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.splitn(3, '|');
    let file = PathBuf::from(parts.next()?);
    let index = parts.next()?.trim().parse().unwrap_or(0);
    // fontconfig always answers; only a font that has the language will do.
    let langs = parts.next().unwrap_or_default();
    langs.split('|').any(|l| l == lang).then_some((file, index))
}

/// The system's font for the language's script, if it has one.
pub fn system_font(language: Language) -> Option<SystemFont> {
    if cfg!(target_family = "wasm") {
        return None;
    }
    candidates(language).into_iter().find_map(|(path, index)| {
        std::fs::read(&path)
            .ok()
            .map(|bytes| SystemFont { bytes, index })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows has fonts for both scripts; elsewhere it depends on what is
    /// installed, so only Windows is checked.
    #[test]
    #[cfg(windows)]
    fn windows_has_a_font_for_japanese_and_for_chinese() {
        for language in [Language::Japanese, Language::Chinese] {
            let font = system_font(language).unwrap();
            assert!(font.bytes.len() > 1_000_000, "{language:?}: {font:?}");
        }
    }
}
