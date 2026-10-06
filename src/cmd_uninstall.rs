// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use serde_json::json;

use crate::{
    Outcome, cmd_revert,
    manifest::Manifest,
    probe, targets,
    util::{Out, hinted},
};

#[derive(ClapArgs)]
pub struct Args {
    /// Leave the yaagl-squircle binary in place.
    #[arg(long)]
    keep_binary: bool,
    /// Do not ask for confirmation.
    #[arg(long)]
    yes: bool,
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    let support = targets::support_dir()?;
    let data = targets::data_dir(&support);
    if !args.yes && !confirm(&data, args.keep_binary)? {
        out.emit(|| "aborted".into(), json!({"result": "aborted"}));
        return Ok(Outcome::Nothing);
    }
    if let Some(name) = probe::running_wine() {
        return Err(hinted(
            format!("`{name}` is running"),
            "quit the game and YAAGL's Wine processes first",
        ));
    }

    let mut manifest = Manifest::load(&data)?;
    let bins = cmd_revert::all_bins(&support, &manifest);
    let rows = cmd_revert::revert_all(&bins, &mut manifest)?;
    if let Some((dir, _)) = rows.iter().find(|(_, r)| *r == "kept") {
        return Err(hinted(
            format!("could not restore {}", dir.display()),
            "fix or reinstall that Wine folder, then run uninstall again; nothing was deleted",
        ));
    }

    match fs::remove_dir_all(&data) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("cannot delete {}", data.display()));
        }
        _ => {}
    }
    if !args.keep_binary {
        let exe = std::env::current_exe()?;
        fs::remove_file(&exe).with_context(|| format!("cannot delete {}", exe.display()))?;
    }
    out.emit(
        || "uninstalled".into(),
        json!({"result": "uninstalled", "binary_removed": !args.keep_binary}),
    );
    Ok(Outcome::Done)
}

fn confirm(data: &Path, keep_binary: bool) -> Result<bool> {
    let binary = if keep_binary { "" } else { " and the binary" };
    print!(
        "Revert all patches, delete {}{binary}? [y/N] ",
        data.display()
    );
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}
