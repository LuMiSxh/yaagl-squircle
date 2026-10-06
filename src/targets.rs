// SPDX-License-Identifier: MPL-2.0

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::util::hinted;

/// An installed Wine `bin` folder inside a YAAGL data directory.
#[derive(Clone)]
pub struct Target {
    pub bin: PathBuf,
    pub loader: &'static str,
}

impl Target {
    /// Accepts a `wine/bin` folder that has a loader (original or renamed) and `lib/wine`.
    pub fn from_bin(bin: &Path) -> Option<Self> {
        if !bin.join("../lib/wine").is_dir() {
            return None;
        }
        ["wine64", "wine"]
            .into_iter()
            .find(|l| bin.join(l).exists() || bin.join(format!("{l}.real")).exists())
            .map(|loader| Self {
                bin: bin.to_path_buf(),
                loader,
            })
    }

    pub fn loader_path(&self) -> PathBuf {
        self.bin.join(self.loader)
    }

    pub fn real_path(&self) -> PathBuf {
        self.bin.join(format!("{}.real", self.loader))
    }

    pub fn preloader_src(&self) -> PathBuf {
        self.bin.join(format!("{}-preloader", self.loader))
    }

    pub fn preloader_link(&self) -> PathBuf {
        self.bin.join(format!("{}.real-preloader", self.loader))
    }

    /// The YAAGL folder name, e.g. `Yaagl OS`.
    pub fn label(&self) -> String {
        self.bin
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map_or_else(
                || self.bin.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
    }
}

pub fn support_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("YAAGL_SQUIRCLE_SUPPORT_DIR") {
        return Ok(dir.into());
    }
    let home = env::var_os("HOME")
        .ok_or_else(|| hinted("HOME is not set", "run from a normal user session"))?;
    Ok(PathBuf::from(home).join("Library/Application Support"))
}

pub fn data_dir(support: &Path) -> PathBuf {
    support.join("YaaglSquircle")
}

/// Scans `<support>/*/wine/bin`, which covers `Yaagl`, `Yaagl OS` and forks.
pub fn discover(support: &Path) -> Vec<Target> {
    let Ok(entries) = fs::read_dir(support) else {
        return Vec::new();
    };
    let mut found: Vec<Target> = entries
        .flatten()
        .filter_map(|e| Target::from_bin(&e.path().join("wine/bin")))
        .collect();
    found.sort_by(|a, b| a.bin.cmp(&b.bin));
    found
}

/// Resolves `--target`: a folder name in the support dir, or an absolute path to either
/// the YAAGL folder or its `wine/bin`.
pub fn resolve(arg: &str, support: &Path) -> Result<Target> {
    let path = Path::new(arg);
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        support.join(arg)
    };
    [base.join("wine/bin"), base.clone()]
        .iter()
        .find_map(|b| Target::from_bin(b))
        .ok_or_else(|| {
            hinted(
                format!("no Wine install found for target `{arg}`"),
                "run `yaagl-squircle status`, or pass the absolute path of the folder containing wine/bin",
            )
        })
}

pub fn select(args: &[String], support: &Path) -> Result<Vec<Target>> {
    if args.is_empty() {
        let found = discover(support);
        if found.is_empty() {
            return Err(hinted(
                format!("no YAAGL Wine install found in {}", support.display()),
                "start YAAGL once so it downloads Wine, or pass --target <path>",
            ));
        }
        return Ok(found);
    }
    args.iter().map(|a| resolve(a, support)).collect()
}
