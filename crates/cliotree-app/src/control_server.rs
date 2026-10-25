//! The test control channel of the running application (DESIGN.md, "The
//! test-control protocol"). It is compiled only with the `test-control` cargo
//! feature, which release builds never enable, so the shipped application has
//! no remote-control surface.
//!
//! It listens on a loopback port chosen by the operating system, writes the port
//! to the file named by `CLIOTREE_TEST_PORT_FILE`, and answers only requests
//! that carry the token in `CLIOTREE_TEST_TOKEN`. It speaks JSON lines, one
//! request and one response per line. The commands themselves are in `control.rs`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::control;
use crate::{ClioApp, control::PROTOCOL};

/// How long a request waits for the window to answer it.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(180);

/// A request on its way to the window, with the way back.
struct Job {
    cmd: String,
    args: Json,
    reply: Sender<Result<Json, String>>,
}

/// The application with the channel to its tests.
pub struct Controlled {
    app: ClioApp,
    jobs: Receiver<Job>,
}

impl Controlled {
    /// Starts listening. `ctx` is used to wake the window when a request comes.
    pub fn start(app: ClioApp, ctx: egui::Context) -> Result<Controlled, String> {
        let token = std::env::var("CLIOTREE_TEST_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .ok_or("CLIOTREE_TEST_TOKEN must be set to a token the tests will send")?;
        let port_file: PathBuf = std::env::var_os("CLIOTREE_TEST_PORT_FILE")
            .filter(|p| !p.is_empty())
            .ok_or("CLIOTREE_TEST_PORT_FILE must name the file to write the port to")?
            .into();
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("cannot listen: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        // Written under another name and renamed, so a reader never sees half.
        let partial = port_file.with_extension("partial");
        std::fs::write(&partial, port.to_string())
            .and_then(|()| std::fs::rename(&partial, &port_file))
            .map_err(|e| format!("cannot write {}: {e}", port_file.display()))?;

        let (jobs_in, jobs) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (token, jobs_in, ctx) = (token.clone(), jobs_in.clone(), ctx.clone());
                thread::spawn(move || serve(stream, &token, &jobs_in, &ctx));
            }
        });
        Ok(Controlled { app, jobs })
    }

    /// Does what the requests that have come in ask.
    fn answer_requests(&mut self, ctx: &egui::Context) {
        while let Ok(job) = self.jobs.try_recv() {
            match job.cmd.as_str() {
                "quit" => {
                    let _ = job.reply.send(Ok(Json::Null));
                    // A test that asks to quit means it: no question about
                    // unsaved changes.
                    self.app.files.quit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                "screenshot" => self.screenshot(ctx, job),
                _ => {
                    let result = control::run(&mut self.app, &job.cmd, &job.args);
                    let _ = job.reply.send(result);
                }
            }
            ctx.request_repaint();
        }
    }

    /// Answers when the rendered window has been read back and saved.
    fn screenshot(&self, ctx: &egui::Context, job: Job) {
        let Some(path) = job.args.get("path").and_then(Json::as_str) else {
            let _ = job
                .reply
                .send(Err("the argument path (text) is missing".into()));
            return;
        };
        let path = PathBuf::from(path);
        let reply = job.reply;
        ctx.request_screenshot(move |image| {
            let [width, height] = image.size;
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let saved = image::RgbaImage::from_raw(width as u32, height as u32, bytes)
                .ok_or_else(|| "the screenshot has the wrong size".to_owned())
                .and_then(|img| img.save(&path).map_err(|e| e.to_string()));
            let _ = reply.send(saved.map(|()| json!({ "width": width, "height": height })));
        });
    }
}

impl eframe::App for Controlled {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.answer_requests(&ctx);
        self.app.show(ui);
        // A request can come while nothing else wakes the window.
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

/// One connection: a request per line, an answer per request.
fn serve(stream: TcpStream, token: &str, jobs: &Sender<Job>, ctx: &egui::Context) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Json>(&line) {
            Err(e) => response(&Json::Null, Err(format!("not JSON: {e}"))),
            Ok(request) => {
                let id = request.get("id").cloned().unwrap_or(Json::Null);
                response(&id, handle(&request, token, jobs, ctx))
            }
        };
        if writeln!(out, "{answer}").is_err() {
            return;
        }
    }
}

fn handle(
    request: &Json,
    token: &str,
    jobs: &Sender<Job>,
    ctx: &egui::Context,
) -> Result<Json, String> {
    if request.get("token").and_then(Json::as_str) != Some(token) {
        return Err("the token is missing or wrong".into());
    }
    let cmd = request
        .get("cmd")
        .and_then(Json::as_str)
        .ok_or("the request has no cmd")?;
    // The version needs no window.
    if cmd == "hello" {
        return Ok(json!({ "protocol": PROTOCOL, "app": "ClioTree" }));
    }
    let (reply, answer) = mpsc::channel();
    let job = Job {
        cmd: cmd.to_owned(),
        args: request.get("args").cloned().unwrap_or(Json::Null),
        reply,
    };
    jobs.send(job).map_err(|_| "the application has closed")?;
    ctx.request_repaint();
    answer
        .recv_timeout(ANSWER_TIMEOUT)
        .map_err(|_| "the application did not answer".to_owned())?
}

fn response(id: &Json, result: Result<Json, String>) -> Json {
    match result {
        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    }
}
