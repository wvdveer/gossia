use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;
use crate::sources::{HttpAuth, SourceDef, SourceKind};

/// A request as the test server received it.
#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub method: String,
    /// With the query.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// An HTTP server on this machine for the tests. Its paths:
/// `/json` answers `{"total": 42, "items": [1, 2]}`; `/missing` 404;
/// `/denied` 401; `/moved` a 302 to another host; `/big` 5000 bytes;
/// `/slow` waits 1.5 s; anything else 200 `ok`.
pub(crate) struct Server {
    pub url: String,
    pub received: Arc<Mutex<Vec<Received>>>,
}

impl Server {
    pub fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve(stream, &log));
            }
        });
        Server { url, received }
    }

    pub fn last(&self) -> Received {
        self.received
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a request")
    }

    pub fn count(&self) -> usize {
        self.received.lock().unwrap().len()
    }
}

fn serve(mut stream: std::net::TcpStream, log: &Mutex<Vec<Received>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    log.lock().unwrap().push(Received {
        method,
        target: target.clone(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    // Below the base URL of `api`.
    let path = target.split('?').next().unwrap_or("");
    let path = path.strip_prefix("/v1").unwrap_or(path);
    if path == "/bytes" {
        // Not UTF-8: only `raw()` gives these back as they are.
        let body = [0xFFu8, 0x00, 0x41];
        let head = format!(
            "HTTP/1.1 200 OK
Content-Length: {}
Connection: close

",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
        return;
    }
    let (status, extra, text) = match path {
        "/json" => (
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"total": 42, "items": [1, 2]}"#.to_owned(),
        ),
        "/xml" => (
            "200 OK",
            "Content-Type: application/xml
",
            r#"<order id="7"><line><sku>A1</sku><qty>2</qty></line><line><sku>B2</sku><qty>5</qty></line></order>"#.to_owned(),
        ),
        "/missing" => ("404 Not Found", "", "not here".to_owned()),
        "/denied" => ("401 Unauthorized", "", String::new()),
        "/moved" => (
            "302 Found",
            "Location: http://example.invalid/\r\n",
            String::new(),
        ),
        "/big" => ("200 OK", "", "x".repeat(5000)),
        "/slow" => {
            std::thread::sleep(Duration::from_millis(1500));
            ("200 OK", "", "late".to_owned())
        }
        _ => ("200 OK", "", "ok".to_owned()),
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(reply.as_bytes());
}

pub(crate) fn api(url: &str) -> SourceDef {
    SourceDef {
        url: Some(format!("{url}/v1")),
        ..SourceDef::new("Api", SourceKind::Http)
    }
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(5),
        max_bytes: 1 << 20,
    }
}

fn get(path: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: path.into(),
        ..HttpRequest::default()
    }
}

#[test]
fn a_path_stays_below_the_base_url() {
    let def = api("https://api.example.com");
    let mut request = get("/orders/12");
    request.params = vec![("q".into(), "a b&c".into()), ("n".into(), "1".into())];
    assert_eq!(
        url(&def, &request).unwrap(),
        "https://api.example.com/v1/orders/12?q=a%20b%26c&n=1"
    );
    assert_eq!(url(&def, &get("")).unwrap(), "https://api.example.com/v1");
    assert!(url(&def, &get("https://evil.example.com/")).is_err());
    assert!(url(&def, &get("//evil.example.com/")).is_err());
    assert!(url(&def, &get("../admin")).is_err());
    assert_eq!(escape("é/~"), "%C3%A9%2F~");
}

#[test]
fn the_authentication_goes_in_the_headers_or_the_query() {
    let mut def = api("https://api.example.com");
    def.headers = vec![("Accept".into(), "text/plain".into())];
    def.auth = HttpAuth::Basic;
    def.user = Some("Aladdin".into());
    def.password = Some("open sesame".into());
    let mut request = get("x");
    request.headers = vec![
        ("accept".into(), "application/json".into()),
        ("Authorization".into(), "forged".into()),
    ];
    let h = headers(&def, &request);
    assert_eq!(
        h,
        [
            ("accept".to_owned(), "application/json".to_owned()),
            (
                "Authorization".to_owned(),
                "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==".to_owned()
            )
        ]
    );
    def.auth = HttpAuth::KeyQuery;
    def.key_name = Some("api_key".into());
    def.token = Some("k3y".into());
    assert!(url(&def, &get("x")).unwrap().ends_with("/x?api_key=k3y"));
    assert!(
        !headers(&def, &get("x"))
            .iter()
            .any(|(k, _)| k == "Authorization")
    );
    def.auth = HttpAuth::Bearer;
    assert!(headers(&def, &get("x")).contains(&("Authorization".into(), "Bearer k3y".into())));
}

#[test]
fn a_request_is_sent_and_its_answer_read() {
    let server = Server::start();
    let mut def = api(&server.url);
    def.auth = HttpAuth::KeyHeader;
    def.key_name = Some("X-Key".into());
    def.token = Some("secret".into());
    let response = send(&def, &get("/json"), limits()).unwrap();
    assert_eq!(response.status, 200);
    assert!(String::from_utf8_lossy(&response.body).contains("42"));
    let received = server.last();
    assert_eq!(received.target, "/v1/json");
    assert_eq!(received.header("X-Key"), Some("secret"));

    // Statuses come back as they are; a redirect is not followed.
    assert_eq!(send(&def, &get("/missing"), limits()).unwrap().status, 404);
    assert_eq!(send(&def, &get("/moved"), limits()).unwrap().status, 302);

    // A body and a method.
    let request = HttpRequest {
        method: "post".into(),
        path: "/orders".into(),
        body: Some(b"{\"a\":1}".to_vec()),
        ..HttpRequest::default()
    };
    send(&def, &request, limits()).unwrap();
    let received = server.last();
    assert_eq!(
        (received.method.as_str(), received.body.as_str()),
        ("POST", "{\"a\":1}")
    );
}

#[test]
fn the_limits_are_kept() {
    let server = Server::start();
    let def = api(&server.url);
    let small = Limits {
        timeout: Duration::from_secs(5),
        max_bytes: 1000,
    };
    let why = send(&def, &get("/big"), small).unwrap_err();
    assert!(why.contains("maximum response size"), "{why}");
    let quick = Limits {
        timeout: Duration::from_millis(300),
        max_bytes: 1000,
    };
    let why = send(&def, &get("/slow"), quick).unwrap_err();
    assert!(why.contains("did not answer"), "{why}");
}

#[test]
fn the_test_gets_the_test_path_and_tells_a_refusal() {
    let server = Server::start();
    let mut def = api(&server.url);
    def.test_path = Some("/json".into());
    let outcome = test(&def, Duration::from_secs(5));
    assert!(outcome.ok, "{}", outcome.message);
    assert!(outcome.message.contains("200"));
    assert_eq!(server.last().target, "/v1/json");
    def.test_path = Some("/denied".into());
    let outcome = test(&def, Duration::from_secs(5));
    assert!(!outcome.ok);
    assert!(outcome.message.contains("refused"), "{}", outcome.message);
    def.test_path = Some("/missing".into());
    let outcome = test(&def, Duration::from_secs(5));
    assert!(
        outcome.ok && outcome.message.contains("404"),
        "{}",
        outcome.message
    );
}

/// The public test API the owner chose (2026-10-29). It needs the internet,
/// so it runs only when asked: `cargo test -p cliotree-calc -- --ignored`.
pub(crate) const RANDOM_USER: &str = "https://randomuser.me/api/";

pub(crate) fn random_user() -> SourceDef {
    SourceDef {
        url: Some(RANDOM_USER.into()),
        headers: vec![("Accept".into(), "application/json".into())],
        ..SourceDef::new("RandomUser", SourceKind::Http)
    }
}

#[test]
#[ignore = "uses the internet: randomuser.me"]
fn randomuser_me_answers_over_https() {
    let def = random_user();
    let limits = Limits {
        timeout: Duration::from_secs(30),
        max_bytes: 1 << 20,
    };
    let mut request = get("");
    request.params = vec![("results".into(), "3".into())];
    assert_eq!(
        url(&def, &request).unwrap(),
        "https://randomuser.me/api/?results=3"
    );
    let response = send(&def, &request, limits).unwrap();
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    let results = body["results"].as_array().expect("a list of users");
    assert_eq!(results.len(), 3);
    assert!(results[0]["name"]["first"].is_string(), "{body}");
    let outcome = test(&def, Duration::from_secs(30));
    assert!(outcome.ok, "{}", outcome.message);
}
