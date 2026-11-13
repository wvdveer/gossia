//! Hidden tabs (DESIGN.md, "Managing tabs", *Hide tab*): hidden from a
//! tab's menu, off the tab bar, and shown again by name from *View >
//! Unhide tab*.

use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use gossia_app::GossiaApp;

fn window() -> Harness<'static, GossiaApp> {
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

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn on_the_bar(harness: &Harness<'_, GossiaApp>, tab: &str) -> bool {
    harness.query(by().role(Role::Tab).label(tab)).is_some()
}

#[test]
fn a_hidden_tab_leaves_the_bar_and_comes_back_from_the_view_menu() {
    let mut harness = window();
    assert!(on_the_bar(&harness, "Prices"));
    harness
        .get(by().role(Role::Tab).label("Prices"))
        .click_secondary();
    harness.run();
    click(&mut harness, "Hide tab");
    assert!(!on_the_bar(&harness, "Prices"));
    assert!(on_the_bar(&harness, "Orders"));
    assert_eq!(harness.state().sheet, "Orders", "the tab shown stays");

    click(&mut harness, "View");
    click(&mut harness, "Unhide tab");
    click(&mut harness, "Prices");
    assert!(on_the_bar(&harness, "Prices"));
    assert_eq!(harness.state().sheet, "Prices", "and it is shown");
    assert!(harness.state().hidden_tabs.is_empty());
}

#[test]
fn hiding_the_tab_shown_shows_the_next_and_the_last_one_stays() {
    let mut harness = window();
    let tabs = harness.state().visible_tabs();
    assert!(tabs.len() > 1);
    harness.state_mut().switch_to(&tabs[0]);
    harness.run();
    // From the Edit menu, which acts on the tab shown.
    click(&mut harness, "Edit");
    click(&mut harness, "Hide tab");
    assert_eq!(harness.state().sheet, tabs[1]);
    assert!(!on_the_bar(&harness, &tabs[0]));

    for tab in &tabs[1..tabs.len() - 1] {
        harness.state_mut().hide_tab(tab);
    }
    harness.run();
    let last = tabs.last().unwrap();
    assert_eq!(harness.state().visible_tabs(), std::slice::from_ref(last));
    assert_eq!(&harness.state().sheet, last);
    assert!(!harness.state().can_hide_tab(last));
    harness.state_mut().hide_tab(last);
    assert_eq!(harness.state().visible_tabs(), std::slice::from_ref(last));
}

#[test]
fn the_unhide_menu_is_greyed_with_no_hidden_tabs() {
    let mut harness = window();
    click(&mut harness, "View");
    assert!(
        harness
            .get_by_label("Unhide tab")
            .accesskit_node()
            .is_disabled()
    );
    harness.state_mut().hide_tab("Prices");
    harness.run();
    assert!(
        !harness
            .get_by_label("Unhide tab")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn going_to_a_hidden_tab_puts_it_back_on_the_bar() {
    let mut harness = window();
    harness.state_mut().hide_tab("Prices");
    harness.run();
    assert!(!on_the_bar(&harness, "Prices"));
    harness.state_mut().switch_to("Prices");
    harness.run();
    assert!(on_the_bar(&harness, "Prices"));
}
