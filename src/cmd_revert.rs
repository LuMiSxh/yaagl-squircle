// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use serde_json::json;

use crate::{
    Outcome,
    cmd_apply::render_row,
    manifest::Manifest,
    probe,
    targets::{self, Target},
    util::{Out, hinted, step},
    wrapper,
};

#[derive(ClapArgs)]
pub struct Args {
    /// Folder name in Application Support, or an absolute path (repeatable).
    #[arg(long = "target", value_name = "NAME|PATH")]
    targets: Vec<String>,
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    let support = targets::support_dir()?;
    let data = targets::data_dir(&support);
    let mut manifest = Manifest::load(&data)?;

    let selected: Vec<PathBuf> = if args.targets.is_empty() {
        all_bins(&support, &manifest)
    } else {
        args.targets
            .iter()
            .map(|a| targets::resolve(a, &support).map(|t| t.bin))
            .collect::<Result<_>>()?
    };
    if selected.is_empty() {
        out.emit(|| "nothing to revert".into(), json!({"targets": []}));
        return Ok(Outcome::Nothing);
    }
    if let Some(name) = probe::running_wine() {
        return Err(hinted(
            format!("`{name}` is running"),
            "quit the game and YAAGL's Wine processes first",
        ));
    }

    let rows = revert_all(&selected, &mut manifest)?;
    if data.exists() {
        manifest.save(&data)?;
    }
    let reverted = rows.iter().any(|r| r["result"] == "reverted");
    out.emit(
        || rows.iter().map(render_row).collect::<Vec<_>>().join("\n\n"),
        json!({"targets": rows}),
    );
    Ok(if reverted {
        Outcome::Done
    } else {
        Outcome::Nothing
    })
}

/// Every discovered install plus every folder the manifest still lists.
pub fn all_bins(support: &std::path::Path, manifest: &Manifest) -> Vec<PathBuf> {
    let mut bins: BTreeSet<PathBuf> = targets::discover(support)
        .into_iter()
        .map(|t| t.bin)
        .collect();
    bins.extend(manifest.targets.iter().map(|e| e.dir.clone()));
    bins.into_iter().collect()
}

/// Reverts each folder and drops it from the manifest unless something is left unrestored.
/// Each row has `dir`, `label`, `layout`, `result` and `steps`, like apply's.
pub fn revert_all(bins: &[PathBuf], manifest: &mut Manifest) -> Result<Vec<serde_json::Value>> {
    let mut rows = Vec::new();
    for bin in bins {
        let mut steps = Vec::new();
        let target = Target::from_bin(bin);
        let result = match &target {
            Some(t) => revert_target(t, &mut steps)?,
            None => {
                step(
                    &mut steps,
                    "forget",
                    "folder is gone; dropped from the manifest",
                );
                "gone"
            }
        };
        if result != "kept" {
            manifest.remove(bin);
        }
        rows.push(json!({
            "dir": bin,
            "label": target.as_ref().map_or_else(|| bin.display().to_string(), Target::label),
            "layout": target.as_ref().map_or_else(|| "missing".into(), Target::layout),
            "result": result,
            "steps": steps,
        }));
    }
    Ok(rows)
}

fn revert_target(t: &Target, steps: &mut Vec<String>) -> Result<&'static str> {
    let (loader, real) = (t.loader_path(), t.real_path());
    if !(loader.exists() && wrapper::is_wrapper(&loader)) {
        if real.exists() {
            step(
                steps,
                "kept",
                format!(
                    "{}.real exists but {} is not our wrapper; neither is ours, both left alone",
                    t.loader, t.loader
                ),
            );
            return Ok("kept");
        }
        step(steps, "nothing", format!("{} is not patched", t.loader));
        return Ok("not patched");
    }
    if !real.exists() {
        eprintln!(
            "warning: {} is our wrapper but {} is missing; nothing safe to restore, left in place",
            loader.display(),
            real.display()
        );
        step(
            steps,
            "kept",
            format!("{}.real is missing; wrapper left in place", t.loader),
        );
        return Ok("kept");
    }
    fs::rename(&real, &loader).with_context(|| format!("cannot restore {}", loader.display()))?;
    step(
        steps,
        "restore",
        format!("{}.real -> {}", t.loader, t.loader),
    );
    if t.preloader_link()
        .symlink_metadata()
        .is_ok_and(|m| m.is_symlink())
    {
        fs::remove_file(t.preloader_link())?;
        step(steps, "remove", format!("{}.real-preloader", t.loader));
    }
    if let Some(l) = t.launcher {
        step(
            steps,
            "keep",
            format!("{l} (launcher script, not ours, untouched)"),
        );
    }
    Ok("reverted")
}
