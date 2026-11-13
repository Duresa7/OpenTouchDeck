# Agent instructions

## Project scope

- Keep changes focused on the ESP32 TouchDeck project.
- Preserve user customizability and avoid introducing vendor lock-in.
- Follow the existing project structure and document setup changes when needed.

## Commit messages

- Use Conventional Commits: `type: short description`, or `type(scope): short description` when a scope helps identify the affected component.
- Choose the type that best describes the change. Common examples include `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `style`, and `chore`; this list is illustrative, not exhaustive. Use another clear, descriptive type when appropriate.
- Write a concise, imperative description in lowercase, without a trailing period (for example, `fix(touch): correct coordinate mapping` or `docs: update setup instructions`).
- Mark breaking changes with `!` before the colon and explain them in a `BREAKING CHANGE:` footer.

## Public repository

- The repository is public. Never commit, push, or post secrets or personal data: passwords, keys, tokens, Wi-Fi credentials, signing keys, email addresses, Windows user-folder paths, local drive paths, serial port names, hostnames, or real network addresses. The maintainer's name is fine.
- Keep local-only material in git-ignored paths such as `.verify/`. In PRs and comments, cite it by repo-relative path.
- A local pre-push hook blocks pushes that contain such data. Remove the data instead of bypassing the hook.

## Build, test and run

- **App.** From `app/`: `pnpm install --frozen-lockfile`, then `pnpm lint`, `pnpm test`, `cargo test --workspace`, and `pnpm dev:electron` to run the Electron window with a mock Device. Use `pnpm dev:ui:demo` for the React UI with demo data. Build the embedded UI with `pnpm build:ui` before `cargo build -p td-companion`. PR CI runs the Linux app checks on every app change, so a local Linux run is optional: use `app/scripts/linux-check.sh` to run them on `HEAD` in Docker when you are chasing a Linux-only failure (needs Docker; takes about 5 minutes the first time and about 3 minutes after). See [app/README.md](app/README.md) and [docs/setup.md](docs/setup.md).
- **UI packages.** Folders under `app/ui/src/lib/` are deep modules: import a package only through its root files, never its subfolders. See [app/ui/src/lib/README.md](app/ui/src/lib/README.md) before adding or importing one; `pnpm lint` enforces it.
- **Firmware.** See [docs/firmware.md](docs/firmware.md).
- **Proof on the real hardware.** The `verify-touchdeck` skill builds, flashes and drives the Device. `verify-touchdeck-app` drives the app against the mock or real Device.
- Each ticket that changes these commands updates this section in the same PR.

## Resume notes

- On a branch you work on for more than one session, keep `.verify/<branch>/NOTES.md` (git-ignored) current: the goal, the ticket or PR, what is done, what is next, and any open question. Update it before you stop, and read it first when you resume, so a new session or a different model can pick up where you left off.

## Agent skills

`.agents/skills/` is the canonical copy and Claude Code reads the copy in `.claude/skills/`. Edit a skill in `.agents/skills/`, then run `python .github/scripts/skills_mirror.py sync`; CI fails when the two differ. [.agents/skills/README.md](.agents/skills/README.md) lists each third-party set with its source and license.

### Issue tracker

Issues and specs live in GitHub Issues for this repo, managed with `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Worktrees and other machines

`AGENTS.md`, `docs/agents/` and the skills are tracked, so every clone and worktree gets them from git. Change them in a PR like any other file.
