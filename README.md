# AntListener 2026

Tauri rewrite of the original WinForms AntListener.

## Implemented

- Tauri 2 desktop shell with React + TypeScript frontend.
- Rust backend TCP listener on `0.0.0.0:{local.port}`.
- System tray menu: show main window, start listener, stop listener, config, quit.
- Single-instance guard.
- `config.toml` compatible with the old program.
- Device list loading from `GET /device`.
- Incoming data parsing compatible with the old socket code:
  - 10-byte packet beginning with `0x00` becomes a 10-digit decimal number from bytes 6-9.
  - 5-byte packet becomes lowercase hex.
  - Other packets become uppercase hex.
- Incoming command queue and patient binding dialog.
- Last washing record lookup from `GET /lastrecordbyeid/{enumber}`.
- Patient list lookup from `POST /getPatientNameList`.
- Chinese name filtering and pinyin initial filtering.
- Bind/unbind through `POST /writeback2`.
- Manual device-number read.

## Development

```bash
npm install
npm run dev
```

## Build

```bash
npm run build
```

The current macOS build target is `.app` only:

```text
src-tauri/target/release/bundle/macos/AntListener 2026.app
```

The DMG bundle was intentionally not enabled as the default target because the local DMG helper failed after the app itself had already built successfully. Enable DMG later when preparing installer packaging.

## Windows build on GitHub Actions

This repository includes a GitHub Actions workflow:

```text
.github/workflows/windows-build.yml
```

After pushing the project to a public GitHub repository, Windows builds run on `windows-latest` when:

- pushing to `main` or `master`
- pushing a `v*` tag
- manually clicking `Run workflow`

The workflow uploads a build artifact named:

```text
AntListener-2026-Windows
```

Expected deployment files should be placed next to the generated Windows executable:

```text
AntListener 2026.exe
config.toml
ding.wav
```

`config.toml` and `ding.wav` are intentionally ignored by git for public repositories.
Use `config.example.toml` as the template for deployment.

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

6. Unzip the artifact. The current direct executable is:

```text
ant-listener-2026.exe
```

7. Put runtime files in the same directory:

```text
ant-listener-2026.exe
config.toml
ding.wav
```

8. Run `ant-listener-2026.exe` on Windows.

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
3. System app config directory.

`ding.wav` is loaded from the same directory as the resolved `config.toml`.

Recommended deployment layout:

macOS:

```text
AntListener 2026.app
config.toml
ding.wav
```

Windows:

```text
AntListener 2026.exe
config.toml
ding.wav
```

## Field test checklist

1. Start the app.
2. Confirm the tray icon appears.
3. Confirm listener starts on the configured port.
4. Send a real device packet to the port.
5. Confirm the binding dialog opens.
6. Confirm `lastrecordbyeid`, `getPatientNameList`, and `writeback2` work against the production data service.
7. Confirm closing the main window hides it instead of exiting.
8. Confirm tray Quit exits completely.
