// SPDX-License-Identifier: MPL-2.0

mod cmd_apply;
mod cmd_revert;
mod cmd_status;
mod cmd_uninstall;
#[cfg(feature = "cmd-update")]
mod cmd_update;
mod manifest;
mod probe;
mod targets;
mod util;
mod wrapper;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// What a command did. Maps to exit codes 0 (done) and 1 (nothing to do); errors exit 2.
pub enum Outcome {
    Done,
    Nothing,
}

#[derive(Parser)]
#[command(name = "yaagl-squircle", version)]
struct Cli {
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    /// Print human-readable text (default).
    #[arg(long, global = true, conflicts_with = "json")]
    human: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Patch installed YAAGL Wine loaders so game Dock icons are squircles.
    Apply(cmd_apply::Args),
    /// Show what is patched, what drifted, and whether the bridge loads.
    Status(cmd_status::Args),
    /// Restore the original Wine loader(s); keeps our data folder.
    Revert(cmd_revert::Args),
    /// Update this CLI from GitHub releases, then refresh existing patches.
    #[cfg(feature = "cmd-update")]
    Update(cmd_update::Args),
    /// Revert everything, delete our data folder and (by default) the binary.
    Uninstall(cmd_uninstall::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = util::Out { json: cli.json };
    let result = match cli.cmd {
        Cmd::Apply(a) => cmd_apply::run(a, &out),
        Cmd::Status(a) => cmd_status::run(a, &out),
        Cmd::Revert(a) => cmd_revert::run(a, &out),
        #[cfg(feature = "cmd-update")]
        Cmd::Update(a) => cmd_update::run(a, &out),
        Cmd::Uninstall(a) => cmd_uninstall::run(a, &out),
    };
    match result {
        Ok(Outcome::Done) => ExitCode::SUCCESS,
        Ok(Outcome::Nothing) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e:#}");
            if let Some(h) = e.downcast_ref::<util::Hinted>() {
                eprintln!("hint: {}", h.hint);
            }
            ExitCode::from(2)
        }
    }
}
