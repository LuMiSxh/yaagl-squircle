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
    Ok,
    NotPatched,
    Drifted,
    Wiped,
    Orphan,
    Broken,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Ok => "ok",
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
        all_ok &= state == State::Ok;
        let mut row = json!({"dir": bin, "state": state.name(), "detail": detail});
        if args.verbose
            && state == State::Ok
            && let Some(t) = target.as_ref()
        {
            let smoke = probe::smoke_test(&t.loader_path());
            row["smoke"] = json!({"version_ok": smoke.version_ok, "bridge_loaded": smoke.marker});
            row["signing"] = json!(probe::entitlements(&t.real_path()));
        }
        rows.push(row);
    }

    out.emit(
        || render(&rows),
        json!({"bridge_embedded": !BRIDGE.is_empty(), "targets": rows}),
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
    let mut lines = Vec::new();
    for r in rows {
        lines.push(format!(
            "{:<12} {}",
            r["state"].as_str().unwrap_or(""),
            r["dir"].as_str().unwrap_or("")
        ));
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
    }
    lines.join("\n")
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
                (State::Ok, String::new())
            }
        }
        (false, true) => (
            State::Orphan,
            format!(
                "{} exists without our wrapper; run: yaagl-squircle revert",
                real.display()
            ),
        ),
        (false, false) if entry.is_some() => (
            State::Wiped,
            "Wine was replaced (normal after a YAAGL update); run: yaagl-squircle apply".into(),
        ),
        (false, false) => (State::NotPatched, "run: yaagl-squircle apply".into()),
    }
}
