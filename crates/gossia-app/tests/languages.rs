//! The interface in other languages (DESIGN.md, step 13f): chosen in Options,
//! shown at once, kept in the preferences, and given the system's font for
//! Japanese and Chinese.

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_i18n::Language;

/// A window in English, on this thread only: the language the Options
/// choose stays on this thread too, so other tests keep English.
fn window() -> Harness<'static, GossiaApp> {
    gossia_i18n::set_thread_language(Some(Language::English));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
    harness
}

fn has(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some()
}

fn choose(harness: &mut Harness<'_, GossiaApp>, language: Option<Language>) {
    if harness.state().options.is_none() {
        harness.state_mut().begin_options(None);
    }
    harness.state_mut().options.as_mut().unwrap().language = language;
    harness.run();
    harness.run();
}

#[test]
fn a_language_chosen_in_options_is_shown_at_once_and_kept() {
    let mut harness = window();
    assert!(has(&harness, "File"));
    choose(&mut harness, Some(Language::French));
    // Previewed while the dialog is open: the menus and the dialog itself.
    assert_eq!(gossia_i18n::language(), Language::French);
    assert!(has(&harness, "Fichier"), "the File menu in French");
    assert!(has(&harness, "Options de Gossia"), "the dialog's title");
    assert!(has(&harness, "Langue"));
    harness.state_mut().apply_options().unwrap();
    harness.run();
    assert_eq!(
        harness.state().user.prefs.appearance.language,
        Some(Language::French)
    );
    assert!(has(&harness, "Fichier"));
    // Cancel takes a previewed language back.
    choose(&mut harness, Some(Language::Spanish));
    assert!(has(&harness, "Archivo"));
    harness.state_mut().cancel_options();
    harness.run();
    assert!(has(&harness, "Fichier"));
    gossia_i18n::set_thread_language(None);
}

#[test]
fn japanese_and_chinese_have_the_system_font() {
    let mut harness = window();
    assert_eq!(harness.state().cjk_font_for, Some(None));
    choose(&mut harness, Some(Language::Japanese));
    assert!(has(&harness, "ファイル"));
    assert_eq!(harness.state().cjk_font_for, Some(Some(Language::Japanese)));
    if cfg!(windows) {
        let fonts = harness.ctx.fonts(|f| f.definitions().clone());
        assert!(fonts.font_data.contains_key("CJK"), "Yu Gothic is used");
        let family = &fonts.families[&egui::FontFamily::Proportional];
        let inter = family.iter().position(|f| f == "Inter").unwrap();
        let cjk = family.iter().position(|f| f == "CJK").unwrap();
        assert!(inter < cjk, "Latin text keeps Inter: {family:?}");
    }
    choose(&mut harness, Some(Language::Chinese));
    assert!(has(&harness, "文件"));
    assert_eq!(harness.state().cjk_font_for, Some(Some(Language::Chinese)));
    // Back to English: the font goes.
    choose(&mut harness, Some(Language::English));
    assert!(has(&harness, "File"));
    assert_eq!(harness.state().cjk_font_for, Some(None));
    gossia_i18n::set_thread_language(None);
}
