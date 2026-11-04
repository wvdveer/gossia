//! End-to-end tests on the running application (DESIGN.md, "Testing and CI",
//! level 8). The real binary is started, with the test control channel, and
//! driven over its loopback port with JSON lines. They need a display (a
//! virtual one, `xvfb-run`, on Linux CI) and are built only with the
//! `test-control` feature: `cargo xtask e2e`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod common;

const TOKEN: &str = "e2e-token";

struct Running {
    child: Child,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
    dir: PathBuf,
}

impl Running {
    fn start(name: &str) -> Running {
        let dir = std::env::temp_dir().join(format!("cliotree-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let port_file = dir.join("port");
        let child = Command::new(env!("CARGO_BIN_EXE_cliotree"))
            .env("CLIOTREE_TEST_TOKEN", TOKEN)
            .env("CLIOTREE_TEST_PORT_FILE", &port_file)
            // Preferences and trust decisions of its own, not the user's.
            .env("CLIOTREE_CONFIG_DIR", dir.join("config"))
            .spawn()
            .expect("the application starts");
        let mut running = Running::connect(child, &port_file, dir);
        running.wait_until_started();
        running
    }

    fn connect(mut child: Child, port_file: &std::path::Path, dir: PathBuf) -> Running {
        let deadline = Instant::now() + Duration::from_secs(180);
        let port = loop {
            if let Ok(text) = std::fs::read_to_string(port_file) {
                break text.trim().parse::<u16>().expect("a port number");
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("the application ended before it listened: {status}");
            }
            assert!(Instant::now() < deadline, "the application did not listen");
            std::thread::sleep(Duration::from_millis(100));
        };
        let writer = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        writer
            .set_read_timeout(Some(Duration::from_secs(200)))
            .unwrap();
        let reader = BufReader::new(writer.try_clone().unwrap());
        Running {
            child,
            reader,
            writer,
            next_id: 1,
            dir,
        }
    }

    /// The first window shows a blank workbook after its first evaluation.
    fn wait_until_started(&mut self) {
        self.send("wait_idle", json!({ "timeout_ms": 120000 }))
            .expect("the workbook settles");
    }

    fn send(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "id": id, "token": TOKEN, "cmd": cmd, "args": args });
        writeln!(self.writer, "{request}").map_err(|e| e.to_string())?;
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let response: Value = serde_json::from_str(&line).map_err(|e| format!("{e}: {line}"))?;
        assert_eq!(response["id"], id, "{response}");
        if response["ok"] == true {
            Ok(response["result"].clone())
        } else {
            Err(response["error"].as_str().unwrap_or_default().to_owned())
        }
    }

    /// Asks it to quit and waits for it to end, with an exit status of 0.
    fn quit(mut self) {
        self.send("quit", Value::Null).expect("quit is accepted");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "the application ended with {status}");
                break;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("the application did not end after quit");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // A failed test must not leave a window behind.
        let _ = self.child.kill();
    }
}

#[test]
fn the_application_starts_edits_and_recalculates_and_quits() {
    let mut app = Running::start("edit");
    common::edit_and_recalculate(&mut |cmd, args| app.send(cmd, args));
    app.quit();
}

#[test]
fn a_screenshot_is_saved_and_a_wrong_token_is_refused() {
    let mut app = Running::start("shot");
    let picture = app.dir.join("window.png");
    let result = app
        .send("screenshot", json!({ "path": picture }))
        .expect("a screenshot");
    assert!(result["width"].as_u64().unwrap() > 100);
    assert!(std::fs::metadata(&picture).unwrap().len() > 1000);

    // Without the token nothing is done.
    let mut stranger = TcpStream::connect(app.writer.peer_addr().unwrap()).unwrap();
    writeln!(stranger, r#"{{"id": 1, "cmd": "quit"}}"#).unwrap();
    let mut line = String::new();
    BufReader::new(stranger).read_line(&mut line).unwrap();
    assert!(
        line.contains(r#""ok":false"#) && line.contains("token"),
        "{line}"
    );
    assert_eq!(app.send("hello", Value::Null).unwrap()["protocol"], 1);
    app.quit();
}
