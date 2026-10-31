#!/bin/sh
# 建置不使用 macOS 私有 API 的版本（將來上架 App Store 用）。
# tauri-cli 帶 --config 建置時會改寫 Cargo.toml 裡 tauri 的 features，建完把它還原。
set -e
cd "$(dirname "$0")/../apps/desktop/src-tauri"
cp Cargo.toml Cargo.toml.orig
trap 'mv Cargo.toml.orig Cargo.toml' EXIT
cargo tauri build --config tauri.appstore.conf.json "$@"
