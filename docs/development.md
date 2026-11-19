# Development

## Layout

- `crates/meka-studio`: the server, a Rust binary that signs you in, serves the interface, passes requests to Meka, and runs Files, terminals, push notifications and Meka's status. Its library is what the binary runs, and what the desktop app runs inside its own process.
- `crates/meka-studio-desktop`: the desktop app, Meka Studio Desktop, on [Tauri](https://tauri.app). That name, `productName` in `tauri.conf.json` and `AppName` in its installer script, is what the system shows: the window title, tray, notifications, launchers, packages and install folders; the interface inside the window is still named Meka Studio. It runs the Meka pinned in `meka.json`, which `fetch-meka.py` downloads into `binaries/`, and the server, and shows the interface in its window. Both crates are Cargo workspace members; the workspace root is the repository root.
- `web`: the React interface, with its own `package.json`. `web/tools/schema` generates TypeScript types from Meka's OpenAPI document and from Meka Studio's.
- `config`: an example config, which goes to `~/.config/meka-studio/config.toml`; a test checks it parses. `deploy`: Meka Studio's systemd unit, as a system service (`deploy/system`) or a user service (`deploy/user`), and its macOS launch agent (`deploy/macos`).
- `docs`: user and developer documentation, and the captured Meka API in `docs/api`.

## Running it

Run a Meka on loopback, and Meka Studio with a config whose `public_origin` is the Vite dev server's address, so its cookie and `Origin` check work as in production:

```toml
listen = "127.0.0.1:4180"
public_origin = "http://localhost:5173"
state_dir = "/tmp/meka-studio-dev"

[meka]
url = "http://127.0.0.1:8080"
token_file = "/tmp/meka-studio-dev/meka-token"
```

```sh
cargo run -p meka-studio -- --config dev.toml user add me
cargo run -p meka-studio -- --config dev.toml serve
cd web
npm run dev
```

Vite passes `/api` (with WebSockets) and `/meka` to `MEKA_STUDIO_GATEWAY`, `http://127.0.0.1:4180` unless set. A debug build of Meka Studio reads `web/dist` from disk, so `npm run build` updates the interface it serves without rebuilding it. Passkeys work on `localhost` over HTTP; push notifications need HTTPS or `localhost`, and a real push service.

The [justfile](../justfile) holds the usual steps: `just build` builds the interface and then the release binary, `just build-static` a static musl one, and `just check` runs every check CI runs.

The desktop app builds on Linux with WebKitGTK 4.1 and libayatana-appindicator installed: `webkit2gtk-4.1` and `libayatana-appindicator` on Arch, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev` and `librsvg2-dev` on Debian and Ubuntu. Every workspace build needs them, `just check` included. `just desktop` builds the interface, fetches the pinned Meka, puts it beside the debug build and runs the app; it keeps its config, store and logs in the folders [Installing Meka Studio](install.md#the-desktop-app) lists. `just desktop-bundle` makes its Linux or macOS bundles with the Tauri CLI (`cargo install tauri-cli --version 2.12.5 --locked`), into `target/release/bundle`, with `LICENSE` and the third-party notices `notices.py` writes from `cargo metadata` and `web/node_modules`. Windows' installer and portable zip come from the Release workflow's steps: a release build with `--features tauri/custom-protocol`, which `cargo tauri build` would add, then Inno Setup 6.7.1 compiling `windows/meka-studio-desktop.iss`. To try a portable copy, put an empty `.portable` file beside the program; a debug build does too, and keeps its data in `target\debug\data`.

## Checks

The interface uses npm with the committed lockfile in `web/`. `web/.nvmrc` specifies Node 24; `web/.npmrc` saves exact dependency versions.

```sh
cd web
npm run check
```

This runs formatting, lint, strict TypeScript, a production build (which also writes compressed copies of its text files), and Node-based unit tests. Formatting covers `web/` and the repository's Markdown, docs, and workflows, using the root `.prettierrc.json`; `npm run format` applies it. CI also verifies that `npm run api:generate`, which needs the Rust toolchain, leaves the generated types unchanged.

The server uses the Rust toolchain named by `rust-version` in `Cargo.toml` or newer, and nightly rustfmt for comment wrapping. Run from the repository root:

```sh
cargo +nightly fmt --check
cargo sort -w --check
cargo clippy --locked --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
cargo test --locked
```

Its tests run the API in process against a stand-in Meka, terminals against `/bin/sh`, or `cmd.exe` on Windows, and Web Push encryption against RFC 8291's example. The desktop app's tests run its Meka supervisor against a shell script standing in for Meka, on Linux and macOS. The server runs on Linux, macOS and Windows; what differs is kept in `files/folder` and `terminal/pty`, and CI runs clippy and the tests on macOS and Windows too. From Linux, `cargo clippy --target x86_64-pc-windows-gnu` checks Windows with mingw-w64 installed, and wine can run its tests, though its console may not start a terminal's shell.

Review the desktop app by running it with `XDG_CONFIG_HOME` and `XDG_DATA_HOME` pointing at a scratch folder on Linux, under Xvfb where there is no display. Review browser behavior and live workflows manually against a disposable Meka, through Meka Studio: signing in with a password and a passkey; messages, approvals, and recovery when Meka or Meka Studio restarts; Files and the terminal in a session, and Meka's status in Settings; rich content, keyboard access, and narrow layouts. Chromium's virtual authenticator, through the DevTools protocol's `WebAuthn` domain, stands in for a passkey.

The [architecture](architecture.md) explains component boundaries and state invariants. [Schema tooling](api/README.md) describes API type generation. Keep the architecture and [API support reference](api-coverage.md) aligned with behavior changes, and keep credentials and local review artifacts out of version control.

## Releases

Meka Studio has its own version, independent of Meka's. `web/package.json` is the source of the version the interface shows; `web/package-lock.json` and the workspace version in `Cargo.toml` record the same version, which the desktop app takes too. Each release states which Meka versions it supports, in `web/src/api/meka-versions.json`. The desktop app comes with the Meka `crates/meka-studio-desktop/meka.json` pins, by version and the SHA-256 of each release file; `fetch-meka.py` refuses a version that `meka-versions.json` does not list as verified. When a release supports a newer Meka, pin it there too.

Record user-visible changes under `Unreleased` in [CHANGELOG.md](../CHANGELOG.md), grouped by Added, Changed, Deprecated, Removed, Fixed, or Security. Use one line per entry, at most 100 characters including `- `, and mark breaking changes with `**Breaking:**`. An entry says what changed, not how to upgrade; upgrade steps go in the GitHub release's notes.

To release, run `npm version <version> --no-git-tag-version` in `web/`, set the same version in `Cargo.toml` and build once to update `Cargo.lock`, move the unreleased notes into a dated version section, and update the changelog's comparison links. Run the checks and commit before tagging. Tags use bare `major.minor.patch` numbers: no `v` prefix or prerelease suffix.

Pushing a tag runs the **Release** workflow. It checks the tag against the three versions, builds the interface, builds static musl binaries for x86-64 and ARM64 Linux on runners of each, an Apple silicon binary, and an x86-64 Windows one, and attaches them as `meka-studio-x86_64-linux`, `meka-studio-aarch64-linux`, `meka-studio-aarch64-macos` and `meka-studio-x86_64-windows.exe` to a draft GitHub release titled with the version, whose notes are the version's changelog section. It bundles the desktop app with its Meka, `LICENSE` and third-party notices for the platforms Meka releases for, unsigned, as `meka-studio-desktop-x86_64-linux.deb` and `.AppImage`, `meka-studio-desktop-aarch64-macos.dmg`, and `meka-studio-desktop-x86_64-windows-setup.exe` and `meka-studio-desktop-x86_64-windows-portable.zip`, on the same release. The Windows job installs Inno Setup 6.7.1 from its GitHub release, checked against its SHA-256. Review the draft's notes and files, add the steps any breaking change needs to its notes, then publish it on GitHub; until then the release is visible only to the repository's maintainers.

web.meka.run, the static site of earlier releases, is retired. GitHub Pages keeps serving its last deployment, Meka Studio 0.23.1, until the site is unpublished; nothing in this repository deploys it anymore.
