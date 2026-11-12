#!/usr/bin/env bash
# Checks changed-areas.sh against example paths. The changes job runs this before it trusts the filter.
# Each line is: check "<the areas that must run, in output order, or none>" <changed path>...
set -euo pipefail

filter="$(dirname "$0")/changed-areas.sh"
failed=0

check() {
  local want=$1 got
  shift
  got=$(printf '%s\0' "$@" | bash "$filter" | sed -n 's/=true$//p' | paste -sd ' ' -)
  if [ "${got:-none}" != "$want" ]; then
    echo "FAIL: $* runs '${got:-none}', expected '$want'"
    failed=1
  fi
}

# Docs and repository files need no check.
check none README.md GLOSSARY.md LICENSE .gitignore .env.example
check none docs/usage.md docs/packaging.md docs/adr/0005-updater-keeps-the-signing-key.md docs/images/app-layout-light.png
check none app/README.md docs/setup.md

# The Rust workspace.
check "rust" app/crates/td-core/src/layout.rs
check "rust" app/crates/td-companion/tests/version_info.rs
check "rust" app/Cargo.toml
check "rust" app/rust-toolchain.toml
check "rust" app/icons/builtin/play.svg
check "rust" docs/companion-api.md
check "rust scripts" app/Cargo.lock
check "rust scripts" app/crates/td-companion/Cargo.toml
check "rust scripts" app/crates/td-host/src/packs.json
check "rust scripts" app/crates/td-link/tests/fixtures/touchdeck-test.tdfw
check "rust scripts" app/scripts/check_exe.ps1
check "rust frozen" app/ui/src/lib/companion/api.gen.ts
check "rust" docs/usage.md app/crates/td-link/src/lib.rs

# The frozen Electron shell and React UI.
check "frozen" app/ui/src/App.tsx
check "frozen" app/ui/e2e/editor.spec.ts
check "frozen" app/electron/src/main.ts
check "frozen" app/electron/installer/installer.nsh
check "frozen" app/pnpm-lock.yaml app/pnpm-workspace.yaml
check "frozen scripts" app/package.json
check "frozen scripts" app/electron/package.json
check "frozen scripts" app/ui/package.json

# The release and data scripts.
check "scripts" app/scripts/release_version.py
check "scripts" app/scripts/tests/test_release_workflow.py
check "scripts" app/release-options.json

# Firmware, and the schema three areas read.
check "firmware" firmware/main/main.c
check "firmware" firmware/sdkconfig.defaults
check "scripts firmware" firmware/tools/make_bundle.py
check "scripts firmware" app/scripts/package_windows.py
check "rust frozen firmware" schema/layout.schema.json
check "rust frozen firmware" schema/protocol/golden-frames-v2.json

# Workflow files run actionlint and the checks they define.
check "rust actionlint" .github/workflows/rust.yml
check "frozen actionlint" .github/workflows/app.yml
check "firmware actionlint" .github/workflows/firmware.yml
check "scripts actionlint" .github/workflows/release-windows.yml
check "frozen scripts actionlint" .github/release-signer/cli.mjs
check "actionlint" .github/workflows/main.yml
check "actionlint" .github/actions/windows-release-build/action.yml
check "rust frozen scripts firmware actionlint" .github/workflows/pr.yml
check "rust frozen scripts firmware actionlint" .github/scripts/changed-areas.sh

# No changed path at all.
check none ""

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "changed-areas.sh: every example runs the checks it should."
