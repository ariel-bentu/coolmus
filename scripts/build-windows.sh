#!/bin/zsh
# Cross-compiles Coolmus for Windows x64 from macOS.
# One-time setup:
#   rustup target add x86_64-pc-windows-msvc
#   brew install llvm lld makensis
#   cargo install cargo-xwin tauri-cli --locked
# Output:
#   target/x86_64-pc-windows-msvc/release/coolmus.exe                         (portable)
#   target/x86_64-pc-windows-msvc/release/bundle/nsis/Coolmus_*_x64-setup.exe (installer)
set -e
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:/opt/homebrew/opt/lld/bin:$PATH"
# The update files are signed with the key made by `cargo tauri signer generate`
# (see README, Releasing); the same key is the TAURI_SIGNING_PRIVATE_KEY secret.
KEY="$HOME/.tauri/coolmus.key"
[[ -f "$KEY" ]] || { echo "No signing key at $KEY: see README, Releasing" >&2; exit 1; }
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
cd "$(dirname "$0")/../app"
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
