# Desktop application

AccessMod 6 on the desktop is a window (Tauri 2, `desktop/`) around the
engine of the release archives ([`distribution.md`](distribution.md)). The
window holds no analysis code: it starts `accessmod serve` and shows the
interface the engine serves, as a browser would.

## Design

- **Sidecar process.** The window runs the `accessmod` binary shipped
  beside it: `accessmod serve --session-stdin --port 0`. An engine crash
  leaves the window up; it reports the exit status and the log path, and
  offers to restart the engine or quit. A crash of the window closes the
  engine's stdin, which stops the engine: no orphan server.
- **Session.** The window writes a random token (32 bytes) on the
  engine's stdin and reads `{"url": …}`, the free port it listens on, from
  its stdout. It then opens `/api/session/open?token=…`, which sets an
  `HttpOnly`, `SameSite=Strict` cookie; every other request needs the
  cookie (`accessmod_server::guard`). Other users of the machine and pages
  in a browser cannot use the server; the token never appears on a
  command line.
- **Loopback Host check**, also for a plain `accessmod serve`: a server on
  127.0.0.1 answers only requests naming `localhost` or a loopback
  address, so a web page cannot reach it through DNS rebinding.
- **Bundle layout.** The engine is a Tauri `externalBin`; `share/` (PROJ
  database, web client) and the licences are resources. The window passes
  `PROJ_DATA` and `--web-dir` from the resource directory.
- **Window behaviour.** Links outside the engine open in the default
  browser. Downloads (tables, job files) go to the Downloads folder under a
  free name. Files dropped on the window reach the page (import drop
  zones). The engine's stderr is appended to `engine.log` in the
  application log directory (macOS: `~/Library/Logs/org.accessmod.accessmod6/`).
- **Data.** Projects live in the same data directory as `accessmod serve`
  (`ACCESSMOD_DATA`, or `~/AccessMod`).
- `desktop/` is a Cargo workspace of its own: the engine gate
  (`scripts/check.sh`) never builds a webview, and Linux CI needs
  WebKitGTK only to package.

## Build

```sh
cargo install tauri-cli --version "^2.12" --locked   # once
sh scripts/package-desktop.sh [target]              # engine archive + installers in dist/
```

The script builds the engine archive (`scripts/package.sh`), stages its
binary as `desktop/binaries/accessmod-<target>` and its `share/` and
licences under `desktop/`, then runs `cargo tauri build`:

| Platform | Bundles |
| --- | --- |
| macOS | `.app`, `.dmg` |
| Linux | `.deb`, `.AppImage` (needs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, `libxdo-dev`, `patchelf`) |
| Windows | setup `.exe` (NSIS; MSI refuses pre-release versions such as `6.0.0-alpha.1`) |

A window over an engine already packaged (`dist/accessmod-<version>-<target>/`,
which finds its `share/` beside it), without bundling:

```sh
cd desktop && ACCESSMOD_BIN=$PWD/../dist/accessmod-<version>-<target>/accessmod cargo run
```

## Status

- macOS arm64, built and run locally (2026-10-31): the `.app` is 63 MB
  (engine 36 MB, window 7 MB, client 10 MB with its basemap, PROJ database
  9 MB); quitting, and killing the
  window process, both stop the engine.
- Linux and Windows installers: built by the release workflow, not yet
  tried by hand.
- **Unsigned.** macOS: a downloaded `.dmg` is quarantined and Gatekeeper
  refuses the app; open it with right-click › Open, or run
  `xattr -dr com.apple.quarantine /Applications/AccessMod.app`. Signing and
  notarizing the `.app` needs the Developer ID secrets the archives already
  use (`APPLE_SIGNING_IDENTITY`, `APPLE_API_KEY`, `APPLE_API_ISSUER` for the
  Tauri bundler). Windows: SmartScreen warns until a code-signing
  certificate exists.
- No in-app updates: a new version is a new installer
  (`tauri-plugin-updater` needs signed update manifests and a feed; later).

## Next

- End-to-end tests in WebKit (macOS window) and WebView2 (Windows): the
  client is tested in Chromium only.
- Native file dialogs for local paths: the window uses uploads today, as a
  browser does.
- Signed and notarized `.app` and `.dmg` once the secrets are in place.
