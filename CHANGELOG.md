# Changelog

## 0.1.0

- Initial release: `apply`, `status`, `revert`, `debug`, `update`, `uninstall`.
- Only YAAGL data folders are patched (recognised by `resources.neu`).
- `debug on|off` writes the bridge's activity to `bridge.log` in the data folder.
- Clips the Dock icon of Wine games started by YAAGL to a squircle by injecting a small
  bridge into Wine's loader. Verified by a smoke test after every apply, with automatic rollback.
