//! `cliotree-cli`: see DESIGN.md, "Batch export from the command line".

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cliotree_cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
