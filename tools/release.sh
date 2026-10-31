#!/bin/bash
# 在本機（Apple Silicon Mac）打包並發佈版本，規則見 spec/20261101-release.md。
#
#   tools/release.sh build              檢查、跑測試、打包四個平台到 dist/v<版本>/
#   tools/release.sh publish <說明檔>    推 tag、建立 GitHub Release、上傳、版本號加一並推送
set -euo pipefail
cd "$(dirname "$0")/.."

REPO=supacode-org/supalive
TARGETS_MAC="aarch64-apple-darwin x86_64-apple-darwin"
TARGETS_WIN="x86_64-pc-windows-msvc aarch64-pc-windows-msvc"

die() { echo "錯誤：$*" >&2; exit 1; }

# [workspace.package] 底下的第一個 version
version() { awk '/^\[workspace.package\]/{p=1;next} /^\[/{p=0} p&&/^version *=/{gsub(/[" ]/,"",$0);split($0,a,"=");print a[2];exit}' Cargo.toml; }

VER=$(version)
TAG="v$VER"
DIST="dist/$TAG"
[ -n "$VER" ] || die "讀不到 Cargo.toml 的版本號"

# 發佈用的檔名，build 產生、publish 檢查
artifacts() {
  echo "SupaLive_${VER}_aarch64.dmg SupaLive_${VER}_x64.dmg SupaLive_${VER}_x64-setup.exe SupaLive_${VER}_arm64-setup.exe"
  for os in macos-aarch64 macos-x86_64 windows-x86_64 windows-aarch64; do echo "supalive-cli_${VER}_$os.zip"; done
}

preflight() {
  [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "要在 main 分支上"
  [ -z "$(git status --porcelain)" ] || die "工作區有未提交的改動"
  git fetch -q origin main --tags
  [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main 和 origin/main 不同步"
  ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "tag $TAG 已經存在"
  for t in $TARGETS_MAC $TARGETS_WIN; do
    rustup target list --installed | grep -qx "$t" || die "缺少 Rust target $t，見 spec 的「本機需要的工具」"
  done
  command -v makensis >/dev/null || die "缺少 makensis（brew install nsis）"
  command -v lld-link >/dev/null || die "缺少 lld-link（brew install lld）"
  command -v cargo-xwin >/dev/null || die "缺少 cargo-xwin（cargo install --locked cargo-xwin）"
  cargo tauri --version >/dev/null 2>&1 || die "缺少 tauri-cli（cargo install tauri-cli --version \"^2\" --locked）"
}

zip_cli() { # <target> <os 名稱> <執行檔名>
  local out="$PWD/$DIST/supalive-cli_${VER}_$2.zip"
  (cd "target/$1/release" && zip -q -X "$out" "$3")
}

build() {
  preflight
  echo "== 檢查 $TAG"
  cargo fmt --all --check
  cargo clippy -q --workspace --all-targets -- -D warnings
  cargo test -q --workspace

  rm -rf "$DIST" && mkdir -p "$DIST"
  local bundle=apps/desktop/src-tauri

  for t in $TARGETS_MAC; do
    echo "== 打包 $t"
    # ad-hoc 簽名，沒有 Developer ID
    (cd "$bundle" && APPLE_SIGNING_IDENTITY=- cargo tauri build --target "$t" --bundles dmg)
    cp target/"$t"/release/bundle/dmg/SupaLive_"$VER"_*.dmg "$DIST/"
    cargo build -q --release -p supalive --target "$t"
    zip_cli "$t" "macos-${t%%-*}" supalive
  done

  # cargo-xwin 要用 Homebrew 的 llvm（clang-cl）；只在打 Windows 時放進 PATH，不影響 macOS 的建置。
  # makensis 在 C locale 下會 bad_alloc 當掉，明確給 UTF-8 locale。
  local llvm; llvm="$(brew --prefix llvm)/bin"
  for t in $TARGETS_WIN; do
    echo "== 打包 $t"
    (cd "$bundle" && PATH="$llvm:$PATH" LC_ALL=en_US.UTF-8 cargo tauri build --runner cargo-xwin --target "$t" --bundles nsis)
    cp target/"$t"/release/bundle/nsis/SupaLive_"$VER"_*-setup.exe "$DIST/"
    PATH="$llvm:$PATH" cargo xwin build -q --release -p supalive --target "$t"
    zip_cli "$t" "windows-${t%%-*}" supalive.exe
  done

  for f in $(artifacts); do [ -f "$DIST/$f" ] || die "缺少 $DIST/$f"; done
  echo "== 完成，產物在 $DIST/"
  ls -lh "$DIST"
}

publish() {
  local notes=${1:-}
  [ -f "$notes" ] || die "要給 Release 說明檔：tools/release.sh publish <說明檔>"
  preflight
  local files=()
  for f in $(artifacts); do [ -f "$DIST/$f" ] || die "缺少 $DIST/$f，先跑 tools/release.sh build"; files+=("$DIST/$f"); done

  echo "== 發佈 $TAG"
  git tag -a "$TAG" -m "SupaLive $VER"
  git push -q origin "$TAG"
  gh release create "$TAG" "${files[@]}" --repo "$REPO" --title "SupaLive $VER" --notes-file "$notes" --verify-tag

  # 修訂號加一
  local next; next=$(echo "$VER" | awk -F. '{print $1"."$2"."$3+1}')
  echo "== 版本號 $VER → $next"
  awk -v v="$next" '/^\[workspace.package\]/{p=1} /^\[/&&!/^\[workspace.package\]/{p=0} p&&/^version *=/&&!d{print "version = \"" v "\"";d=1;next} {print}' Cargo.toml > Cargo.toml.new
  mv Cargo.toml.new Cargo.toml
  cargo update -q --workspace
  git add Cargo.toml Cargo.lock
  git commit -q -m "版本號改為 $next" -m "$VER 已發佈，下一次發佈用這個版本號。"
  git push -q origin main
  echo "== 完成：https://github.com/$REPO/releases/tag/$TAG"
}

case "${1:-}" in
  build) build ;;
  publish) shift; publish "$@" ;;
  *) echo "用法：tools/release.sh build | publish <說明檔>" >&2; exit 2 ;;
esac
