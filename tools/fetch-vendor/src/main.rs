//! Standalone entry point for the vendor commands: `fetch-vendor <fetch|status|verify|update>`.
//! The same commands are available as `cargo xtask vendor ...`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = fetch_vendor::run_vendor_command(&args) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}
