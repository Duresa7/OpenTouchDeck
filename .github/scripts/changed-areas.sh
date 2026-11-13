#!/usr/bin/env bash
# Reads NUL-separated changed paths on stdin and prints which pull request checks to run,
# as app=, firmware=, e2e=, actionlint= and skills= lines for $GITHUB_OUTPUT.
set -euo pipefail

app=false
firmware=false
e2e=false
actionlint=false
skills=false
while IFS= read -r -d '' path; do
  case "$path" in
    .github/*) app=true; firmware=true; e2e=true; actionlint=true; skills=true ;;
  esac
  case "$path" in
    app/*|schema/*|.github/workflows/app.yml) app=true ;;
  esac
  case "$path" in
    app/ui/*|app/electron/*|app/crates/*|schema/*|app/package.json|app/pnpm-lock.yaml|app/pnpm-workspace.yaml|.github/workflows/app.yml) e2e=true ;;
  esac
  case "$path" in
    firmware/*|schema/*|app/scripts/package_windows.py|.github/workflows/firmware.yml) firmware=true ;;
  esac
  case "$path" in
    .agents/*|.claude/*) skills=true ;;
  esac
done

echo "app=$app"
echo "firmware=$firmware"
echo "e2e=$e2e"
echo "actionlint=$actionlint"
echo "skills=$skills"
