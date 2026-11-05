use std::time::Duration;

use super::Fetches;
use crate::http::{FetchRequest, HttpResponse};

fn request(method: &str, url: &str) -> FetchRequest {
    FetchRequest {
        id: 0,
        method: method.to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: None,
        timeout: Duration::from_secs(5),
        max_bytes: 100,
    }
}

fn ok(body: &str) -> Result<HttpResponse, String> {
    Ok(HttpResponse {
        status: 200,
        reason: "OK".to_owned(),
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    })
}

#[test]
fn a_read_is_asked_once_and_answered_for_the_rounds_of_a_pass() {
    let mut fetches = Fetches::default();
    assert!(
        fetches
            .answer(request("GET", "http://x/a"), false)
            .is_none()
    );
    assert!(
        fetches
            .answer(request("GET", "http://x/a"), false)
            .is_none()
    );
    let asked = fetches.take_new();
    assert_eq!(asked.len(), 1, "asked once");
    assert!(fetches.waiting());
    fetches.answered(asked[0].id, ok("one"));
    assert!(!fetches.waiting());
    let answer = fetches.answer(request("GET", "http://x/a"), false);
    assert_eq!(answer.unwrap().unwrap().body, b"one");
    // A second call in the same round: asked, and the first stays answered.
    assert!(
        fetches
            .answer(request("GET", "http://x/b"), false)
            .is_none()
    );
    fetches.end_pass();
    assert!(
        fetches
            .answer(request("GET", "http://x/a"), false)
            .is_some()
    );
    let b = fetches.take_new();
    fetches.answered(b[0].id, ok("two"));
    assert!(
        fetches
            .answer(request("GET", "http://x/b"), false)
            .is_some()
    );
    // A pass that asks for nothing ends the reading: the next reads afresh.
    fetches.end_pass();
    assert!(
        fetches
            .answer(request("GET", "http://x/a"), false)
            .is_none()
    );
}

#[test]
fn an_answer_larger_than_allowed_is_refused() {
    let mut fetches = Fetches::default();
    fetches.answer(request("GET", "http://x/big?key=secret"), false);
    let asked = fetches.take_new();
    fetches.answered(asked[0].id, ok(&"x".repeat(101)));
    let answer = fetches.answer(request("GET", "http://x/big?key=secret"), false);
    let why = answer.unwrap().unwrap_err();
    assert!(why.contains("larger than the maximum"), "{why}");
    assert!(!why.contains("secret"), "the query is not shown: {why}");
}

#[test]
fn a_procedure_gets_its_logged_answers_in_order_and_a_write_is_sent_once() {
    let mut fetches = Fetches::default();
    fetches.begin_run();
    assert!(
        fetches
            .answer(request("POST", "http://x/orders"), true)
            .is_none()
    );
    let post = fetches.take_new();
    assert_eq!(post.len(), 1);
    fetches.answered(post[0].id, ok("made"));
    // Run again: the POST is answered from the log, the GET is asked.
    fetches.begin_run();
    let made = fetches.answer(request("POST", "http://x/orders"), true);
    assert_eq!(made.unwrap().unwrap().body, b"made");
    assert!(
        fetches
            .answer(request("GET", "http://x/orders"), true)
            .is_none()
    );
    let get = fetches.take_new();
    assert_eq!(get.len(), 1);
    assert_eq!(get[0].method, "GET", "the POST is not sent again");
    fetches.answered(get[0].id, ok("list"));
    fetches.begin_run();
    assert!(
        fetches
            .answer(request("POST", "http://x/orders"), true)
            .is_some()
    );
    assert!(
        fetches
            .answer(request("GET", "http://x/orders"), true)
            .is_some()
    );
    assert!(fetches.take_new().is_empty());
    fetches.end_run();
    // The next run starts afresh.
    fetches.begin_run();
    assert!(
        fetches
            .answer(request("POST", "http://x/orders"), true)
            .is_none()
    );
}

#[test]
fn a_run_that_takes_another_path_fails_and_sends_nothing_more() {
    let mut fetches = Fetches::default();
    fetches.begin_run();
    fetches.answer(request("POST", "http://x/a"), true);
    let asked = fetches.take_new();
    fetches.answered(asked[0].id, ok("done"));
    fetches.begin_run();
    let other = fetches.answer(request("DELETE", "http://x/b?token=t"), true);
    let why = other.unwrap().unwrap_err();
    assert!(
        why.contains("DELETE http://x/b where it called something else"),
        "{why}"
    );
    assert!(fetches.take_new().is_empty());
}
