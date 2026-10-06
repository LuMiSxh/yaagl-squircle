// SPDX-License-Identifier: MPL-2.0

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/GameIconBridge.m");
    println!("cargo:rerun-if-env-changed=YAAGL_SQUIRCLE_PREBUILT_BRIDGE");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("GameIconBridge.dylib");

    if let Ok(prebuilt) = env::var("YAAGL_SQUIRCLE_PREBUILT_BRIDGE") {
        fs::copy(prebuilt, &out).expect("cannot copy the prebuilt bridge");
        return;
    }
    // cfg!(target_os) in a build script describes the HOST. Linux and Windows CI only
    // run check, clippy and tests, so an empty stub is enough there.
    if !cfg!(target_os = "macos") {
        fs::write(&out, b"").unwrap();
        println!("cargo:warning=bridge not built (needs a macOS host); embedding an empty stub");
        return;
    }

    let status = Command::new("xcrun")
        .args([
            "--sdk", "macosx", "clang", "-arch", "x86_64", "-arch", "arm64",
        ])
        .args(["-mmacosx-version-min=11.0", "-O2", "-dynamiclib", "-lobjc"])
        .arg("native/GameIconBridge.m")
        .arg("-o")
        .arg(&out)
        .status()
        .expect("xcrun not found; install the Xcode Command Line Tools");
    assert!(
        status.success(),
        "clang failed to build GameIconBridge.dylib"
    );

    let archs = Command::new("lipo")
        .arg("-archs")
        .arg(&out)
        .output()
        .expect("lipo failed");
    let archs = String::from_utf8_lossy(&archs.stdout);
    assert!(
        archs.contains("x86_64") && archs.contains("arm64"),
        "bridge is not universal: {archs}"
    );
}
