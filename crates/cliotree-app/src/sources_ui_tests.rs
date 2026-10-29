use super::*;

fn postgres() -> SourceDef {
    SourceDef {
        host: Some("db.example.com".into()),
        database: Some("sales".into()),
        user: Some("ro".into()),
        password: Some("hunter2".into()),
        ..SourceDef::new("Sales", SourceKind::Postgres)
    }
}

#[test]
fn editing_keeps_a_stored_password_unless_it_is_changed_or_cleared() {
    let mut form = SourceForm::edit(postgres());
    // The form never holds the secret where it could be shown.
    assert_eq!(form.draft.password, None);
    assert_eq!(form.password, Secret::Keep);
    assert_eq!(
        form.definition().unwrap().password.as_deref(),
        Some("hunter2")
    );
    form.password = Secret::Change("new".into());
    assert_eq!(form.definition().unwrap().password.as_deref(), Some("new"));
    form.password = Secret::Clear;
    assert_eq!(form.definition().unwrap().password, None);
}

#[test]
fn a_copy_has_the_settings_but_not_the_credentials() {
    let form = SourceForm::copy(&postgres(), "Sales_copy".into());
    assert_eq!(form.old, None);
    let def = form.definition().unwrap();
    assert_eq!(def.name, "Sales_copy");
    assert_eq!(def.host.as_deref(), Some("db.example.com"));
    assert_eq!(def.password, None);
}

#[test]
fn the_form_keeps_only_what_the_kind_uses_and_reads_its_numbers() {
    let mut form = SourceForm::edit(postgres());
    form.port = "70000".into();
    assert!(form.definition().unwrap_err().contains("port"));
    form.port = "6543".into();
    form.pooling = true;
    form.pool = "0".into();
    assert!(form.definition().unwrap_err().contains("pooling"));
    form.pool = "3".into();
    let def = form.definition().unwrap();
    assert_eq!((def.port, def.pool), (Some(6543), Some(3)));

    // Switching to HTTP drops the database settings and the password.
    form.draft.kind = SourceKind::Http;
    form.draft.url = Some("https://api.example.com".into());
    form.headers = "Accept: application/json\n\n X-Team : blue ".into();
    let def = form.definition().unwrap();
    assert_eq!(def.host, None);
    assert_eq!(def.password, None);
    assert_eq!(def.port, None);
    assert_eq!(
        def.headers,
        [
            ("Accept".to_owned(), "application/json".to_owned()),
            ("X-Team".to_owned(), "blue".to_owned())
        ]
    );
    form.headers = "no colon".into();
    assert!(form.definition().unwrap_err().contains("colon"));
}

#[test]
fn saving_adds_renames_and_refuses_a_taken_name() {
    let mut app = ClioApp::sample();
    let mut form = SourceForm::new();
    form.draft.name = "Api".into();
    form.draft.url = Some("https://api.example.com".into());
    assert_eq!(app.save_source_form(&form), Ok("Api".to_owned()));
    // The same name again, as a new one, is refused.
    assert!(app.save_source_form(&form).is_err());
    let mut edit = SourceForm::edit(app.user.sources.get("Api").unwrap().unwrap());
    edit.draft.name = "Web".into();
    app.save_source_form(&edit).unwrap();
    let names: Vec<String> = app
        .user
        .sources
        .list()
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, ["Web"]);
    assert_eq!(app.copy_name("Web"), "Web_copy");
}
