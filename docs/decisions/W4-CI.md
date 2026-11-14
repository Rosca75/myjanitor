# W4-3: CI workflow

`.github/workflows/ci.yml`: jobs `test` and `bundle` (needs `test`), matrix `ubuntu-22.04` and `windows-latest`, `fail-fast: false`.

- Triggers: push to `main` and `claude/**`, pull_request, workflow_dispatch. Concurrency cancels superseded runs per ref. `permissions: contents: read`; no secrets, signing or publishing.
- Toolchain: plain `rustup`, pinned 1.92.0 (workspace `rust-version` is 1.88; tauri 2.12.3 needs 1.90). Only third-party actions: `Swatinem/rust-cache@v2`; the rest are `actions/*` majors (checkout v4, setup-node v4 with Node 22, cache v4, upload-artifact v4).
- Gates: fmt, clippy `-D warnings`, `cargo test --workspace`, `node --test ui/` on both OSes.
- Bundles: `cargo tauri build --bundles deb,appimage` (Linux) and `--bundles nsis` (Windows). The bundle list is passed on the command line, so `tauri.conf.json` was NOT edited (targets stay `appimage`, `deb`). Windows also uploads the raw `target/release/myjanitor.exe`. `cargo build --release -p mj-cli` yields `myjanitor-cli`.
- Artifacts: `myjanitor-linux-x64-bundles`, `myjanitor-windows-x64-installer`, `myjanitor-windows-x64-portable`, `myjanitor-cli-linux-x64`, `myjanitor-cli-windows-x64`; all with `if-no-files-found: error`. No `cd` anywhere.
- `tauri-cli` (`^2`, `--locked`) is cached per OS and toolchain; a floating `^2` is not re-resolved on a cache hit.

Known risks: unproven first run; NSIS tooling is downloaded by tauri at build time; `--bundles nsis` on Windows relies on `icons/icon.ico` existing; AppImage needs `NO_STRIP` and `APPIMAGE_EXTRACT_AND_RUN` (set); the pinned Rust may need a bump if a dependency raises its MSRV.
