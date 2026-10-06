// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use serde_json::json;

use crate::{
    Outcome,
    manifest::{Entry, Manifest},
    probe,
    targets::{self, Target},
    util::{Out, hinted, now_rfc3339, sha256_hex, write_atomic},
    wrapper,
};

pub static BRIDGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/GameIconBridge.dylib"));
const BRIDGE_FILE: &str = "GameIconBridge.dylib";
const ICON_FILE: &str = "icon.png";

#[derive(ClapArgs)]
pub struct Args {
    /// Folder name in Application Support, or an absolute path (repeatable).
    #[arg(long = "target", value_name = "NAME|PATH")]
    targets: Vec<String>,
    /// Custom PNG to use instead of the clipped game icon (remembered for later runs).
    #[arg(long, value_name = "PNG")]
    icon: Option<PathBuf>,
    /// Print the plan and change nothing.
    #[arg(long)]
    dry_run: bool,
    /// Overwrite a foreign `.real` file, and re-verify targets that are already current.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Plan {
    Install,
    Refresh,
    Current,
}

impl Plan {
    fn verb(self) -> &'static str {
        match self {
            Plan::Install => "patch",
            Plan::Refresh => "refresh",
            Plan::Current => "current",
        }
    }
}

enum Undo {
    Swapped,
    Restore(Vec<u8>),
}

pub fn bridge_path(data: &Path) -> PathBuf {
    data.join(BRIDGE_FILE)
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    if BRIDGE.is_empty() {
        return Err(hinted(
            "this binary has no embedded bridge",
            "it was built on a non-macOS host; rebuild on macOS with `cargo build --release`",
        ));
    }
    let support = targets::support_dir()?;
    let data = targets::data_dir(&support);
    let found = targets::select(&args.targets, &support)?;
    let mut manifest = Manifest::load(&data)?;

    let bridge_path = bridge_path(&data);
    let bridge_sha = sha256_hex(BRIDGE);
    let bridge_ok = fs::read(&bridge_path).is_ok_and(|b| sha256_hex(&b) == bridge_sha);
    let icon_src = match &args.icon {
        Some(p) => Some(
            p.canonicalize()
                .with_context(|| format!("cannot read icon {}", p.display()))?,
        ),
        None => None,
    };
    let icon_path = icon_src
        .as_ref()
        .map(|_| data.join(ICON_FILE))
        .or_else(|| manifest.icon.as_ref().map(PathBuf::from));

    let mut plans = Vec::new();
    for t in &found {
        let expected = wrapper::render(t.loader, &bridge_path, icon_path.as_deref());
        let mut plan = plan_for(t, &expected, bridge_ok && icon_src.is_none(), args.force)?;
        if plan == Plan::Current && args.force {
            plan = Plan::Refresh;
        }
        plans.push((t, expected, plan));
    }

    if args.dry_run {
        let rows: Vec<_> = plans
            .iter()
            .map(|(t, _, p)| json!({"dir": t.bin, "plan": p.verb()}))
            .collect();
        out.emit(
            || {
                plans
                    .iter()
                    .map(|(t, _, p)| format!("would {}: {}", p.verb(), t.label()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            json!({"dry_run": true, "targets": rows}),
        );
        return Ok(Outcome::Done);
    }
    if plans.iter().all(|(_, _, p)| *p == Plan::Current) {
        out.emit(|| "already current".into(), json!({"result": "current"}));
        return Ok(Outcome::Nothing);
    }
    if let Some(name) = probe::running_wine() {
        return Err(hinted(
            format!("`{name}` is running"),
            "quit the game and YAAGL's Wine processes, then run apply again",
        ));
    }

    fs::create_dir_all(&data)?;
    if !bridge_ok {
        write_atomic(&bridge_path, BRIDGE, 0o755)?;
    }
    if let Some(src) = &icon_src {
        write_atomic(&data.join(ICON_FILE), &fs::read(src)?, 0o644)?;
        manifest.icon = Some(data.join(ICON_FILE).to_string_lossy().into_owned());
    }

    let mut results = Vec::new();
    let mut first_error = None;
    for (t, expected, plan) in plans {
        if plan == Plan::Current {
            results.push(json!({"dir": t.bin, "result": "current"}));
            continue;
        }
        match apply_one(t, &expected, plan) {
            Ok(()) => {
                manifest.cli_version = env!("CARGO_PKG_VERSION").into();
                manifest.bridge_sha256 = bridge_sha.clone();
                manifest.upsert(Entry {
                    dir: t.bin.clone(),
                    loader: t.loader.into(),
                    real: format!("{}.real", t.loader),
                    wrapper_sha256: sha256_hex(expected.as_bytes()),
                    applied_at: now_rfc3339(),
                });
                manifest.save(&data)?;
                results.push(json!({"dir": t.bin, "result": "patched"}));
            }
            Err(e) => {
                results.push(json!({"dir": t.bin, "result": "failed", "error": format!("{e:#}")}));
                if first_error.is_none() {
                    first_error = Some(e);
                } else {
                    eprintln!("error: {}: {e:#}", t.label());
                }
            }
        }
    }
    out.emit(
        || {
            results
                .iter()
                .map(|r| {
                    format!(
                        "{}: {}",
                        r["result"].as_str().unwrap_or(""),
                        r["dir"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        json!({"targets": results}),
    );
    match first_error {
        Some(e) => Err(e),
        None => Ok(Outcome::Done),
    }
}

fn plan_for(t: &Target, expected: &str, bridge_ok: bool, force: bool) -> Result<Plan> {
    let (loader, real) = (t.loader_path(), t.real_path());
    if loader.exists() && wrapper::is_wrapper(&loader) {
        if !real.exists() {
            return Err(hinted(
                format!(
                    "{} is our wrapper but {} is missing",
                    loader.display(),
                    real.display()
                ),
                "reinstall Wine from YAAGL to get a clean loader, then run apply again",
            ));
        }
        let link_ok = !t.preloader_src().exists() || t.preloader_link().symlink_metadata().is_ok();
        let same = fs::read_to_string(&loader).is_ok_and(|c| c == expected);
        return Ok(if same && bridge_ok && link_ok {
            Plan::Current
        } else {
            Plan::Refresh
        });
    }
    if !loader.exists() {
        return Err(hinted(
            format!("{} does not exist", loader.display()),
            "the original loader is gone; reinstall Wine from YAAGL",
        ));
    }
    if real.exists() && !force {
        return Err(hinted(
            format!(
                "{} already exists but {} is not our wrapper",
                real.display(),
                loader.display()
            ),
            "inspect both files; pass --force to overwrite the .real file",
        ));
    }
    Ok(Plan::Install)
}

fn apply_one(t: &Target, expected: &str, plan: Plan) -> Result<()> {
    let (loader, real) = (t.loader_path(), t.real_path());
    let undo = if plan == Plan::Install {
        fs::rename(&loader, &real)
            .with_context(|| format!("cannot rename {}", loader.display()))?;
        Undo::Swapped
    } else {
        Undo::Restore(fs::read(&loader)?)
    };

    let mut link_created = false;
    let outcome = (|| -> Result<()> {
        write_atomic(&loader, expected.as_bytes(), 0o755)?;
        link_created = ensure_preloader_link(t)?;
        verify(t)
    })();

    if outcome.is_err() {
        // Best effort: the original error is what the user needs to see.
        let _ = match undo {
            Undo::Swapped => fs::rename(&real, &loader).map_err(anyhow::Error::from),
            Undo::Restore(old) => write_atomic(&loader, &old, 0o755),
        };
        if link_created {
            let _ = fs::remove_file(t.preloader_link());
        }
    }
    outcome
}

/// Wine's macOS launcher re-executes a sibling `<name>-preloader`; after the rename it
/// looks for `wine64.real-preloader`. Returns true if a link was created.
fn ensure_preloader_link(t: &Target) -> Result<bool> {
    let link = t.preloader_link();
    if !t.preloader_src().exists() || link.symlink_metadata().is_ok() {
        return Ok(false);
    }
    symlink(format!("{}-preloader", t.loader), &link)
        .with_context(|| format!("cannot create {}", link.display()))?;
    Ok(true)
}

fn verify(t: &Target) -> Result<()> {
    let smoke = probe::smoke_test(&t.loader_path());
    if !smoke.version_ok {
        return Err(hinted(
            format!(
                "`{} --version` failed after the loader was replaced: {}",
                t.loader, smoke.output
            ),
            "Wine's launcher may depend on its own file name; the original was restored",
        ));
    }
    if !smoke.marker {
        return Err(hinted(
            "the bridge did not load (dyld ignored DYLD_INSERT_LIBRARIES)",
            format!(
                "Wine is probably hardened, so a different injection route is needed; the original was restored.\n{}",
                probe::entitlements(&t.real_path())
            ),
        ));
    }
    Ok(())
}
