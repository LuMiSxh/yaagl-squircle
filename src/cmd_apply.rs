// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    io::{self, IsTerminal, Write},
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
    util::{Out, hinted, now_rfc3339, sha256_hex, step, write_atomic},
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
    /// Re-verify targets that are already current.
    #[arg(long)]
    force: bool,
    /// Patch the Wine binary behind a foreign launcher script without asking.
    #[arg(long)]
    deep: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Plan {
    Install,
    Refresh,
    Current,
    Declined,
}

impl Plan {
    fn verb(self) -> &'static str {
        match self {
            Plan::Install => "patch",
            Plan::Refresh => "refresh",
            Plan::Current => "current",
            Plan::Declined => "skip",
        }
    }
}

enum Undo {
    Swapped,
    Restore(Vec<u8>),
}

pub fn log_path(data: &Path) -> PathBuf {
    data.join("bridge.log")
}

/// Arguments that re-apply exactly the given folders, for commands that change the wrapper.
pub fn refresh_args(targets: Vec<String>) -> Args {
    Args {
        targets,
        icon: None,
        dry_run: false,
        force: false,
        deep: false,
    }
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

    let log_path = manifest.debug.then(|| log_path(&data));
    let mut plans = Vec::new();
    for t in &found {
        let expected = wrapper::render(
            t.loader,
            &bridge_path,
            icon_path.as_deref(),
            log_path.as_deref(),
        );
        let mut plan = plan_for(t, &expected, bridge_ok && icon_src.is_none())?;
        if plan == Plan::Current && args.force {
            plan = Plan::Refresh;
        }
        plans.push((t, expected, plan));
    }

    if args.dry_run {
        let rows: Vec<_> = plans
            .iter()
            .map(|(t, _, p)| json!({"dir": t.bin, "plan": p.verb(), "deep": t.launcher.is_some()}))
            .collect();
        out.emit(
            || {
                plans
                    .iter()
                    .map(|(t, _, p)| {
                        format!(
                            "would {}: {}  ({})\n  {}",
                            p.verb(),
                            t.label(),
                            t.layout(),
                            t.bin.display()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            json!({"dry_run": true, "targets": rows}),
        );
        return Ok(Outcome::Done);
    }
    if plans.iter().all(|(_, _, p)| *p == Plan::Current) {
        out.emit(
            || {
                let mut s = "already current".to_owned();
                for (t, _, _) in &plans {
                    s.push_str(&format!("\n  {}  ({})", t.label(), t.layout()));
                }
                s
            },
            json!({"result": "current"}),
        );
        return Ok(Outcome::Nothing);
    }
    if let Some(name) = probe::running_wine() {
        return Err(hinted(
            format!("`{name}` is running"),
            "quit the game and YAAGL's Wine processes, then run apply again",
        ));
    }

    // A first deep patch goes into a file another tool owns: ask before anything changes.
    for (t, _, plan) in &mut plans {
        if *plan == Plan::Install && t.launcher.is_some() && !confirm_deep(t, args.deep)? {
            *plan = Plan::Declined;
        }
    }

    let mut setup = Vec::new();
    fs::create_dir_all(&data)?;
    if !bridge_ok {
        write_atomic(&bridge_path, BRIDGE, 0o755)?;
        step(
            &mut setup,
            "bridge",
            format!("wrote {}", bridge_path.display()),
        );
    }
    if let Some(src) = &icon_src {
        write_atomic(&data.join(ICON_FILE), &fs::read(src)?, 0o644)?;
        manifest.icon = Some(data.join(ICON_FILE).to_string_lossy().into_owned());
        step(
            &mut setup,
            "icon",
            format!(
                "copied {} -> {}",
                src.display(),
                data.join(ICON_FILE).display()
            ),
        );
    }

    let mut results = Vec::new();
    let mut first_error = None;
    for (t, expected, plan) in plans {
        let mut steps = Vec::new();
        let result = match plan {
            Plan::Current => {
                step(
                    &mut steps,
                    "nothing",
                    "wrapper, bridge and icon are current",
                );
                "current"
            }
            Plan::Declined => {
                step(&mut steps, "nothing", "declined; no file was changed");
                "skipped"
            }
            Plan::Install | Plan::Refresh => match apply_one(t, &expected, plan, &mut steps) {
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
                    if t.launcher.is_some() {
                        "patched (deep)"
                    } else {
                        "patched"
                    }
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    } else {
                        eprintln!("error: {}: {e:#}", t.label());
                    }
                    "failed"
                }
            },
        };
        results.push(json!({
            "dir": t.bin,
            "label": t.label(),
            "layout": t.layout(),
            "deep": t.launcher.is_some(),
            "result": result,
            "steps": steps,
        }));
    }
    out.emit(
        || {
            let mut blocks: Vec<String> = Vec::new();
            if !setup.is_empty() {
                blocks.push(setup.join("\n"));
            }
            blocks.extend(results.iter().map(render_row));
            blocks.join("\n\n")
        },
        json!({"setup": setup, "targets": results}),
    );
    let changed = results.iter().any(|r| {
        r["result"]
            .as_str()
            .is_some_and(|s| s.starts_with("patched"))
    });
    match first_error {
        Some(e) => Err(e),
        None if changed => Ok(Outcome::Done),
        None => Ok(Outcome::Nothing),
    }
}

/// One target as a block: result and name, its folder, then the steps taken.
pub fn render_row(r: &serde_json::Value) -> String {
    let mut s = format!(
        "{}: {}  ({})\n  {}",
        r["result"].as_str().unwrap_or(""),
        r["label"].as_str().unwrap_or(""),
        r["layout"].as_str().unwrap_or(""),
        r["dir"].as_str().unwrap_or("")
    );
    for line in r["steps"].as_array().into_iter().flatten() {
        s.push_str(&format!("\n    {}", line.as_str().unwrap_or("")));
    }
    s
}

fn confirm_deep(t: &Target, yes: bool) -> Result<bool> {
    let launcher = t.launcher.unwrap_or("wine");
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(hinted(
            format!(
                "{}: `{launcher}` is a launcher script that is not ours",
                t.label()
            ),
            format!(
                "pass --deep to patch the Wine binary behind it (`{}`), or leave this folder out with --target",
                t.loader
            ),
        ));
    }
    eprint!(
        "{label}: `{launcher}` is a launcher script that is not ours.\n\
         Wrapping it would not keep the bridge loaded, so the Wine binary behind it\n\
         would be patched instead: `{loader}` is renamed to `{loader}.real` and our\n\
         wrapper takes its place. `{launcher}` itself stays untouched.\n\
         Patch `{loader}`? [y/N] ",
        label = t.label(),
        loader = t.loader,
    );
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

fn plan_for(t: &Target, expected: &str, bridge_ok: bool) -> Result<Plan> {
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
    if real.exists() {
        return Err(hinted(
            format!(
                "{} already exists but {} is not our wrapper",
                real.display(),
                loader.display()
            ),
            "that file is not ours, so it is never overwritten; if it is a leftover, reinstall Wine from YAAGL",
        ));
    }
    Ok(Plan::Install)
}

fn apply_one(t: &Target, expected: &str, plan: Plan, steps: &mut Vec<String>) -> Result<()> {
    let (loader, real) = (t.loader_path(), t.real_path());
    let undo = if plan == Plan::Install {
        fs::rename(&loader, &real)
            .with_context(|| format!("cannot rename {}", loader.display()))?;
        step(
            steps,
            "rename",
            format!("{} -> {}.real", t.loader, t.loader),
        );
        Undo::Swapped
    } else {
        step(steps, "keep", format!("{}.real (original)", t.loader));
        Undo::Restore(fs::read(&loader)?)
    };

    let mut link_created = false;
    let outcome = (|| -> Result<()> {
        write_atomic(&loader, expected.as_bytes(), 0o755)?;
        step(steps, "write", format!("{} (our wrapper)", t.loader));
        link_created = ensure_preloader_link(t)?;
        if link_created {
            step(
                steps,
                "link",
                format!("{}.real-preloader -> {}-preloader", t.loader, t.loader),
            );
        }
        let version = verify(t)?;
        step(steps, "verify", format!("{version}, bridge loaded"));
        Ok(())
    })();

    if outcome.is_err() {
        step(steps, "verify", "FAILED, undoing");
        // Best effort: the original error is what the user needs to see.
        let (undone, what) = match undo {
            Undo::Swapped => (
                fs::rename(&real, &loader).map_err(anyhow::Error::from),
                format!("{}.real -> {}", t.loader, t.loader),
            ),
            Undo::Restore(old) => (
                write_atomic(&loader, &old, 0o755),
                format!("{} (previous wrapper)", t.loader),
            ),
        };
        match undone {
            Ok(()) => step(steps, "undo", what),
            Err(e) => step(steps, "undo", format!("FAILED to restore {what}: {e:#}")),
        }
        if link_created {
            let _ = fs::remove_file(t.preloader_link());
            step(
                steps,
                "undo",
                format!("removed {}.real-preloader", t.loader),
            );
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

/// Returns the version line Wine printed.
fn verify(t: &Target) -> Result<String> {
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
    Ok(smoke.output.lines().next().unwrap_or("").to_owned())
}
