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
check "rust" app/crates/td-link/tests/fixtures/touchdeck-test.tdfw
check "rust scripts" app/scripts/check_exe.ps1
check "rust" docs/usage.md app/crates/td-link/src/lib.rs

# The release and data scripts.
check "scripts" app/scripts/release_version.py
check "scripts" app/scripts/tests/test_release_workflow.py
check "scripts" app/scripts/setup-windows.ps1

# The agent skills and their copy. AGENTS.md and the docs the skills read need no check.
check "scripts" .agents/skills/tdd/SKILL.md
check "scripts" .claude/skills/verify-touchdeck/scripts/touchdeck.ps1
check "scripts actionlint" .github/scripts/skills_mirror.py
check none AGENTS.md docs/agents/issue-tracker.md skills-lock.json

# Firmware, and the schema two areas read.
check "firmware" firmware/main/main.c
check "firmware" firmware/sdkconfig.defaults
check "firmware" firmware/tools/make_bundle.py
check "rust firmware" schema/layout.schema.json
check "rust firmware" schema/protocol/golden-frames-v2.json

# Workflow files run actionlint and the checks they define.
check "rust actionlint" .github/workflows/rust.yml
check "firmware actionlint" .github/workflows/firmware.yml
check "scripts actionlint" .github/release-signer/cli.mjs
check "scripts actionlint" .github/release-signer-test/signer.test.mjs
check "rust scripts actionlint" .github/release-signer-test/fixtures/tauri-cli-2.12.4-test-only-key.json
check "actionlint" .github/workflows/main.yml
check "actionlint" .github/workflows/release.yml .github/workflows/sign-hook.yml
check "rust actionlint" .github/actions/rust-setup/action.yml
check "scripts actionlint" .github/workflows/scripts.yml
check "rust scripts firmware actionlint" .github/workflows/pr.yml
check "rust scripts firmware actionlint" .github/scripts/changed-areas.sh
# The Windows evidence workflow and its scripts run after a merge or by hand, never as a pull request check.
check "actionlint" .github/workflows/windows-evidence.yml .github/scripts/windows-evidence/tray.ps1

# The 2.0 installer: release.yml starts its own dry run for these, so they need no area here.
check none app/installer/OpenTouchDeck.wxs
check "scripts" app/scripts/msi_check.ps1

# No changed path at all.
check none ""

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "changed-areas.sh: every example runs the checks it should."
