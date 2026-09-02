#!/usr/bin/env bash
# The commit gate (CLAUDE.md, REVIEW.md), as one command that mirrors ci.yml:
# every check CI runs, in the same form, so "green locally" means "green in CI".
#   scripts/gate.sh            # full gate
#   scripts/gate.sh --quick    # fmt + clippy + test only (inner loop)
set -uo pipefail
cd "$(dirname "$0")/.."
QUICK=0; [ "${1:-}" = "--quick" ] && QUICK=1
fail=0
step() {
  local name="$1"; shift
  printf '\n\033[1m== %s\033[0m\n' "$name"
  if "$@"; then printf '\033[32mok\033[0m   %s\n' "$name"; else printf '\033[31mFAIL\033[0m %s\n' "$name"; fail=1; fi
}
step "fmt"     cargo fmt --all --check
step "clippy"  cargo clippy --workspace --all-targets --all-features -- -D warnings
step "test"    cargo test --workspace
if [ "$QUICK" = 0 ]; then
  step "doc"       env RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
  step "release"   cargo build --release --locked
  if rustup run 1.88.0 rustc --version >/dev/null 2>&1; then
    step "msrv 1.88" cargo +1.88.0 check --workspace --locked --all-features
  else
    echo "msrv 1.88: toolchain not installed — CI-only check"
  fi
  step "per-crate" bash -c 'cargo check -p gridwatch-store && cargo check -p gridwatch-ui'
  step "features"  bash -c 'cargo check -p gridwatch --no-default-features && cargo check -p gridwatch --all-features'
  step "schemas"   python3 scripts/check-schemas.py
  if command -v cargo-deny >/dev/null; then step "deny" cargo deny check; else echo "deny: cargo-deny not installed"; fi
  step "dup guard" bash -c '! cargo tree -d -p gridwatch 2>/dev/null | grep -E "^(ratatui-core|crossterm) "'
fi
printf '\n'
if [ "$fail" = 0 ]; then echo "GATE GREEN"; else echo "GATE RED"; exit 1; fi
