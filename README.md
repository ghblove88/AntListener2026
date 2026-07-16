# AntListener 2026

Tauri rewrite of the original WinForms AntListener.

## Implemented

- Tauri 2 desktop shell with React + TypeScript frontend.
- Rust backend TCP listener on `0.0.0.0:{local.port}`.
- System tray menu: show main window, start listener, stop listener, config, quit.
- Single-instance guard.
- `config.toml` preserves the old URL, autorun, port, and identifier keys; unused username/password keys were removed.
- Device list loading from `GET /device`.
- Incoming data parsing compatible with the old socket code:
  - 10-byte packet beginning with `0x00` becomes a 10-digit decimal number from bytes 6-9.
  - 5-byte packet becomes lowercase hex.
  - TCP fragmentation and coalescing are handled with a streaming packet buffer.
- Listener status reports the real bound port, active connection count, and latest runtime error.
- Optional reader IP allowlist and a 32-connection safety limit.
- Shared HTTP client with connect/request timeouts and HTTP status validation.
- Incoming command queue and patient binding dialog.
- Last washing record lookup from `GET /lastrecordbyeid/{enumber}`.
- Patient list lookup from `POST /getPatientNameList`.
- Chinese name filtering and pinyin initial filtering.
- Bind/unbind through `POST /writeback2`.
- Manual device-number read.
- The latest 200 local binding records survive restarts and can be cleared from the UI.
- Runtime errors are written to `ant-listener.log` in the operating system app log directory.

## Development

```bash
npm install
npm run dev
```

Run the complete local validation suite with:

```bash
npm run check
```

The repository can be checked out in any directory; source files and runtime
configuration do not depend on an absolute checkout path. After moving an
existing working tree, clear Cargo's generated dependency metadata before the
next build because it records the previous absolute path:

```bash
cargo clean --manifest-path src-tauri/Cargo.toml
```

## Build

```bash
npm run build
```

The macOS build is only a local development smoke test:

```text
src-tauri/target/release/bundle/macos/AntListener 2026.app
```

The application is deployed only to Windows. macOS signing, notarization, DMG packaging, and Intel/universal builds are intentionally out of scope.

## Windows build on GitHub Actions

This repository includes a GitHub Actions workflow:

```text
.github/workflows/windows-build.yml
```

After pushing the project to a public GitHub repository, Windows builds run on `windows-latest` when:

- pushing to `main` or `master`
- pushing a `v*` tag
- manually clicking `Run workflow`

The workflow typechecks the frontend, runs Rust tests and Clippy, builds NSIS, and uploads an artifact named:

```text
AntListener-2026-Windows
```

The artifact contains both an NSIS installer and a ready-to-edit portable ZIP:

```text
NSIS installer executable
AntListener-2026-Portable.zip
```

The portable ZIP already contains `AntListener-2026.exe`, `config.toml`, and this README. The NSIS build creates a default config in the user app-config directory on first launch.

The generated Windows files are currently unsigned. A hospital-trusted or commercial code-signing certificate is required to remove Windows SmartScreen warnings; CI cannot complete that step until the certificate and password are provided as secrets.

### GitHub Actions build steps

1. Push code to the public GitHub repository:

```bash
git push
```

2. Open the Actions page:

```text
https://github.com/ghblove88/AntListener2026/actions
```

3. Select `Build Windows`.

4. Either wait for the automatic `push` build, or click `Run workflow`.

5. After the workflow succeeds, download the artifact:

```text
AntListener-2026-Windows
```

6. Unzip the artifact, then choose either the NSIS installer or `AntListener-2026-Portable.zip`.

7. For portable deployment, edit `config.toml` and run:

```text
AntListener-2026.exe
```

Windows may ask for firewall permission the first time the app listens on port `9000`; allow it for the network used by the endoscope reader.

### Local Windows build

Use this only on a Windows machine or Windows VM.

Required tools:

- Node.js
- Rust stable
- Microsoft Visual Studio Build Tools with C++ desktop components
- WebView2 Runtime

Commands:

```powershell
npm install
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run tauri -- build --bundles nsis
```

Build outputs:

```text
src-tauri\target\release\ant-listener-2026.exe
src-tauri\target\release\bundle\nsis\
```

The GitHub Actions workflow uses the same build path on `windows-latest`, so it is the preferred method for producing the Windows artifact.

## Important files

- `src-tauri/src/lib.rs`: TCP listener, tray, config, API bridge, Tauri commands.
- `src/App.tsx`: main UI, record table, config modal, binding modal, incoming queue.
- `src/styles.css`: desktop UI styling.
- `config.toml`: current runtime config copied from the old project.

## Runtime files

The app resolves `config.toml` in this order:

1. macOS: the directory next to `AntListener 2026.app`; Windows: the `.exe` directory.
2. Current working directory.
3. System app config directory. A validated default file is created here when no external config exists.

Custom sound files are optional and loaded from the same directory as the resolved `config.toml`:

- `ding.wav`: card data received.
- `bdcg.wav`: bind succeeded.
- `bdjc.wav`: unbind succeeded.

When a custom file is absent, the app plays a built-in tone, so missing WAV files no longer make notifications silently disappear.

Recommended portable Windows layout:

```text
AntListener-2026.exe
config.toml
ding.wav  (optional)
bdcg.wav  (optional)
bdjc.wav  (optional)
```

`config.toml` no longer contains username or password fields. `local.allowed_ips` can be left empty for compatibility or set to the reader IP addresses for a restricted deployment.

## Field test checklist

1. Start the app.
2. Confirm the tray icon appears.
3. Confirm listener starts on the configured port and reports the real connection count.
4. Occupy the configured port with another process and confirm the app reports a start failure instead of showing “running”.
5. Send fragmented and back-to-back real device packets and confirm each card produces exactly one dialog.
6. Stop the listener while a reader remains connected and confirm later packets are ignored.
7. Confirm `lastrecordbyeid`, `getPatientNameList`, and `writeback2` work against the production data service.
8. Confirm the binding flow still works when the patient list endpoint is unavailable by manually entering a name.
9. Confirm local records survive an application restart and can be cleared.
10. Confirm closing the main window hides it instead of exiting and tray Quit exits completely.
