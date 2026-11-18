# Meka Studio build helpers. The interface is compiled into the binary, so every build builds web/
# first.

# The suffix of an executable here.
exe := if os() == "windows" { ".exe" } else { "" }

# List recipes.
default:
    @just --list

# Build the interface into web/dist.
web:
    cd web && npm ci && npm run build

# Build the release binary, with the interface, into target/release/meka-studio.
build: web
    cargo build --release --locked -p meka-studio

# Needs musl-tools (musl-gcc) for the bundled OpenSSL and SQLite, and the target:
# rustup target add x86_64-unknown-linux-musl
# Build a static binary for any Linux, into target/x86_64-unknown-linux-musl/release/meka-studio.
build-static: web
    cargo build --release --locked --target x86_64-unknown-linux-musl -p meka-studio

# Fetch the Meka the desktop app is built with into crates/meka-studio-desktop/binaries.
desktop-meka:
    python3 crates/meka-studio-desktop/fetch-meka.py "$(rustc -vV | sed -n 's/^host: //p')"

# Run the desktop app from source, with the Meka it is built with beside it.
desktop: web desktop-meka
    cargo build --locked -p meka-studio-desktop
    cp crates/meka-studio-desktop/binaries/meka{{ exe }} target/debug/
    ./target/debug/meka-studio-desktop

# Needs the Tauri CLI: cargo install tauri-cli --version 2.12.5 --locked
# Bundle the desktop app for Linux or macOS, with its Meka, license and third-party notices, into
# target/release/bundle. Windows' installer and portable zip are made as the Release workflow does.
desktop-bundle: web desktop-meka
    python3 crates/meka-studio-desktop/notices.py "$(rustc -vV | sed -n 's/^host: //p')" crates/meka-studio-desktop/binaries/THIRD-PARTY-NOTICES.txt
    cd crates/meka-studio-desktop && cargo tauri build --config '{"bundle":{"resources":{"binaries/meka":"meka","../../LICENSE":"LICENSE","binaries/THIRD-PARTY-NOTICES.txt":"THIRD-PARTY-NOTICES.txt"}}}'

# Run every check CI runs.
check:
    cargo +nightly fmt --check
    cargo sort -w --check
    cargo clippy --locked --all-targets -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
    cargo test --locked
    cargo +1.95 check --locked --all-targets
    cd web && npm ci && npm run api:generate && git diff --exit-code -- src/api && npm run check
