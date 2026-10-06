// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::util::write_atomic;

#[derive(Serialize, Deserialize, Default)]
pub struct Manifest {
    pub cli_version: String,
    pub bridge_sha256: String,
    pub icon: Option<String>,
    pub targets: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
pub struct Entry {
    pub dir: PathBuf,
    pub loader: String,
    pub real: String,
    pub wrapper_sha256: String,
    pub applied_at: String,
}

impl Manifest {
    fn path(data: &Path) -> PathBuf {
        data.join("manifest.json")
    }

    pub fn load(data: &Path) -> Result<Self> {
        match fs::read(Self::path(data)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("manifest.json is corrupt"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("cannot read manifest.json"),
        }
    }

    pub fn save(&self, data: &Path) -> Result<()> {
        fs::create_dir_all(data)?;
        write_atomic(&Self::path(data), &serde_json::to_vec_pretty(self)?, 0o644)
    }

    pub fn entry(&self, dir: &Path) -> Option<&Entry> {
        self.targets.iter().find(|e| e.dir == dir)
    }

    pub fn upsert(&mut self, entry: Entry) {
        self.targets.retain(|e| e.dir != entry.dir);
        self.targets.push(entry);
    }

    pub fn remove(&mut self, dir: &Path) {
        self.targets.retain(|e| e.dir != dir);
    }
}
