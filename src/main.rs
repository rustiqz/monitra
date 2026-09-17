//! Entry point. Phase 1: proves the workspace builds and runs. Real command
//! dispatch (via the `cli` crate) lands in Phase 2 — this is deliberately not
//! that yet.

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("version") => println!("monitra {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("monitra: unknown or missing command (try: version)");
            std::process::exit(1);
        }
    }
}
