#!/usr/bin/env bash
# Reads NUL-separated changed paths on stdin and prints which checks the change needs, as rust=, scripts=,
# firmware= and actionlint= lines for $GITHUB_OUTPUT.
#
#   rust        the Rust workspace under app/: fmt, clippy and tests on Windows and Linux
#   scripts     the release and data scripts under app/scripts, the release signer, and the agent skills copy
#   firmware    the ESP-IDF builds, the QEMU check and the layout schema
#   actionlint  the workflow files
#
# A path that matches no rule needs no check. Docs are the usual case: their pull request gets a green gate from
# the changes job alone. test-changed-areas.sh holds one example per rule; add one with every rule you add.
#
# The 2.0 installer has no area here: release.yml lists its own paths and starts a release dry run for a pull
# request that changes them. CodeQL has no area either: codeql.yml skips a pull request whose paths all match its
# paths-ignore list (docs, images, agent skills).
set -euo pipefail

rust=false
scripts=false
firmware=false
actionlint=false
while IFS= read -r -d '' path; do
  # The pull request workflow and this filter decide what runs, so a change to them runs everything.
  case "$path" in
    # The Windows evidence scripts run only in windows-evidence.yml, after a merge or by hand.
    .github/scripts/windows-evidence/*) ;;
    # The agent skills check runs in the scripts job alone.
    .github/scripts/skills_mirror.py) scripts=true ;;
    .github/workflows/pr.yml|.github/scripts/*) rust=true; scripts=true; firmware=true ;;
  esac
  case "$path" in
    .github/*) actionlint=true ;;
  esac

  # The Rust workspace, then the files its build scripts and tests read from outside it.
  case "$path" in
    app/crates/*|app/Cargo.toml|app/Cargo.lock|app/rust-toolchain.toml) rust=true ;;
    app/icons/*|schema/*) rust=true ;;
    # td-updater's tests read the signer test's key fixture.
    .github/release-signer-test/fixtures/*) rust=true ;;
    app/scripts/check_exe.ps1|.github/workflows/rust.yml|.github/actions/rust-setup/*) rust=true ;;
  esac

  # The scripts, then every file release_version.py checks, then what the script tests read, then the release
  # signer and its test.
  case "$path" in
    app/scripts/*|app/crates/*/Cargo.toml|app/Cargo.lock) scripts=true ;;
    app/crates/td-host/src/catalog/*|app/crates/td-host/src/packs.json) scripts=true ;;
    .github/release-signer/*|.github/release-signer-test/*|.github/workflows/scripts.yml) scripts=true ;;
    # .claude/skills must stay a copy of .agents/skills.
    .agents/*|.claude/*) scripts=true ;;
  esac

  # The Firmware embeds schema/ files.
  case "$path" in
    firmware/*|schema/*|.github/workflows/firmware.yml) firmware=true ;;
  esac
done

echo "rust=$rust"
echo "scripts=$scripts"
echo "firmware=$firmware"
echo "actionlint=$actionlint"
