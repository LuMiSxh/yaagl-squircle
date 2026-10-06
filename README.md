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

`apply --icon my.png` uses your own PNG instead of the clipped game icon. `--target <name|path>` selects a specific install.

Only YAAGL installs are touched: a folder counts only if it has `wine/bin`, `wine/lib/wine` and YAAGL's `resources.neu`. Wine installs of other launchers are ignored, even with `--target`.

## Something does not work

```sh
yaagl-squircle debug on     # then start the game once, then quit it
yaagl-squircle debug        # shows the log's last lines; the file is in the data folder
yaagl-squircle debug off
```

Include the output of `yaagl-squircle status --verbose` and `yaagl-squircle debug` in an issue. If `apply` fails, it restores the original loader and prints a `hint:` line; include that too.

## Build

macOS with the Xcode Command Line Tools: `cargo build --release`. Linux can run `cargo check` and `cargo test` with an empty bridge stub; Windows is not supported.

MPL-2.0.
