// SPDX-License-Identifier: MPL-2.0

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{util::hinted, wrapper};

/// An installed Wine `bin` folder inside a YAAGL data directory.
#[derive(Clone)]
pub struct Target {
    pub bin: PathBuf,
    pub loader: &'static str,
    /// A foreign launcher script in front of `loader` (a "deep" patch), e.g. `wine`.
    pub launcher: Option<&'static str>,
}

impl Target {
    /// Accepts a `wine/bin` folder that has a loader (original or renamed) and `lib/wine`,
    /// and only inside a YAAGL data folder. `resources.neu` is the Neutralino bundle that
    /// YAAGL's launcher copies there; Wine installs of other launchers lack it and are
    /// never touched.
    pub fn from_bin(bin: &Path) -> Option<Self> {
        if !bin.join("../lib/wine").is_dir() || !bin.join("../../resources.neu").is_file() {
            return None;
        }
        let name = ["wine64", "wine"]
            .into_iter()
            .find(|l| bin.join(l).exists() || bin.join(format!("{l}.real")).exists())?;
        // Some builds ship their own launcher script at `wine` that sets up the runtime,
        // unsets DYLD_INSERT_LIBRARIES and execs `wine.real`. Wrapping that script would
        // lose the bridge, so the binary behind it is the loader we wrap instead.
        let inner = bin.join(format!("{name}.real"));
        let deep = is_foreign_script(&bin.join(name))
            && (is_macho(&inner)
                || wrapper::is_wrapper(&inner)
                || bin.join(format!("{name}.real.real")).exists());
        let (loader, launcher) = match (deep, name) {
            (false, _) => (name, None),
            (true, "wine64") => ("wine64.real", Some(name)),
            (true, _) => ("wine.real", Some(name)),
        };
        Some(Self {
            bin: bin.to_path_buf(),
            loader,
            launcher,
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

    /// The relevant files in `wine/bin` with their role, for status output.
    pub fn files(&self) -> Vec<(&'static str, PathBuf)> {
        let mut files = Vec::new();
        if let Some(l) = self.launcher {
            files.push(("launcher", self.bin.join(l)));
        }
        files.push(("loader", self.loader_path()));
        // A missing original only matters once our wrapper depends on it.
        if self.real_path().exists() || wrapper::is_wrapper(&self.loader_path()) {
            files.push(("original", self.real_path()));
        }
        for (role, p) in [
            ("preloader", self.preloader_src()),
            ("link", self.preloader_link()),
        ] {
            if p.symlink_metadata().is_ok() {
                files.push((role, p));
            }
        }
        files
    }

    /// "deep" when the patch sits behind a foreign launcher script.
    pub fn layout(&self) -> String {
        match self.launcher {
            Some(l) => format!(
                "deep: launcher script `{l}` (not ours) -> `{}`",
                self.loader
            ),
            None => format!("loader `{}`", self.loader),
        }
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

fn head4(path: &Path) -> [u8; 4] {
    let mut head = [0u8; 4];
    let _ = fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head));
    head
}

/// A shell script that is not our wrapper, i.e. a launcher script shipped with Wine.
fn is_foreign_script(path: &Path) -> bool {
    head4(path).starts_with(b"#!") && !wrapper::is_wrapper(path)
}

fn is_macho(path: &Path) -> bool {
    matches!(
        head4(path),
        [0xcf, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe]
    )
}

/// What a file in `wine/bin` is, for showing the user what is installed.
pub fn describe(path: &Path) -> String {
    match fs::symlink_metadata(path) {
        Err(_) => "missing".into(),
        Ok(m) if m.is_symlink() => match fs::read_link(path) {
            Ok(dest) => format!("symlink -> {}", dest.display()),
            Err(_) => "symlink".into(),
        },
        Ok(_) if wrapper::is_wrapper(path) => "our wrapper".into(),
        Ok(_) if is_macho(path) => "Wine binary (Mach-O)".into(),
        Ok(_) if is_foreign_script(path) => "launcher script (not ours)".into(),
        Ok(_) => "unknown file (not ours)".into(),
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
