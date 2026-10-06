// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use serde_json::json;

use crate::{
    Outcome,
    manifest::Manifest,
    probe,
    targets::{self, Target},
    util::{Out, hinted},
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
    let reverted = rows.iter().any(|(_, r)| *r == "reverted");
    out.emit(
        || rows.iter().map(|(d, r)| format!("{r}: {}", d.display())).collect::<Vec<_>>().join("\n"),
        json!({"targets": rows.iter().map(|(d, r)| json!({"dir": d, "result": r})).collect::<Vec<_>>()}),
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
pub fn revert_all(
    bins: &[PathBuf],
    manifest: &mut Manifest,
) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut rows = Vec::new();
    for bin in bins {
        let result = match Target::from_bin(bin) {
            Some(t) => revert_target(&t)?,
            None => "gone",
        };
        if result != "kept" {
            manifest.remove(bin);
        }
        rows.push((bin.clone(), result));
    }
    Ok(rows)
}

fn revert_target(t: &Target) -> Result<&'static str> {
    let (loader, real) = (t.loader_path(), t.real_path());
    if !(loader.exists() && wrapper::is_wrapper(&loader)) {
        return Ok(if real.exists() { "kept" } else { "not patched" });
    }
    if !real.exists() {
        eprintln!(
            "warning: {} is our wrapper but {} is missing; nothing safe to restore, left in place",
            loader.display(),
            real.display()
        );
        return Ok("kept");
    }
    fs::rename(&real, &loader).with_context(|| format!("cannot restore {}", loader.display()))?;
    if t.preloader_link()
        .symlink_metadata()
        .is_ok_and(|m| m.is_symlink())
    {
        fs::remove_file(t.preloader_link())?;
    }
    Ok("reverted")
}
