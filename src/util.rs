// SPDX-License-Identifier: MPL-2.0

use std::{
    fmt,
    fs::{self, Permissions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// An error that carries a `hint:` line for the user.
#[derive(Debug)]
pub struct Hinted {
    msg: String,
    pub hint: String,
}

impl fmt::Display for Hinted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Hinted {}

pub fn hinted(msg: impl Into<String>, hint: impl Into<String>) -> anyhow::Error {
    Hinted {
        msg: msg.into(),
        hint: hint.into(),
    }
    .into()
}

pub struct Out {
    pub json: bool,
}

impl Out {
    pub fn emit(&self, human: impl FnOnce() -> String, value: serde_json::Value) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("json value serializes")
            );
        } else {
            println!("{}", human());
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Writes `<path>.new` then renames it over `path`, so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".new");
    let tmp = PathBuf::from(tmp);
    let mut file =
        fs::File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.set_permissions(Permissions::from_mode(mode))?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))
}

/// Current UTC time as RFC 3339, without a date-time dependency.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn timestamp_has_rfc3339_shape() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 20);
        assert!(t.ends_with('Z') && t.as_bytes()[10] == b'T');
    }
}
