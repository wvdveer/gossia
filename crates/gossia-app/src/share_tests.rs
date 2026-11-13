//! Share files (DESIGN.md, The web version on GitHub Pages, 17e): written,
//! read back, and opened as a new workbook that is not saved yet and not
//! trusted.

use std::path::PathBuf;
use std::time::Duration;

use crate::GossiaApp;
use crate::share::{self, SITE};
use crate::workbook_file::new_workbook_id;

/// A folder of its own, removed when it goes out of scope.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let path = std::env::temp_dir().join(format!(
            "gossia-share-{tag}-{}-{}",
            std::process::id(),
            new_workbook_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn app() -> GossiaApp {
    let mut app = GossiaApp::sample();
    app.files.browse = false;
    settle(&mut app);
    app
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

// ------------------------------------------------------------- the file

#[test]
fn a_share_file_holds_the_workbook_unchanged_and_gives_it_back() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let name = r#"R&D <"plans">.gossia"#;
    let html = share::to_html(name, &bytes);
    assert!(share::is_share(html.as_bytes()));
    let (read_name, read_bytes) = share::read(html.as_bytes()).unwrap();
    assert_eq!(read_name, name);
    assert_eq!(read_bytes, bytes);
    // The name is escaped wherever it is shown.
    assert!(!html.contains(r#"<"plans">"#));
    assert!(html.contains("R&amp;D &lt;&quot;plans&quot;&gt;"));
}

#[test]
fn the_page_loads_nothing_from_elsewhere_and_names_the_site() {
    let html = share::to_html("Orders.gossia", b"PK");
    assert!(html.contains("<title>Orders</title>"));
    assert!(html.contains("Open in Gossia"));
    // The site, as the link for when the button does nothing, and where
    // the button goes.
    assert!(html.contains(&format!(r#"<a href="{SITE}">{SITE}</a>"#)));
    assert!(html.contains(&format!("const SITE = \"{SITE}\";")));
    // No script, style sheet or image from another address.
    for loaded in ["src=", "<link", "@import", "url("] {
        assert!(!html.contains(loaded), "{loaded}");
    }
    // Without scripts the button is not shown, and the way in is.
    assert!(html.contains(r#"<button id="open" type="button" hidden>"#));
    assert!(html.contains("drop this file on it"));
}

#[test]
fn the_page_is_in_the_interface_language() {
    gossia_i18n::set_thread_language(Some(gossia_i18n::Language::French));
    let html = share::to_html("Commandes.gossia", b"PK");
    gossia_i18n::set_thread_language(None);
    assert!(html.contains(r#"<html lang="fr">"#));
    assert!(html.contains("Ouvrir dans Gossia"));
}

#[test]
fn other_files_are_not_share_files_and_a_damaged_one_says_so() {
    assert!(!share::is_share(b"PK\x03\x04 a zip"));
    assert!(!share::is_share(b"<!doctype html><title>A page</title>"));
    assert!(share::read(b"<!doctype html><title>A page</title>").is_err());
    let html = share::to_html("Orders.gossia", b"PK");
    let damaged = html.replace(r#"id="workbook">"#, r#"id="workbook">!!!"#);
    let error = share::read(damaged.as_bytes()).unwrap_err();
    assert!(error.contains("damaged"), "{error}");
}

#[test]
fn a_name_from_a_share_file_is_a_workbook_name_without_folders() {
    let html = share::to_html("../../secret/Orders", b"PK");
    let (name, _) = share::read(html.as_bytes()).unwrap();
    assert_eq!(name, "Orders.gossia");
    assert_eq!(share::file_name("Orders.gossia"), "Orders.html");
}

// ------------------------------------------------- saving and opening

#[test]
fn a_share_file_is_a_copy_and_the_workbook_keeps_its_own_file() {
    let folder = Folder::new("copy");
    let mut app = app();
    app.save_as(&folder.file("orders.gossia")).unwrap();
    app.save_share_to(&folder.file("orders")).unwrap();
    // `.html` is added, the workbook keeps its file and is not changed.
    let html = std::fs::read(folder.file("orders.html")).unwrap();
    assert!(share::is_share(&html));
    assert_eq!(app.file_name(), "orders.gossia");
    assert!(!app.is_dirty());
    assert_eq!(app.notice, None, "no warning for a small file");
    // Save As to an `.html` name writes a share file too.
    app.files.dialog = Some(crate::workbook_file::FileDialog {
        purpose: crate::workbook_file::Browsing::SaveAs,
        text: folder.file("copy.html").display().to_string(),
        error: None,
    });
    app.apply_file_dialog();
    assert!(
        app.files.dialog.is_none(),
        "{:?}",
        app.files.dialog.as_ref().and_then(|d| d.error.clone())
    );
    assert!(share::is_share(
        &std::fs::read(folder.file("copy.html")).unwrap()
    ));
    assert_eq!(app.file_name(), "orders.gossia");
}

#[test]
fn a_share_file_opens_as_a_new_workbook_not_saved_and_not_trusted() {
    let folder = Folder::new("open");
    let mut app = app();
    app.save_as(&folder.file("orders.gossia")).unwrap();
    assert!(app.calc().is_trusted());
    app.save_share_to(&folder.file("orders.html")).unwrap();

    // The same user, who trusts the original: the share file is still not
    // trusted, since it could be anyone's.
    app.open_path(&folder.file("orders.html")).unwrap();
    settle(&mut app);
    assert_eq!(app.doc.path, None);
    assert_eq!(app.file_name(), "orders.gossia");
    assert!(app.is_dirty(), "it is in no file of the user's");
    assert!(!app.calc().is_trusted());
    assert!(app.banner_shown());
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.contains("came from a share file"), "{notice}");
    assert_eq!(
        app.calc().sheet_names().collect::<Vec<_>>(),
        ["Orders", "Prices", "Orders_schema", "Prices_schema"]
    );
    // It is not a recent file.
    assert!(
        !app.user
            .prefs
            .recent
            .iter()
            .any(|p| p.ends_with("orders.html"))
    );

    // Saving makes it the user's own, under the name offered.
    app.save_as(&folder.file("mine.gossia")).unwrap();
    assert!(!app.is_dirty());
    assert_eq!(app.file_name(), "mine.gossia");
    assert!(!app.doc.from_share);
}

#[test]
fn dropping_a_share_file_opens_it() {
    let folder = Folder::new("drop");
    let mut sender = app();
    sender.save_share_to(&folder.file("orders.html")).unwrap();
    let mut other = app();
    other.drop_files(vec![folder.file("orders.html")]);
    settle(&mut other);
    assert!(other.doc.from_share);
    assert!(other.imports.is_empty(), "not imported as a page");
    // An `.html` file that is no share file is imported as before.
    std::fs::write(folder.file("page.html"), "<p>hi</p>").unwrap();
    other.drop_files(vec![folder.file("page.html")]);
    assert!(other.doc.from_share, "the share stays open");
}
