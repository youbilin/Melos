//! Terminal frontend (ratatui, added in v1.1).
//!
//! It shares `melos-core` with the GUI, which is the point: if the terminal can
//! drive the app, the business logic is genuinely UI-agnostic.

fn main() {
    eprintln!(
        "melos-tui {} - terminal frontend lands in v1.1",
        env!("CARGO_PKG_VERSION")
    );
}
