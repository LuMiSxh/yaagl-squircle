// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use clap::Args as ClapArgs;
use serde_json::{Value, json};

use crate::{
    Outcome,
    cmd_apply::{self, BRIDGE},
    manifest::Manifest,
    probe,
    targets::{self, Target},
    util::{Out, sha256_hex},
    wrapper,
};

#[derive(ClapArgs)]
pub struct Args {
    /// Also run the smoke test and print signing details.
    #[arg(long)]
    verbose: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Applied,
    NotPatched,
    Drifted,
    Wiped,
    Orphan,
    Broken,
}

impl State {
    fn name(self, deep: bool) -> &'static str {
        match self {
            State::Applied if deep => "applied (deep)",
            State::Applied => "applied",
            State::NotPatched => "not patched",
            State::Drifted => "drifted",
            State::Wiped => "wiped",
            State::Orphan => "orphan",
            State::Broken => "broken",
        }
    }
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    let support = targets::support_dir()?;
    let data = targets::data_dir(&support);
    let manifest = Manifest::load(&data)?;

    let mut bins: BTreeSet<PathBuf> = targets::discover(&support)
        .into_iter()
        .map(|t| t.bin)
        .collect();
    bins.extend(manifest.targets.iter().map(|e| e.dir.clone()));

    let bridge_sha = fs::read(cmd_apply::bridge_path(&data))
        .map(|b| sha256_hex(&b))
        .ok();
    let mut rows = Vec::new();
    let mut all_ok = !bins.is_empty();
    for bin in bins {
        let target = Target::from_bin(&bin);
        let (state, detail) = classify(&bin, target.as_ref(), &manifest, bridge_sha.as_deref());
        all_ok &= state == State::Applied;
        let deep = target.as_ref().is_some_and(|t| t.launcher.is_some());
        let files: Vec<_> = target
            .as_ref()
            .map(Target::files)
            .unwrap_or_default()
            .into_iter()
            .map(|(role, p)| {
                json!({
                    "role": role,
                    "name": p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "is": targets::describe(&p),
                })
            })
            .collect();
        let mut row = json!({
            "dir": bin,
            "label": target.as_ref().map(Target::label),
            "layout": target.as_ref().map(Target::layout),
            "deep": deep,
            "state": state.name(deep),
            "detail": detail,
            "files": files,
            "applied_at": manifest.entry(&bin).map(|e| e.applied_at.clone()),
        });
        if args.verbose
            && state == State::Applied
            && let Some(t) = target.as_ref()
        {
            let smoke = probe::smoke_test(&t.loader_path());
            row["smoke"] = json!({"version_ok": smoke.version_ok, "bridge_loaded": smoke.marker});
            row["signing"] = json!(probe::entitlements(&t.real_path()));
        }
        rows.push(row);
    }

    out.emit(
        || {
            let mut s = render(&rows);
            let bridge = cmd_apply::bridge_path(&data);
            let bridge_state = match bridge_sha.as_deref() {
                None => "missing",
                Some(sha) if !BRIDGE.is_empty() && sha != sha256_hex(BRIDGE) => "outdated",
                Some(_) => "present",
            };
            s.push_str(&format!("\n\nbridge  {bridge_state}  {}", bridge.display()));
            s.push_str(&format!(
                "\nicon    {}",
                manifest.icon.as_deref().unwrap_or("clipped game icon")
            ));
            s.push_str(&if manifest.debug {
                format!("\ndebug   on, log {}", cmd_apply::log_path(&data).display())
            } else {
                "\ndebug   off".to_owned()
            });
            s
        },
        json!({
            "bridge_embedded": !BRIDGE.is_empty(),
            "bridge_installed": bridge_sha.is_some(),
            "icon": manifest.icon,
            "debug": manifest.debug,
            "targets": rows,
        }),
    );
    Ok(if all_ok {
        Outcome::Done
    } else {
        Outcome::Nothing
    })
}

fn render(rows: &[Value]) -> String {
    if rows.is_empty() {
        return "no YAAGL Wine install found\nhint: start YAAGL once so it downloads Wine".into();
    }
    let mut blocks = Vec::new();
    for r in rows {
        let mut lines = Vec::new();
        let label = r["label"].as_str().unwrap_or("");
        lines.push(match r["layout"].as_str() {
            Some(layout) => format!("{}: {label}  ({layout})", r["state"].as_str().unwrap_or("")),
            None => format!("{}: {label}", r["state"].as_str().unwrap_or("")),
        });
        lines.push(format!("  {}", r["dir"].as_str().unwrap_or("")));
        for f in r["files"].as_array().into_iter().flatten() {
            lines.push(format!(
                "    {:<10}{:<20}{}",
                f["role"].as_str().unwrap_or(""),
                f["name"].as_str().unwrap_or(""),
                f["is"].as_str().unwrap_or("")
            ));
        }
        if let Some(at) = r["applied_at"].as_str() {
            lines.push(format!("    applied   {at}"));
        }
        if let Some(d) = r["detail"].as_str().filter(|d| !d.is_empty()) {
            lines.push(format!("  {d}"));
        }
        if let Some(s) = r.get("smoke") {
            lines.push(format!(
                "  smoke test: version {}, bridge {}",
                pass(&s["version_ok"]),
                pass(&s["bridge_loaded"])
            ));
            lines.push(format!(
                "  {}",
                r["signing"].as_str().unwrap_or("").replace('\n', "\n  ")
            ));
        }
        blocks.push(lines.join("\n"));
    }
    blocks.join("\n\n")
}

fn pass(v: &Value) -> &'static str {
    if v.as_bool() == Some(true) {
        "ok"
    } else {
        "FAILED"
    }
}

fn classify(
    bin: &Path,
    target: Option<&Target>,
    m: &Manifest,
    bridge_sha: Option<&str>,
) -> (State, String) {
    let entry = m.entry(bin);
    let Some(t) = target else {
        return (
            State::Wiped,
            "folder is gone; run: yaagl-squircle revert to forget it".into(),
        );
    };
    let (loader, real) = (t.loader_path(), t.real_path());
    let patched = loader.exists() && wrapper::is_wrapper(&loader);
    match (patched, real.exists()) {
        (true, false) => (
            State::Broken,
            "wrapper present but the original loader is missing".into(),
        ),
        (true, true) => {
            let wrapper_sha = fs::read(&loader)
                .map(|b| sha256_hex(&b))
                .unwrap_or_default();
            if entry.is_none_or(|e| e.wrapper_sha256 != wrapper_sha) {
                (
                    State::Drifted,
                    "wrapper differs from the manifest; run: yaagl-squircle apply".into(),
                )
            } else if bridge_sha != Some(m.bridge_sha256.as_str()) {
                (
                    State::Drifted,
                    "bridge dylib is missing or changed; run: yaagl-squircle apply".into(),
                )
            } else if !BRIDGE.is_empty() && bridge_sha != Some(sha256_hex(BRIDGE).as_str()) {
                (
                    State::Drifted,
                    "this CLI ships a newer bridge; run: yaagl-squircle apply".into(),
                )
            } else {
                (State::Applied, String::new())
            }
        }
        (false, true) => (
            State::Orphan,
            format!(
                "{} exists without our wrapper and is left alone; if it is a leftover, reinstall Wine from YAAGL",
                real.display()
            ),
        ),
        (false, false) if entry.is_some() => (
            State::Wiped,
            "Wine was replaced (normal after a YAAGL update); run: yaagl-squircle apply".into(),
        ),
        (false, false) if t.launcher.is_some() => (
            State::NotPatched,
            "a launcher script that is not ours sits in front of Wine; run: yaagl-squircle apply (it asks before patching the binary behind it)".into(),
        ),
        (false, false) => (State::NotPatched, "run: yaagl-squircle apply".into()),
    }
}
