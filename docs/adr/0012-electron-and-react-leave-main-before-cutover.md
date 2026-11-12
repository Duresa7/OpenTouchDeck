# Electron and React leave main before cutover

Decided 2026-11-12. Supersedes the Electron and React timing in ADR 0011. The rest of ADR 0011 stands.

ADR 0011 kept the Electron shell, the React UI and the Companion's local web API frozen on main and deleted all three at M6. The Electron shell and the React UI now leave main at once. The local web API stays for now.

A frozen app still cost work on main. Its pnpm workspace, its lint, unit and end-to-end checks and its three 1.x release workflows ran beside the 2.0 work, and a Rust change had to keep a generated TypeScript client and an embedded web build in step with code nobody was changing.

## What moves and what stays

- **1.x lives on the branch `release/1.x`**, created at the v1.2.0 commit. It holds the Electron shell, the React UI, the 1.x installer recipe and the 1.x release workflows. The bridge release that moves 1.x to 2.0, and any other 1.x fix, are built there. Its release workflow starts on a `vX.Y.Z` tag push and runs the workflow file at the tagged commit, so main needs no 1.x workflow.
- **Main holds 2.0 only**: the Rust workspace, the Firmware, the schema and the 2.0 release pipeline.
- **The Companion's JSON API stays** until the 2.0 screens replace it, as ADR 0011 planned for M6. The Companion's tests and the `td-companion` program still use it. The Companion no longer embeds or serves a web UI: a path outside `/api` is a 404.
- **The TypeScript client is gone**: `pnpm gen:api`, `api.gen.ts` and the test that compared them. The `ts-rs` type declarations on the API types stay until the JSON API goes, because the route registry still reads them.

## Consequences

The `td-companion` program built from main has no window to open: its tray's Open still looks for the 1.x window program next to it. The 2.0 window is `td-app`, which has Home and About so far. People who want a working app use a 1.x release.

Main has no Node workspace. Node is still used in three small places that need no install step beyond `npm ci` in `schema/`: the layout schema checks, the net-v2 reference client, and the release signer and its test.

Pull request checks drop the frozen-app area. Jobs that were split only to run side by side are merged, which makes a run slower and its list of checks shorter: a Rust change runs one job on Windows and one on Ubuntu, and a Firmware change runs one job.

A fix that both lines need is made on main and cherry-picked to `release/1.x`, or the other way round. Nothing merges `release/1.x` back into main.
