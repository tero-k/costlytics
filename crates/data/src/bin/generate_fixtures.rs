//! Generates the local synthetic fixture datasets used for manual/dev
//! testing (`cargo run -p api` against real-ish data) and referenced by
//! `config/example.toml`'s `local-focus12`/`local-cur2` sources.
//!
//! Writes both:
//! - `fixtures/focus12/` — a synthetic FOCUS 1.2 Parquet dataset
//! - `fixtures/cur2/`    — a synthetic CUR 2.0 Parquet dataset
//!
//! relative to the current working directory (run from the workspace root,
//! e.g. via `cargo run -p data --bin generate-fixtures`). Both directories
//! are gitignored (`/fixtures/`) — this binary is the supported way to
//! (re)create them locally; it does not modify the tested generator
//! functions in `data::fixtures`, only calls them.

use data::fixtures::{generate_cur2_fixture, generate_focus12_fixture};
use std::path::Path;

fn main() {
    let focus12_dir = Path::new("fixtures/focus12");
    let cur2_dir = Path::new("fixtures/cur2");

    println!("Generating FOCUS 1.2 fixture at {}...", focus12_dir.display());
    if let Err(e) = generate_focus12_fixture(focus12_dir) {
        eprintln!("failed to generate FOCUS 1.2 fixture: {e}");
        std::process::exit(1);
    }
    println!("  done.");

    println!("Generating CUR 2.0 fixture at {}...", cur2_dir.display());
    if let Err(e) = generate_cur2_fixture(cur2_dir) {
        eprintln!("failed to generate CUR 2.0 fixture: {e}");
        std::process::exit(1);
    }
    println!("  done.");

    println!(
        "\nFixtures ready. `config/example.toml` already registers `local-focus12`/`local-cur2` \
         sources pointing at these directories — run `cargo run -p api` to query them."
    );
}
