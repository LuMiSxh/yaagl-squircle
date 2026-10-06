# yaagl-squircle

Gives Wine games started by [YAAGL](https://github.com/yaagl/yet-another-anime-game-launcher) a squircle Dock icon on macOS.

It replaces Wine's loader inside YAAGL's data folder with a small wrapper that injects a bridge dylib. The bridge clips the game's icon to a rounded square. `YAAGL.app` itself is never touched.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/LuMiSxh/yaagl-squircle/main/install.sh | sh
```

## Use

```sh
yaagl-squircle apply     # patch, verify, roll back on failure
yaagl-squircle status    # ok / not patched / drifted / wiped / orphan
yaagl-squircle revert    # back to stock
yaagl-squircle update    # new CLI, patches refreshed
yaagl-squircle uninstall
```

Quit the game and YAAGL first. A Wine update inside YAAGL wipes the patch; `status` then shows `wiped`, and `apply` restores it.

`apply --icon my.png` uses your own PNG instead of the clipped game icon. `--target <name|path>` selects a specific install; by default every `~/Library/Application Support/*/wine/bin` is patched.

## Something does not work

Run `yaagl-squircle status --verbose` and include its output in an issue. If `apply` fails, it restores the original loader and prints a `hint:` line; include that too.

## Build

macOS with the Xcode Command Line Tools: `cargo build --release`. Other hosts can run `cargo check` and `cargo test` with an empty bridge stub.

MPL-2.0.
