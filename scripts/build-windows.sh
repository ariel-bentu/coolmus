#!/bin/zsh
# Cross-compiles Sketcher for Windows x64 from macOS.
# One-time setup:
#   rustup target add x86_64-pc-windows-msvc
#   brew install llvm lld makensis
#   cargo install cargo-xwin tauri-cli --locked
# Output:
#   target/x86_64-pc-windows-msvc/release/sketcher.exe                         (portable)
#   target/x86_64-pc-windows-msvc/release/bundle/nsis/Sketcher_*_x64-setup.exe (installer)
set -e
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:/opt/homebrew/opt/lld/bin:$PATH"
cd "$(dirname "$0")/../app"
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
