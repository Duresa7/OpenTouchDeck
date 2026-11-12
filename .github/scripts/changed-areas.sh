#!/usr/bin/env bash
# Reads NUL-separated changed paths on stdin and prints which checks the change needs, as rust=, frozen=,
# scripts=, firmware= and actionlint= lines for $GITHUB_OUTPUT.
#
#   rust        the Rust workspace under app/: fmt, clippy and tests on Windows and Linux
#   frozen      the frozen Electron shell and React UI: lint, unit tests, UI e2e and the Electron specs
#   scripts     the release and data scripts under app/scripts, and the release signer
#   firmware    the ESP-IDF builds, the radio build, the QEMU check and the layout schema
#   actionlint  the workflow files
#
# A path that matches no rule needs no check. Docs are the usual case: their pull request gets a green gate from
# the changes job alone. test-changed-areas.sh holds one example per rule; add one with every rule you add.
set -euo pipefail

rust=false
frozen=false
scripts=false
firmware=false
actionlint=false
while IFS= read -r -d '' path; do
  # The pull request workflow and this filter decide what runs, so a change to them runs everything.
  case "$path" in
    .github/workflows/pr.yml|.github/scripts/*) rust=true; frozen=true; scripts=true; firmware=true ;;
  esac
  case "$path" in
    .github/*) actionlint=true ;;
  esac

  # The Rust workspace, then the files its build scripts and tests read from outside it.
  case "$path" in
    app/crates/*|app/Cargo.toml|app/Cargo.lock|app/rust-toolchain.toml) rust=true ;;
    app/icons/*|schema/*|docs/companion-api.md) rust=true ;;
    app/ui/src/lib/companion/api.gen.ts|app/ui/src/lib/companion/api.test.gen.ts) rust=true ;;
    app/scripts/check_exe.ps1|.github/workflows/rust.yml) rust=true ;;
  esac

  # The frozen Electron shell and React UI. The UI validates Layouts against schema/, and the Electron tests
  # cover the release signer.
  case "$path" in
    app/ui/*|app/electron/*|app/package.json|app/pnpm-lock.yaml|app/pnpm-workspace.yaml) frozen=true ;;
    schema/*|.github/release-signer/*|.github/workflows/app.yml) frozen=true ;;
  esac

  # The scripts, then every file release_version.py checks, then what the script tests read.
  case "$path" in
    app/scripts/*|app/release-options.json) scripts=true ;;
    app/package.json|app/electron/package.json|app/ui/package.json|app/crates/*/Cargo.toml|app/Cargo.lock) scripts=true ;;
    app/crates/td-host/src/catalog/*|app/crates/td-host/src/packs.json|app/crates/td-link/tests/fixtures/*) scripts=true ;;
    firmware/tools/*|.github/release-signer/*|.github/workflows/release-windows.yml) scripts=true ;;
  esac

  # The Firmware embeds schema/ files, and its radio-free check runs the release recipe's audit.
  case "$path" in
    firmware/*|schema/*|app/scripts/package_windows.py|.github/workflows/firmware.yml) firmware=true ;;
  esac
done

echo "rust=$rust"
echo "frozen=$frozen"
echo "scripts=$scripts"
echo "firmware=$firmware"
echo "actionlint=$actionlint"
