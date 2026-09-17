//! Entry point. Phase 2: real command parsing via `cli::Cli`, but no
//! execution yet — every command besides `version` just prints what it
//! parsed. Wiring into `engine`/`backend`/`storage` starts once those crates
//! exist (Phase 3 onward).

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Version => println!("monitra {}", env!("CARGO_PKG_VERSION")),
        other => println!("{other:#?}"),
    }
}
