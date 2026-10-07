# Changelog

## 0.2.0

- Wine builds with their own launcher script at `wine` (in front of `wine.real`) are patched
  one level deeper, after asking: the script stays untouched and `wine.real` is wrapped.
  `--deep` answers yes non-interactively; `status` shows `applied (deep)`.
- A `.real` file that is not ours is never overwritten; `--force` no longer does that.
- `apply`, `revert` and `status` print each install with its folder, the files found and
  every step taken. `status` state `ok` is now `applied`.

## 0.1.0

- Initial release: `apply`, `status`, `revert`, `debug`, `update`, `uninstall`.
- Only YAAGL data folders are patched (recognised by `resources.neu`).
- `debug on|off` writes the bridge's activity to `bridge.log` in the data folder.
- Clips the Dock icon of Wine games started by YAAGL to a squircle by injecting a small
  bridge into Wine's loader. Verified by a smoke test after every apply, with automatic rollback.
