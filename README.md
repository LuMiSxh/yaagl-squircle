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
yaagl-squircle status    # each install: state, files and whose they are
yaagl-squircle revert    # back to stock
yaagl-squircle update    # new CLI, patches refreshed
yaagl-squircle uninstall
```

Quit the game and YAAGL first. A Wine update inside YAAGL wipes the patch; `status` then shows `wiped`, and `apply` restores it.

`apply --icon my.png` uses your own PNG instead of the clipped game icon. `--target <name|path>` selects a specific install.

`apply`, `revert` and `status` list every install with its folder and each file they touched or found, so you can see what is ours and what is not.

Some Wine builds put their own launcher script at `wine` that sets up the runtime and then runs `wine.real`. The script is never touched; instead `apply` asks before wrapping the binary behind it (`wine.real` moves to `wine.real.real`), and `status` shows that install as `applied (deep)`. Pass `--deep` to skip the question, e.g. in scripts. HSR-style installs with a plain `wine64` are patched as before.

A `.real` file that is not ours is never overwritten, not even with `--force`; `--force` only re-verifies installs that are already current.

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
