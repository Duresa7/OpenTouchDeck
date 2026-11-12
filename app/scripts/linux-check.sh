#!/usr/bin/env bash
set -uo pipefail

usage() {
  cat <<'EOF'
Usage: app/scripts/linux-check.sh [options] [<git-ref>]

Run the Linux app checks from the pull request workflows (rust.yml, app.yml and
scripts.yml under .github/workflows) in Docker.

Checks the committed HEAD of the git checkout it is run from (via git archive
of the repository root), or <git-ref> if given. Uncommitted changes are not
included; commit your changes before running this script.

Options:
  --rust-only    Skip pnpm lint and pnpm test (still runs the release version
                 check and pnpm build:ui, which the Companion embeds).
  --log <path>   Write the full output to <path> instead of a file under the
                 system temp folder.
  --clean        Remove all touchdeck-linux-* Docker volumes and exit.
  -h, --help     Show this help message and exit.

Environment:
  TOUCHDECK_LINUX_VOLUME_PREFIX  Docker volume prefix (default: touchdeck-linux).
EOF
}

vol_prefix=${TOUCHDECK_LINUX_VOLUME_PREFIX:-touchdeck-linux}
rust_only=0
clean_only=0
log_file=""
ref=""

while (( $# > 0 )); do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --rust-only)
      rust_only=1
      shift
      ;;
    --clean)
      clean_only=1
      shift
      ;;
    --log)
      if (( $# < 2 )) || [[ -z "${2:-}" ]]; then
        echo "error: --log requires a file path" >&2
        exit 2
      fi
      log_file=$2
      shift 2
      ;;
    --log=*)
      log_file=${1#--log=}
      if [[ -z "$log_file" ]]; then
        echo "error: --log requires a file path" >&2
        exit 2
      fi
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "error: unknown option '$1' (try --help)" >&2
      exit 2
      ;;
    *)
      if [[ -n "$ref" ]]; then
        echo "error: unexpected argument '$1' (try --help)" >&2
        exit 2
      fi
      ref=$1
      shift
      ;;
  esac
done

if (( $# > 0 )); then
  if [[ -z "$ref" ]] && (( $# == 1 )); then
    ref=$1
  else
    echo "error: unexpected argument '$1' (try --help)" >&2
    exit 2
  fi
fi

if (( clean_only )); then
  vols=()
  while IFS= read -r vol; do
    [[ -n "$vol" ]] && vols+=("$vol")
  done < <(MSYS_NO_PATHCONV=1 docker volume ls --format '{{.Name}}' | grep -E "^${vol_prefix}-" || true)
  if (( ${#vols[@]} == 0 )); then
    echo "No ${vol_prefix}-* volumes found."
  else
    MSYS_NO_PATHCONV=1 docker volume rm "${vols[@]}"
  fi
  exit 0
fi

ref=${ref:-HEAD}

if ! repo_root=$(git rev-parse --show-toplevel 2>/dev/null); then
  script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
  if ! repo_root=$(git -C "$script_dir" rev-parse --show-toplevel 2>/dev/null); then
    echo "error: must be run from a git checkout" >&2
    exit 1
  fi
fi
repo_root=$(cd "$repo_root" && pwd -P)

if ! short_sha=$(git -C "$repo_root" rev-parse --short "${ref}^{commit}" 2>/dev/null); then
  echo "error: unknown git commit '$ref'" >&2
  exit 1
fi

# The Rust image is the toolchain the checked commit pins in app/rust-toolchain.toml. A commit from before
# that file gets the latest stable.
rust_version=$(git -C "$repo_root" show "${ref}:app/rust-toolchain.toml" 2>/dev/null | sed -n 's/^channel = "\([0-9.]*\)"$/\1/p')
rust_image="rust:${rust_version:-1}"

checkout_hash=$(printf '%s' "$repo_root" | git hash-object --stdin | cut -c1-12)
src_vol="${vol_prefix}-src-${checkout_hash}"
cargo_vol="${vol_prefix}-cargo"
pnpm_vol="${vol_prefix}-pnpm"
target_vol="${vol_prefix}-target"

if [[ -z "$log_file" ]]; then
  log_file=$(mktemp "/tmp/touchdeck-linux-check-${short_sha}-${checkout_hash}-XXXXXX.log")
else
  mkdir -p "$(dirname "$log_file")"
  : > "$log_file"
fi

t0=$(date +%s)
overall_rc=0

for vol in "$src_vol" "$cargo_vol" "$pnpm_vol" "$target_vol"; do
  MSYS_NO_PATHCONV=1 docker volume create "$vol" >/dev/null || exit 1
done

export_t0=$(date +%s)
{
  echo "=== STEP START: export ($short_sha -> $src_vol) ==="
  MSYS_NO_PATHCONV=1 docker run --rm -v "${src_vol}:/src" alpine:3.22     sh -c 'set -eu; find /src -mindepth 1 -maxdepth 1 -exec rm -rf {} +' &&
    git -C "$repo_root" archive "$ref" | MSYS_NO_PATHCONV=1 docker run --rm -i -v "${src_vol}:/src" alpine:3.22 tar -x -C /src
} >>"$log_file" 2>&1
export_rc=$?
export_elapsed=$(( $(date +%s) - export_t0 ))
if (( export_rc == 0 )); then
  echo "=== STEP RESULT: export | PASS | ${export_elapsed}s ===" >>"$log_file"
else
  echo "=== STEP RESULT: export | FAIL (exit $export_rc) | ${export_elapsed}s ===" >>"$log_file"
  overall_rc=1
fi

if (( overall_rc == 0 )); then
  MSYS_NO_PATHCONV=1 docker run --rm \
    -v "${src_vol}:/src" \
    -v "${pnpm_vol}:/pnpm-store" \
    -w /src/app \
    -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    -e CI=true \
    -e NO_COLOR=1 \
    -e PYTHONUTF8=1 \
    -e RUST_ONLY="$rust_only" \
    node:24 \
    bash -c '
      set -uo pipefail
      run_step() {
        local name=$1
        shift
        local s0 elapsed rc
        s0=$(date +%s)
        echo "=== STEP START: $name ==="
        "$@"
        rc=$?
        elapsed=$(( $(date +%s) - s0 ))
        if (( rc == 0 )); then
          echo "=== STEP RESULT: $name | PASS | ${elapsed}s ==="
        else
          echo "=== STEP RESULT: $name | FAIL (exit $rc) | ${elapsed}s ==="
        fi
        return "$rc"
      }
      check_release_version() {
        python3 scripts/release_version.py check &&
          python3 -B -m unittest discover -s scripts/tests -p "test_release_version.py" -v
      }
      install_pnpm_deps() {
        corepack enable &&
          pnpm config set store-dir /pnpm-store &&
          flock /pnpm-store/.touchdeck-pnpm.lock pnpm install --frozen-lockfile
      }
      rc=0
      run_step "release-version" check_release_version || rc=1
      if (( rc == 0 )); then
        run_step "pnpm install" install_pnpm_deps || rc=1
      fi
      if (( rc == 0 )); then
        if (( RUST_ONLY == 1 )); then
          echo "=== STEP RESULT: pnpm lint | SKIP (--rust-only) | 0s ==="
          echo "=== STEP RESULT: pnpm test | SKIP (--rust-only) | 0s ==="
        else
          run_step "pnpm lint" pnpm lint || rc=1
          run_step "pnpm test" pnpm test || rc=1
        fi
        run_step "pnpm build:ui" pnpm build:ui || rc=1
      fi
      exit "$rc"
    ' >>"$log_file" 2>&1 || overall_rc=1
fi

if (( overall_rc == 0 )) || grep -q "=== STEP RESULT: pnpm build:ui | PASS |" "$log_file"; then
  MSYS_NO_PATHCONV=1 docker run --rm \
    -v "${src_vol}:/src" \
    -v "${target_vol}:/src/app/target" \
    -v "${cargo_vol}:/usr/local/cargo/registry" \
    -w /src/app \
    -e DEBIAN_FRONTEND=noninteractive \
    -e CARGO_TERM_COLOR=never \
    -e CARGO_PROFILE_DEV_DEBUG=line-tables-only \
    "$rust_image" \
    bash -c '
      set -uo pipefail
      run_step() {
        local name=$1
        shift
        local s0 elapsed rc
        s0=$(date +%s)
        echo "=== STEP START: $name ==="
        "$@"
        rc=$?
        elapsed=$(( $(date +%s) - s0 ))
        if (( rc == 0 )); then
          echo "=== STEP RESULT: $name | PASS | ${elapsed}s ==="
        else
          echo "=== STEP RESULT: $name | FAIL (exit $rc) | ${elapsed}s ==="
        fi
        return "$rc"
      }
      setup_rust_env() {
        apt-get update -qq >/dev/null &&
          apt-get install -y -qq libudev-dev >/dev/null &&
          rustup component add rustfmt clippy >/dev/null 2>&1 &&
          cargo --version
      }
      rc=0
      if ! run_step "rust setup" setup_rust_env; then
        exit 1
      fi
      run_step "cargo fmt" cargo fmt --check || rc=1
      exec 9>/src/app/target/.touchdeck-linux.lock
      flock 9
      run_step "cargo clippy" cargo clippy --workspace -- -D warnings || rc=1
      run_step "cargo test" cargo test --workspace --no-fail-fast || rc=1
      flock -u 9
      exit "$rc"
    ' >>"$log_file" 2>&1 || overall_rc=1
fi

total_elapsed=$(( $(date +%s) - t0 ))

print_summary() {
  echo "Linux check summary for ${ref} (${short_sha})"
  echo "Log: ${log_file}"
  echo ""
  echo "Steps:"
  while IFS='|' read -r step_part status_part time_part; do
    step_name=${step_part#=== STEP RESULT: }
    step_name=${step_name% }
    status_str=${status_part# }
    status_str=${status_str% }
    time_str=${time_part# }
    time_str=${time_str% ===}
    printf '  %-18s %-20s %s
' "$step_name" "$status_str" "$time_str"
  done < <(grep '^=== STEP RESULT: ' "$log_file" || true)

  echo ""
  local total_passed=0 total_failed=0 total_ignored=0 p f ig
  local -a test_lines=()
  while IFS= read -r line; do
    test_lines+=("$line")
    if [[ "$line" =~ ([0-9]+)[[:space:]]+passed\;[[:space:]]+([0-9]+)[[:space:]]+failed\;[[:space:]]+([0-9]+)[[:space:]]+ignored ]]; then
      p=${BASH_REMATCH[1]}
      f=${BASH_REMATCH[2]}
      ig=${BASH_REMATCH[3]}
      total_passed=$(( total_passed + p ))
      total_failed=$(( total_failed + f ))
      total_ignored=$(( total_ignored + ig ))
    fi
  done < <(grep -E '^[[:space:]]*Running (unittests |tests/)|^test result:' "$log_file" || true)

  echo "Test results (${total_passed} passed, ${total_failed} failed, ${total_ignored} ignored):"
  if (( ${#test_lines[@]} == 0 )); then
    echo "  (no Rust test results recorded)"
  else
    for line in "${test_lines[@]}"; do
      echo "  ${line}"
    done
  fi

  echo ""
  echo "Failures / panics:"
  local -a fail_lines=()
  while IFS= read -r line; do
    fail_lines+=("$line")
  done < <(grep -E 'FAILED|panicked' "$log_file" | grep -v '^=== STEP RESULT: ' || true)

  if (( ${#fail_lines[@]} == 0 )); then
    echo "  none"
  else
    for line in "${fail_lines[@]}"; do
      echo "  ${line}"
    done
  fi

  echo ""
  if (( overall_rc == 0 )); then
    echo "Overall: PASS in ${total_elapsed}s (exit 0)"
  else
    echo "Overall: FAIL in ${total_elapsed}s (exit ${overall_rc})"
  fi
}

summary_text=$(print_summary)
printf '%s
' "$summary_text"
{
  echo ""
  echo "=== SUMMARY ==="
  printf '%s
' "$summary_text"
} >>"$log_file"

exit "$overall_rc"
