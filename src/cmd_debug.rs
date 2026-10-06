// SPDX-License-Identifier: MPL-2.0

use std::fs;

use anyhow::Result;
use clap::{Args as ClapArgs, ValueEnum};
use serde_json::json;

use crate::{Outcome, cmd_apply, manifest::Manifest, targets, util::Out};

const TAIL_LINES: usize = 20;

#[derive(Clone, Copy, ValueEnum)]
enum Switch {
    On,
    Off,
}

#[derive(ClapArgs)]
pub struct Args {
    /// Turn the bridge's debug log on or off; omit to show the state and the log's tail.
    state: Option<Switch>,
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    let support = targets::support_dir()?;
    let data = targets::data_dir(&support);
    let mut manifest = Manifest::load(&data)?;
    let log = cmd_apply::log_path(&data);

    let Some(switch) = args.state else {
        let text = fs::read_to_string(&log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let tail: Vec<String> = lines[lines.len().saturating_sub(TAIL_LINES)..]
            .iter()
            .map(|l| (*l).to_owned())
            .collect();
        out.emit(
            || {
                let state = if manifest.debug { "on" } else { "off" };
                let mut s = format!("debug: {state}\nlog: {}", log.display());
                if !tail.is_empty() {
                    s.push_str(&format!(
                        "\n\nlast {} lines:\n{}",
                        tail.len(),
                        tail.join("\n")
                    ));
                }
                s
            },
            json!({"debug": manifest.debug, "log": log, "tail": tail}),
        );
        return Ok(Outcome::Done);
    };

    let want = matches!(switch, Switch::On);
    if manifest.debug == want {
        out.emit(
            || format!("debug is already {}", if want { "on" } else { "off" }),
            json!({"debug": want, "changed": false}),
        );
        return Ok(Outcome::Nothing);
    }

    manifest.debug = want;
    manifest.save(&data)?;
    let dirs: Vec<String> = manifest
        .targets
        .iter()
        .map(|e| e.dir.to_string_lossy().into_owned())
        .collect();
    if dirs.is_empty() {
        out.emit(
            || {
                format!(
                    "debug {}; takes effect on the next apply",
                    if want { "on" } else { "off" }
                )
            },
            json!({"debug": want, "changed": true, "rewritten": 0}),
        );
        return Ok(Outcome::Done);
    }

    // apply reads the flag from the manifest, so refreshing rewrites every wrapper.
    // The log is kept when turning debug off, so it can still be read afterwards.
    match cmd_apply::run(cmd_apply::refresh_args(dirs), out) {
        Ok(_) => Ok(Outcome::Done),
        Err(e) => {
            let mut m = Manifest::load(&data)?;
            m.debug = !want;
            m.save(&data)?;
            Err(e)
        }
    }
}
