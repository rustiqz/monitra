//! `monitra alert …` — argument shape only (DESIGN.md §5.1, ADR-009's
//! `AlertEvent`; command added Phase 9 so alert history is reachable from
//! the CLI, not just the TUI/web — P3).

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum AlertCommand {
    /// List every recorded alert event, across every monitor, most recent first.
    List,
}
