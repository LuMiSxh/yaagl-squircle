// SPDX-License-Identifier: MPL-2.0

// The apply/revert tests only run on macOS, which leaves their helpers unused elsewhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

/// A shell script loader. dyld strips DYLD_* variables when it runs the SIP-protected
/// /bin/sh, so the bridge can never load into it, which makes it the "blocked" case.
const SCRIPT_WINE: &str = "#!/bin/sh\necho wine-test\n";
const BINARY_WINE_SRC: &str =
    "#include <stdio.h>\nint main(void) { puts(\"wine-test\"); return 0; }\n";

enum Loader {
    /// A compiled Mach-O named like Wine's loader, so the real bridge loads into it.
    Binary,
    Script,
}

struct Env {
    dir: TempDir,
}

impl Env {
    fn new(loader: Loader) -> Self {
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("Yaagl/wine/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(dir.path().join("Yaagl/wine/lib/wine")).unwrap();
        fs::write(dir.path().join("Yaagl/resources.neu"), "").unwrap();
        match loader {
            Loader::Script => {
                fs::write(bin.join("wine64"), SCRIPT_WINE).unwrap();
                fs::set_permissions(bin.join("wine64"), fs::Permissions::from_mode(0o755)).unwrap();
            }
            Loader::Binary => {
                let src = dir.path().join("wine.c");
                fs::write(&src, BINARY_WINE_SRC).unwrap();
                let status = Command::new("xcrun")
                    .args(["--sdk", "macosx", "clang"])
                    .arg(&src)
                    .arg("-o")
                    .arg(bin.join("wine64"))
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        }
        fs::write(bin.join("wine64-preloader"), "preloader").unwrap();
        Self { dir }
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("Yaagl/wine/bin")
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("YaaglSquircle")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yaagl-squircle"))
            .args(args)
            .env("YAAGL_SQUIRCLE_SUPPORT_DIR", self.dir.path())
            .env("YAAGL_SQUIRCLE_NO_PGREP", "1")
            .output()
            .unwrap()
    }
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn read(p: &Path) -> Vec<u8> {
    fs::read(p).unwrap()
}

#[test]
fn status_without_install_exits_1() {
    let dir = TempDir::new().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_yaagl-squircle"))
        .arg("status")
        .env("YAAGL_SQUIRCLE_SUPPORT_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(code(&o), 1);
}

#[test]
fn status_reports_not_patched_and_json() {
    let env = Env::new(Loader::Script);
    let o = env.run(&["status"]);
    assert_eq!(code(&o), 1);
    assert!(stdout(&o).contains("not patched"));
    let j = env.run(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&j.stdout).unwrap();
    assert_eq!(v["targets"][0]["state"], "not patched");
}

#[test]
fn wine_folders_without_yaagl_marker_are_never_touched() {
    let env = Env::new(Loader::Script);
    let other = env.dir.path().join("OtherLauncher/wine/bin");
    fs::create_dir_all(&other).unwrap();
    fs::create_dir_all(env.dir.path().join("OtherLauncher/wine/lib/wine")).unwrap();
    fs::write(other.join("wine64"), SCRIPT_WINE).unwrap();

    let status = env.run(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(v["targets"].as_array().unwrap().len(), 1);

    let o = env.run(&["apply", "--dry-run", "--target", "OtherLauncher"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("hint:"));
    assert_eq!(read(&other.join("wine64")), SCRIPT_WINE.as_bytes());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn apply_without_bridge_explains_itself() {
    let env = Env::new(Loader::Script);
    let o = env.run(&["apply"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("hint:"));
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    #[test]
    fn debug_toggle_rewrites_the_wrapper_and_the_bridge_logs() {
        let env = Env::new(Loader::Binary);
        assert_eq!(code(&env.run(&["apply"])), 0);
        let wrapper = || String::from_utf8(read(&env.bin().join("wine64"))).unwrap();
        assert!(!wrapper().contains("YAAGL_SQUIRCLE_DEBUG"));

        let on = env.run(&["debug", "on"]);
        assert_eq!(code(&on), 0, "{}", stderr(&on));
        assert!(wrapper().contains("YAAGL_SQUIRCLE_LOG"));
        assert_eq!(code(&env.run(&["debug", "on"])), 1);
        assert_eq!(code(&env.run(&["status"])), 0);

        let run = Command::new(env.bin().join("wine64"))
            .arg("--version")
            .output()
            .unwrap();
        assert!(run.status.success());
        let log = fs::read_to_string(env.data().join("bridge.log")).unwrap();
        assert!(log.contains("bridge loaded in wine64.real"), "{log}");
        let shown = stdout(&env.run(&["debug"]));
        assert!(
            shown.contains("debug: on") && shown.contains("bridge loaded"),
            "{shown}"
        );

        let off = env.run(&["debug", "off"]);
        assert_eq!(code(&off), 0, "{}", stderr(&off));
        assert!(!wrapper().contains("YAAGL_SQUIRCLE_DEBUG"));
        assert_eq!(code(&env.run(&["debug", "off"])), 1);
    }

    #[test]
    fn dry_run_changes_nothing() {
        let env = Env::new(Loader::Script);
        let o = env.run(&["apply", "--dry-run"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert!(stdout(&o).contains("would patch"));
        assert!(!env.bin().join("wine64.real").exists());
        assert!(!env.data().exists());
    }

    #[test]
    fn apply_is_idempotent_and_revert_is_byte_identical() {
        let env = Env::new(Loader::Binary);
        let original = read(&env.bin().join("wine64"));

        let first = env.run(&["apply"]);
        assert_eq!(code(&first), 0, "{}", stderr(&first));
        assert_eq!(read(&env.bin().join("wine64.real")), original);
        assert!(
            env.bin()
                .join("wine64.real-preloader")
                .symlink_metadata()
                .unwrap()
                .is_symlink()
        );
        assert!(env.data().join("GameIconBridge.dylib").exists());
        assert!(env.data().join("manifest.json").exists());
        assert_eq!(code(&env.run(&["status"])), 0);

        let second = env.run(&["apply"]);
        assert_eq!(code(&second), 1, "{}", stderr(&second));

        let revert = env.run(&["revert"]);
        assert_eq!(code(&revert), 0, "{}", stderr(&revert));
        assert_eq!(read(&env.bin().join("wine64")), original);
        assert!(!env.bin().join("wine64.real").exists());
        assert!(
            env.bin()
                .join("wine64.real-preloader")
                .symlink_metadata()
                .is_err()
        );
        assert_eq!(code(&env.run(&["revert"])), 1);
    }

    #[test]
    fn status_detects_wiped_wine() {
        let env = Env::new(Loader::Binary);
        let original = read(&env.bin().join("wine64"));
        assert_eq!(code(&env.run(&["apply"])), 0);
        // YAAGL's Wine update replaces the whole folder with a pristine loader.
        fs::remove_file(env.bin().join("wine64")).unwrap();
        fs::remove_file(env.bin().join("wine64.real")).unwrap();
        fs::remove_file(env.bin().join("wine64.real-preloader")).unwrap();
        fs::write(env.bin().join("wine64"), original).unwrap();
        fs::set_permissions(env.bin().join("wine64"), fs::Permissions::from_mode(0o755)).unwrap();

        let o = env.run(&["status"]);
        assert_eq!(code(&o), 1);
        assert!(stdout(&o).contains("wiped"), "{}", stdout(&o));
        assert_eq!(code(&env.run(&["apply"])), 0);
        assert_eq!(code(&env.run(&["status"])), 0);
    }

    #[test]
    fn apply_rolls_back_when_the_bridge_does_not_load() {
        let env = Env::new(Loader::Script);
        let original = read(&env.bin().join("wine64"));
        let o = env.run(&["apply"]);
        assert_eq!(code(&o), 2);
        assert!(stderr(&o).contains("hint:"));
        assert_eq!(read(&env.bin().join("wine64")), original);
        assert!(!env.bin().join("wine64.real").exists());
        assert!(
            env.bin()
                .join("wine64.real-preloader")
                .symlink_metadata()
                .is_err()
        );
        assert!(!env.data().join("manifest.json").exists());
    }

    #[test]
    fn apply_refuses_foreign_real_file_without_force() {
        let env = Env::new(Loader::Script);
        fs::write(env.bin().join("wine64.real"), "foreign").unwrap();
        let o = env.run(&["apply"]);
        assert_eq!(code(&o), 2);
        assert!(stderr(&o).contains("--force"));
        assert_eq!(read(&env.bin().join("wine64.real")), b"foreign");
    }

    #[test]
    fn custom_icon_is_copied_and_exported_by_the_wrapper() {
        let env = Env::new(Loader::Binary);
        let icon = env.dir.path().join("my.png");
        fs::write(&icon, "png").unwrap();
        let o = env.run(&["apply", "--icon", icon.to_str().unwrap()]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let wrapper = String::from_utf8(read(&env.bin().join("wine64"))).unwrap();
        assert!(wrapper.contains("YAAGL_SQUIRCLE_ICON"));
        assert_eq!(read(&env.data().join("icon.png")), b"png");
    }

    #[test]
    fn uninstall_removes_everything_but_the_binary_with_keep_binary() {
        let env = Env::new(Loader::Binary);
        let original = read(&env.bin().join("wine64"));
        assert_eq!(code(&env.run(&["apply"])), 0);
        let o = env.run(&["uninstall", "--yes", "--keep-binary"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert!(!env.data().exists());
        assert_eq!(read(&env.bin().join("wine64")), original);
        assert!(!env.bin().join("wine64.real").exists());
    }
}
