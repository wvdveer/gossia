//! The ClioTree desktop application.
//!
//! The egui user interface arrives with build step 4 (see DESIGN.md, Build
//! order). Until then this binary only reports its version.

fn main() {
    println!("cliotree {}", cliotree_core::VERSION);
}
