use std::path::PathBuf;

use super::*;

/// The books of a small shop, in a fresh folder: an opening balance, a sale
/// invoiced and paid, a purchase at a cost, a padded balance, and an
/// included file of Expenses.
pub(crate) const BOOKS: &str = r#"option "operating_currency" "AUD"

2025-01-01 open Assets:Bank AUD
2025-01-01 open Assets:Debtors:Bloggs AUD
2025-01-01 open Assets:Shares ACME
2025-01-01 open Revenue:Sales AUD
2025-01-01 open Expenses:Rent AUD
2025-01-01 open Equity:Opening AUD

2025-01-02 pad Assets:Bank Equity:Opening
2025-01-03 balance Assets:Bank 5000.00 AUD

2025-10-17 * "Bloggs" "Invoice #10765" #sale ^inv-10765
  invoice: "10765"
  Assets:Debtors:Bloggs   1000.00 AUD
  Revenue:Sales

2025-10-20 * "Bloggs" "Payment of invoice #10765" ^inv-10765
  Assets:Bank             1000.00 AUD
  Assets:Debtors:Bloggs  -1000.00 AUD

2025-10-21 * "Buy shares"
  Assets:Shares   10 ACME {50.00 AUD}
  Assets:Bank

2025-10-22 price ACME 52.10 AUD

include "Expenses/*.beancount"
"#;

pub(crate) const RENT: &str = r#"2025-10-01 * "Rent for October"
  Expenses:Rent   1500.00 AUD
  Assets:Bank
"#;

pub(crate) fn books(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("cliotree-journal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Expenses")).unwrap();
    let main = dir.join("books.beancount");
    std::fs::write(&main, BOOKS).unwrap();
    std::fs::write(dir.join("Expenses").join("rent.beancount"), RENT).unwrap();
    (dir, main)
}

pub(crate) fn def(main: &Path, writes: bool) -> SourceDef {
    SourceDef {
        path: Some(main.display().to_string()),
        writes,
        ..SourceDef::new("Books", SourceKind::Beancount)
    }
}

fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn decimal_of(json: &Json) -> Decimal {
    d(json["$dec"].as_str().unwrap())
}

#[test]
fn a_journal_is_read_with_its_included_files_and_its_padding() {
    let (_dir, main) = books("read");
    let j = journal(&def(&main, false)).unwrap();
    assert_eq!(j.files, 2);
    assert!(j.errors.is_empty(), "{:?}", j.errors);
    // Padding first, then the rent from the included file, by date.
    let narrations: Vec<&str> = j
        .transactions
        .iter()
        .map(|t| t.narration.as_str())
        .collect();
    assert!(narrations[0].starts_with("(Padding inserted"));
    assert_eq!(narrations[1], "Rent for October");
    assert_eq!(j.transactions[1].file, "Expenses/rent.beancount");
    assert_eq!(
        j.options,
        vec![("operating_currency".to_owned(), "AUD".to_owned())]
    );
    assert_eq!(j.accounts.len(), 6);
}

#[test]
fn the_posting_without_an_amount_is_filled_in_and_costs_are_weighed() {
    let (_dir, main) = books("fill");
    let j = journal(&def(&main, false)).unwrap();
    let invoice = j
        .transactions
        .iter()
        .find(|t| t.narration == "Invoice #10765")
        .unwrap();
    let sales = &invoice.postings[1];
    assert!(sales.auto);
    assert_eq!(sales.amount, Some(d("-1000.00")));
    let shares = j
        .transactions
        .iter()
        .find(|t| t.narration == "Buy shares")
        .unwrap();
    assert_eq!(shares.postings[0].cost, Some(d("50.00")));
    assert_eq!(shares.postings[1].amount, Some(d("-500.00")));
    assert_eq!(shares.postings[1].currency.as_deref(), Some("AUD"));
}

#[test]
fn balances_are_exact_and_include_sub_accounts() {
    let (_dir, main) = books("balance");
    let def = def(&main, false);
    let all = read_call(&def, "balance", &json!({"account": "Assets"})).unwrap();
    // 5000 opening + 1000 paid - 500 shares - 1500 rent = 4000 in the bank,
    // the debtor is paid off, and 10 ACME.
    assert_eq!(decimal_of(&all["AUD"]), d("4000.00"));
    assert_eq!(decimal_of(&all["ACME"]), d("10"));
    let before = read_call(
        &def,
        "balance",
        &json!({"account": "Assets:Debtors", "until": "2025-10-19"}),
    )
    .unwrap();
    assert_eq!(decimal_of(&before["AUD"]), d("1000.00"));
    assert_eq!(all["AUD"]["$dec"], "4000.00", "the scale is kept");
}

#[test]
fn transactions_are_filtered_by_account_dates_payee_tag_and_link() {
    let (_dir, main) = books("filter");
    let def = def(&main, false);
    let count = |args: Json| {
        read_call(&def, "transactions", &args)
            .unwrap()
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(count(json!({})), 5);
    assert_eq!(count(json!({"account": "Assets:Debtors"})), 2);
    assert_eq!(
        count(json!({"account": "Assets:Debt"})),
        0,
        "a whole name only"
    );
    assert_eq!(
        count(json!({"since": "2025-10-17", "until": "2025-10-20"})),
        2
    );
    assert_eq!(count(json!({"payee": "bloggs"})), 2);
    assert_eq!(count(json!({"tag": "sale"})), 1);
    assert_eq!(count(json!({"link": "inv-10765"})), 2);
    let found = read_call(&def, "transactions", &json!({"tag": "sale"})).unwrap();
    let t = &found[0];
    assert_eq!(t["date"]["$date"], "2025-10-17");
    assert_eq!(t["meta"]["invoice"], "10765");
    assert_eq!(t["postings"][1]["auto"], true);
    assert_eq!(t["generated"], Json::Null);
}

#[test]
fn what_is_wrong_with_the_books_is_listed_with_its_file_and_line() {
    let (dir, main) = books("errors");
    std::fs::write(
        &main,
        format!(
            "{BOOKS}\nplugin \"beancount.plugins.auto_accounts\"\n\
             2025-11-01 * \"Off by a cent\"\n  Expenses:Rent  10.00 AUD\n  Assets:Bank  -9.99 AUD\n\
             2025-11-02 * \"Unknown account\"\n  Expenses:Food  1 AUD\n  Assets:Bank\n\
             2025-11-03 balance Assets:Bank 1.00 AUD\n"
        ),
    )
    .unwrap();
    let j = journal(&def(&main, false)).unwrap();
    let all = j.errors.join("\n");
    assert!(
        all.contains("plugin \"beancount.plugins.auto_accounts\" is not run"),
        "{all}"
    );
    assert!(all.contains("do not balance: 0.01 AUD left over"), "{all}");
    assert!(
        all.contains("Expenses:Food is used but never opened"),
        "{all}"
    );
    assert!(all.contains("the balance of Assets:Bank is"), "{all}");
    assert!(
        j.errors.iter().all(|e| e.starts_with("books.beancount:")),
        "{all}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_syntax_error_or_an_include_outside_the_folder_fails_the_whole_journal() {
    let (dir, main) = books("syntax");
    std::fs::write(&main, "2025-01-01 open Assets:Bank AUD\n2025-01-02 * \"x\n").unwrap();
    let why = journal(&def(&main, false)).unwrap_err();
    assert!(
        why.contains("books.beancount:2: this is not Beancount syntax"),
        "{why}"
    );
    std::fs::write(&main, "include \"../elsewhere.beancount\"\n").unwrap();
    std::fs::write(dir.parent().unwrap().join("elsewhere.beancount"), "").unwrap();
    let why = journal(&def(&main, false)).unwrap_err();
    assert!(why.contains("must be in the journal's folder"), "{why}");
}

#[test]
fn the_journal_is_read_again_when_a_file_changes() {
    let (dir, main) = books("fresh");
    let def = def(&main, false);
    let first = journal(&def).unwrap();
    assert!(
        Arc::ptr_eq(&first, &journal(&def).unwrap()),
        "kept while nothing changed"
    );
    assert!(!changed(&def));
    std::fs::write(
        dir.join("Expenses").join("power.beancount"),
        "2025-10-05 * \"Power\"\n  Expenses:Rent  100.00 AUD\n  Assets:Bank\n",
    )
    .unwrap();
    // A new file in a folder that an include lists.
    assert!(changed(&def));
    assert_eq!(
        journal(&def).unwrap().transactions.len(),
        first.transactions.len() + 1
    );
    assert!(!changed(&def));
}

#[test]
fn add_appends_a_marked_transaction_and_changes_nothing_else() {
    let (_dir, main) = books("add");
    let def = def(&main, true);
    let added = append(
        &def,
        &json!({
            "date": "2025-10-17", "narration": "Invoice #10766", "payee": "Bloggs",
            "tags": ["sale"], "links": ["inv-10766"], "meta": {"invoice": "10766"},
            "postings": [
                {"account": "Assets:Debtors:Bloggs", "amount": {"$dec": "250.00"}, "currency": "AUD"},
                {"account": "Revenue:Sales", "amount": null, "currency": null},
            ],
        }),
    )
    .unwrap();
    let text = std::fs::read_to_string(&main).unwrap();
    assert!(text.starts_with(BOOKS), "the old text is untouched");
    let expected = "2025-10-17 * \"Bloggs\" \"Invoice #10766 [ClioTree generated original transaction]\" #sale ^inv-10766\n  invoice: \"10766\"\n  Assets:Debtors:Bloggs  250.00 AUD\n  Revenue:Sales\n";
    assert_eq!(added["text"], expected);
    assert!(text.ends_with(&format!("\n\n{expected}")));
    let line = added["line"].as_u64().unwrap() as usize;
    assert!(
        text.lines()
            .nth(line - 1)
            .unwrap()
            .starts_with("2025-10-17 * \"Bloggs\"")
    );
    // It reads back, marked.
    let j = journal(&def).unwrap();
    assert!(j.errors.is_empty(), "{:?}", j.errors);
    let back = read_call(&def, "transactions", &json!({"link": "inv-10766"})).unwrap();
    assert_eq!(back[0]["generated"], "original");
}

#[test]
fn adjustments_and_reversals_are_marked_as_such() {
    let (_dir, main) = books("adjust");
    let def = def(&main, true);
    let added = append(
        &def,
        &json!({
            "date": "2025-10-17", "kind": "adjustment",
            "narration": "10% Discount granted on Invoice #10765",
            "postings": [
                {"account": "Assets:Debtors:Bloggs", "amount": {"$dec": "-100.00"}, "currency": "AUD"},
                {"account": "Revenue:Sales", "amount": {"$dec": "100.00"}, "currency": "AUD"},
            ],
        }),
    )
    .unwrap();
    assert!(added["text"].as_str().unwrap().starts_with(
        "2025-10-17 * \"10% Discount granted on Invoice #10765 [ClioTree generated adjustment transaction]\""
    ));
    assert_eq!(
        generated("Undo it [ClioTree generated reversal transaction]"),
        Some("reversal")
    );
    assert_eq!(generated("Typed by hand"), None);
    assert_eq!(generated("[ClioTree generated nonsense transaction]"), None);
    let wrong = append(
        &def,
        &json!({"date": "2025-10-17", "kind": "edit", "postings": []}),
    );
    assert!(
        wrong
            .unwrap_err()
            .contains("original, adjustment or reversal")
    );
    let forged = append(
        &def,
        &json!({"date": "2025-10-17", "narration": "x [ClioTree generated original transaction]",
                "postings": [{"account": "Assets:Bank", "amount": "1", "currency": "AUD"},
                             {"account": "Revenue:Sales", "amount": "-1", "currency": "AUD"}]}),
    );
    assert!(forged.unwrap_err().contains("ClioTree adds that itself"));
}

#[test]
fn add_refuses_what_does_not_balance_or_uses_an_account_that_is_not_open() {
    let (_dir, main) = books("refuse");
    let def = def(&main, true);
    let before = std::fs::read_to_string(&main).unwrap();
    let posting = |account: &str, amount: &str| json!({"account": account, "amount": amount, "currency": "AUD"});
    let tries = [
        (
            json!({"date": "2025-10-17", "postings": [posting("Assets:Bank", "10"), posting("Revenue:Sales", "-9")]}),
            "do not balance",
        ),
        (
            json!({"date": "2025-10-17", "postings": [posting("Expenses:Food", "10"), posting("Assets:Bank", "-10")]}),
            "Expenses:Food is used but never opened",
        ),
        (
            json!({"date": "2024-12-31", "postings": [posting("Expenses:Rent", "10"), posting("Assets:Bank", "-10")]}),
            "before it is opened",
        ),
        (
            json!({"date": "2025-02-30", "postings": [posting("Expenses:Rent", "10"), posting("Assets:Bank", "-10")]}),
            "is not a date",
        ),
        (
            json!({"date": "2025-10-17", "postings": [{"account": "Assets:Shares", "amount": "1", "currency": "AUD"}, posting("Assets:Bank", "-1")]}),
            "holds only ACME",
        ),
        (
            json!({"date": "2025-10-17", "narration": "two\nlines", "postings": [posting("Expenses:Rent", "1"), posting("Assets:Bank", "-1")]}),
            "line break",
        ),
    ];
    for (request, why) in tries {
        let got = append(&def, &request).unwrap_err();
        assert!(got.contains(why), "{got} should say {why}");
    }
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        before,
        "nothing was written"
    );
}

#[test]
fn the_test_says_what_is_in_the_journal() {
    let (_dir, main) = books("test");
    let outcome = test(&def(&main, false));
    assert!(outcome.ok);
    assert!(
        outcome
            .message
            .starts_with("4 transactions in 6 accounts, from 2025-01-02 to 2025-10-21, 2 files"),
        "{}",
        outcome.message
    );
    let missing = test(&def(&main.with_file_name("nope.beancount"), false));
    assert!(!missing.ok);
}

#[test]
fn wildcards_match_names() {
    assert!(wildcard("*.beancount", "rent.beancount"));
    assert!(wildcard("202?-*.bean*", "2025-10.beancount"));
    assert!(!wildcard("*.beancount", "rent.txt"));
}

#[test]
fn a_line_the_parser_would_pass_over_fails_the_journal() {
    let (_dir, main) = books("unread");
    std::fs::write(
        &main,
        "; a comment\n* A heading\n2025-01-01 open Assets:Bank AUD\n\
         2025-01-02 note Assets:Bank \"called them\"\n\
         Rent paid to the landlord\n",
    )
    .unwrap();
    let why = journal(&def(&main, false)).unwrap_err();
    assert!(
        why.contains("books.beancount:5: this line is not Beancount"),
        "{why}"
    );
}
