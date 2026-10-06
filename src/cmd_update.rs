// SPDX-License-Identifier: MPL-2.0

use std::{io::Read, process::Command};

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use serde_json::json;

use crate::{
    Outcome,
    util::{Out, hinted, sha256_hex, write_atomic},
};

const REPO: &str = "LuMiSxh/yaagl-squircle";
const BINARY: &str = "yaagl-squircle";
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

#[derive(ClapArgs)]
pub struct Args {
    /// Only report whether a newer release exists (exit 0 if so, 1 if not).
    #[arg(long)]
    check: bool,
    /// Reinstall even if already on the latest version.
    #[arg(long)]
    force: bool,
}

pub fn run(args: Args, out: &Out) -> Result<Outcome> {
    let current = env!("CARGO_PKG_VERSION");
    let release: serde_json::Value = http_get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?
    .body_mut()
    .read_json()
    .context("unexpected response from the GitHub releases API")?;
    let tag = release["tag_name"]
        .as_str()
        .context("release has no tag_name")?
        .to_owned();
    let latest = tag.trim_start_matches('v');
    let newer = is_newer(latest, current);

    if args.check {
        out.emit(
            || {
                if newer {
                    format!("update available: {current} -> {latest}")
                } else {
                    format!("up to date ({current})")
                }
            },
            json!({"current": current, "latest": latest, "update_available": newer}),
        );
        return Ok(if newer {
            Outcome::Done
        } else {
            Outcome::Nothing
        });
    }
    if !newer && !args.force {
        out.emit(
            || format!("up to date ({current})"),
            json!({"current": current, "latest": latest}),
        );
        return Ok(Outcome::Nothing);
    }

    let name = format!("{BINARY}-{}.tar.gz", target_triple()?);
    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    let archive = download(&format!("{base}/{name}"))?;
    let sums = String::from_utf8(download(&format!("{base}/{name}.sha256"))?)?;
    verify_sha256(&archive, &sums)?;
    let binary = extract_binary(&archive, BINARY)?;

    let exe = std::env::current_exe()?;
    // Renaming over the running binary is safe on unix; the old inode lives until exit.
    write_atomic(&exe, &binary, 0o755)?;
    out.emit(
        || format!("updated {current} -> {latest}"),
        json!({"updated_from": current, "updated_to": latest}),
    );

    let status = Command::new(&exe)
        .arg("apply")
        .status()
        .context("cannot run the new binary")?;
    match status.code() {
        Some(0 | 1) => Ok(Outcome::Done),
        _ => Err(hinted(
            "the new binary failed to refresh the patches",
            "run: yaagl-squircle apply",
        )),
    }
}

fn target_triple() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("aarch64-apple-darwin"),
        "x86_64" => Ok("x86_64-apple-darwin"),
        other => bail!("unsupported architecture {other}"),
    }
}

fn http_get(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    ureq::get(url)
        .header(
            "User-Agent",
            concat!("yaagl-squircle/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| {
            hinted(
                format!("request to {url} failed: {e}"),
                "check your connection and that the release exists",
            )
        })
}

fn download(url: &str) -> Result<Vec<u8>> {
    Ok(http_get(url)?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()?)
}

fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.split('.').map(|p| p.parse().ok()).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// `sums` is the content of a `.sha256` file: `<hex>  <file name>` (name optional).
fn verify_sha256(data: &[u8], sums: &str) -> Result<()> {
    let expected = sums.split_whitespace().next().unwrap_or_default();
    let actual = sha256_hex(data);
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(hinted(
            format!("checksum mismatch: expected {expected}, got {actual}"),
            "the download is corrupt or tampered with; nothing was installed",
        ));
    }
    Ok(())
}

fn extract_binary(tgz: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let is_match = entry.path()?.file_name().is_some_and(|n| n == name);
        if is_match {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    bail!("archive does not contain `{name}`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numeric_versions() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn checksum_accepts_match_and_rejects_mismatch() {
        let sum = sha256_hex(b"abc");
        assert!(verify_sha256(b"abc", &format!("{sum}  file.tar.gz\n")).is_ok());
        assert!(verify_sha256(b"abd", &sum).is_err());
    }

    #[test]
    fn extracts_binary_from_archive() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "yaagl-squircle", &b"bin"[..])
            .unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        let tgz = gz.finish().unwrap();
        assert_eq!(extract_binary(&tgz, "yaagl-squircle").unwrap(), b"bin");
        assert!(extract_binary(&tgz, "other").is_err());
    }
}
