//! Scenarios written once against the commands of the test control protocol,
//! run in process (`ui.rs`) and on the running application (`e2e.rs`).

use serde_json::{Value, json};

/// Sends one command and returns its result or its error.
pub type Send<'a> = &'a mut dyn FnMut(&str, Value) -> Result<Value, String>;

fn ok(send: Send<'_>, cmd: &str, args: Value) -> Value {
    send(cmd, args).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
}

fn value_of(send: Send<'_>, path: &str) -> Value {
    ok(send, "cell", json!({ "path": path }))["value"].clone()
}

/// An order, the file dropped on the blank workbook the scenario starts with.
const ORDERS: &str = r#"{
  "customer": {"name": "Acme Ltd", "city": "Delft"},
  "items": [
    {"sku": "A1", "qty": 2, "price": 9.5},
    {"sku": "B2", "qty": 1, "price": 24.5},
    {"sku": "C3", "qty": 12, "price": 0.75}
  ],
  "tax_rate": 0.21,
  "subtotal": 0.5
}"#;

/// Writes the order to a file of its own, for dropping on the window.
fn orders_file() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("cliotree-scenario-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Orders.json");
    std::fs::write(&path, ORDERS).unwrap();
    path
}

/// The application starts with a blank workbook; a JSON file dropped on it
/// is imported, a formula is entered, and then: edit, recalculate, undo,
/// and the refusals along the way.
pub fn edit_and_recalculate(send: Send<'_>) {
    assert_eq!(ok(send, "hello", Value::Null)["protocol"], 1);
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    let status = ok(send, "status", Value::Null);
    assert_eq!(status["text"], "Ready");
    assert_eq!(status["problems"], "No errors");
    assert_eq!(status["tab"], "Data");

    // Dropping the file imports it as a tab of its own, with a schema
    // inferred from it.
    let file = orders_file();
    let dropped = ok(send, "drop", json!({ "paths": [file] }));
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
    assert_eq!(dropped["tabs"], json!(["Orders"]));
    ok(send, "tab", json!({ "name": "Orders" }));
    assert_eq!(ok(send, "status", Value::Null)["tab"], "Orders");
    assert_eq!(value_of(send, "subtotal"), json!(0.5));

    // 2 * 9.5 + 1 * 24.5 + 12 * 0.75 = 52.5.
    ok(send, "select", json!({ "path": "subtotal" }));
    ok(
        send,
        "enter",
        json!({ "text": "=sum(i.qty * i.price for i in Orders.items)" }),
    );
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    assert_eq!(value_of(send, "subtotal"), json!(52.5));
    let subtotal = ok(send, "cell", json!({ "path": "subtotal" }));
    assert!(subtotal["formula"].as_str().unwrap().starts_with('='));
    assert_eq!(subtotal["pending"], false);

    // Entering a literal recalculates what depends on it.
    ok(send, "select", json!({ "path": "items[0].qty" }));
    ok(send, "enter", json!({ "text": "3" }));
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    assert_eq!(value_of(send, "items[0].qty"), json!(3));
    assert_eq!(value_of(send, "subtotal"), json!(62.0));

    // The key that undoes it.
    ok(send, "key", json!({ "chord": "ctrl+z" }));
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    assert_eq!(value_of(send, "subtotal"), json!(52.5));
    ok(send, "key", json!({ "chord": "ctrl+y" }));
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    assert_eq!(value_of(send, "subtotal"), json!(62.0));

    // A literal that does not fit the schema is refused with the reason.
    ok(send, "select", json!({ "path": "items[0].qty" }));
    let refused = send("enter", json!({ "text": "many" })).unwrap_err();
    assert!(!refused.is_empty());
    ok(send, "key", json!({ "chord": "escape" }));
    assert_eq!(value_of(send, "items[0].qty"), json!(3));

    // A formula, and a subtree.
    ok(send, "select", json!({ "path": "tax_rate" }));
    ok(send, "enter", json!({ "text": "=0.5 / 2" }));
    ok(send, "wait_idle", json!({ "timeout_ms": 60000 }));
    assert_eq!(value_of(send, "tax_rate"), json!(0.25));
    let tree = ok(send, "tree", json!({ "path": "customer", "depth": 1 }));
    assert!(tree.is_object());

    // Commands that still wait for the workbook layer say so; unknown ones are
    // refused.
    let close = send("close", Value::Null).unwrap_err();
    assert!(close.contains("not supported yet"), "{close}");
    let unknown = send("explode", Value::Null).unwrap_err();
    assert!(unknown.contains("unknown command"), "{unknown}");

    // File > New replaces the workbook with an empty data tab and its schema.
    let made = ok(send, "new_workbook", Value::Null);
    assert_eq!(made["tabs"], json!(["Data", "DataSchema"]));
    let status = ok(send, "status", Value::Null);
    assert_eq!(status["tab"], "Data");
    assert_eq!(status["modified"], false);
}
