// SPDX-License-Identifier: MPL-2.0

use std::{
    env,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SMOKE_TIMEOUT: Duration = Duration::from_secs(10);
const WINE_PROCESSES: [&str; 5] = [
    "wineserver",
    "wine64",
    "wine64-preloader",
    "wine",
    "wine-preloader",
];

pub struct Smoke {
    pub version_ok: bool,
    pub marker: bool,
    pub output: String,
}

/// Runs `<loader> --version` with the bridge's debug output on. It passes when Wine
/// still starts (stdout begins with `wine-`) and the bridge announced itself on stderr.
pub fn smoke_test(loader: &Path) -> Smoke {
    let child = Command::new(loader)
        .arg("--version")
        .env("YAAGL_SQUIRCLE_DEBUG", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return Smoke {
                version_ok: false,
                marker: false,
                output: e.to_string(),
            };
        }
    };
    let (mut stdout, mut stderr) = (child.stdout.take(), child.stderr.take());
    let readers = [
        thread::spawn(move || read_all(stdout.as_mut())),
        thread::spawn(move || read_all(stderr.as_mut())),
    ];

    let start = Instant::now();
    let mut timed_out = false;
    while child.try_wait().ok().flatten().is_none() {
        if start.elapsed() > SMOKE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let [out, err] = readers.map(|r| r.join().unwrap_or_default());
    let mut output = format!("{out}{err}");
    if timed_out {
        output.push_str("(timed out)");
    }
    Smoke {
        version_ok: !timed_out && out.starts_with("wine-"),
        marker: err.contains("yaagl-squircle: bridge loaded"),
        output: output.trim().to_owned(),
    }
}

fn read_all(stream: Option<&mut impl Read>) -> String {
    let mut buf = String::new();
    if let Some(s) = stream {
        let _ = s.read_to_string(&mut buf);
    }
    buf
}

/// Entitlements and signing flags of a binary, for explaining a blocked injection.
pub fn entitlements(binary: &Path) -> String {
    let run = |args: &[&str]| {
        Command::new("codesign")
            .args(args)
            .arg(binary)
            .output()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_else(|e| format!("codesign unavailable: {e}"))
    };
    let flags: Vec<String> = run(&["-dv"])
        .lines()
        .filter(|l| l.contains("flags="))
        .map(str::to_owned)
        .collect();
    let ents = run(&["-d", "--entitlements", "-"]);
    let dyld = ents.contains("allow-dyld-environment-variables");
    let lib = ents.contains("disable-library-validation");
    format!(
        "{}\nallow-dyld-environment-variables: {dyld}\ndisable-library-validation: {lib}",
        flags.join("\n")
    )
    .trim()
    .to_owned()
}

/// Name of a running Wine process, if any. Renaming the loader under a live game is unsafe.
pub fn running_wine() -> Option<&'static str> {
    if env::var_os("YAAGL_SQUIRCLE_NO_PGREP").is_some() {
        return None;
    }
    WINE_PROCESSES.into_iter().find(|name| {
        Command::new("pgrep")
            .args(["-x", name])
            .output()
            .is_ok_and(|o| o.status.success())
    })
}
