# Native updater groundwork

`td-updater` is a separate crate so release selection, scheduling and verification
can be tested without the Companion web API, Electron, a native window or network
access. It uses the workspace's existing blocking `ureq` transport and
`minisign-verify` for streaming BLAKE2b-prehashed Ed25519 verification, including the
global signature over the trusted comment. It also binds the signed filename and
version to the selected release. It never starts an installer.

The durable public key in `updater.pub` is a byte-for-byte snapshot of
`.github/release-signer/updater.pub` from PR #364
(`ci/release-2.0-msi-pipeline`), embedded using `include_str!`. That PR is still open
when this foundation is written. `build.rs` requires the two files to match once
the release pipeline is in the checkout. No environment override can replace the
trust anchor. Never regenerate the production key (ADR 0005).

The native caller should use its Companion `handle`, load `AppSettings`, and poll
on a worker thread:

```rust,ignore
use td_core::{app_dirs::Dirs, settings::AppSettings};
use td_updater::{GitHubClient, SystemClock, Updater};

let dirs = Dirs::current();
let mut updater = Updater::new(
    GitHubClient::default(), SystemClock::default(),
    env!("CARGO_PKG_VERSION").parse()?, // caller's installed version
);
// On each periodic wakeup, reload the preference; surface load errors and skip
// polling on error, rather than defaulting to enabled.
let settings = AppSettings::load(&dirs)?;
if let Some(result) = updater.poll(&settings, &|| handle.shutdown_requested()) {
    // Handle UpToDate, UpdateReady (owns the temporary files), or Failed.
}
// A native Settings control can persist the opt-out with:
let mut settings = AppSettings::load(&dirs)?;
settings.automatic_update_checks = false;
settings.save(&dirs)?;
```

`None` means disabled, not due, or cancelled. The native caller supplies the
Companion shutdown signal through the cancellation callback. Cancellation is
checked before each request and staging operation, and between MSI reads and
writes; partial staging is removed when the worker returns. A blocking request or
read must return before the worker can observe cancellation, so `GitHubClient`
fails any single read that waits more than 10 seconds. Shutdown waits for the
worker within the runtime shutdown limit; if a read is still blocked when that
limit ends, the process exits and leaves its staging folder. Call
`remove_stale_staging` once at startup, before the first poll and while holding the
single-instance lock, to remove such folders. It only removes real folders named
`touchdeck-update-<random>` that hold nothing but `OpenTouchDeck-*-windows-x64.msi`
files and their `.sig` files. Enabled polling checks
immediately and then at most every 24 hours per updater instance, including
failures. Keep one instance alive for the app's lifetime. The injected clock is
monotonic; the signed timestamp is authenticated metadata, not a freshness policy.
Unsigned GitHub feed metadata cannot authorize a downgrade or relabel an older
signed installer as a newer version.

The GitHub release list is paginated and version-sorted locally, since a latest
release URL could point at 1.x, 3.x or a release published out of version order.
Only stable 2.x releases are eligible. An older selected version returns a typed
failure; an equal version (ignoring build metadata) or no eligible release returns
up to date. Missing/duplicate assets, oversized responses, download errors and
signature failures fail closed. The transport enforces HTTPS across redirects and
a five-minute request deadline with a 10-second limit per read. Limits are 10 pages, 2 MiB per feed page, 16 KiB per
signature and 256 MiB per MSI.

`UpdateReady` owns a unique temporary directory and exposes the MSI path, signature
path and version; dropping it cleans up. Installation, verification immediately
before launch, startup scheduling, native Settings UI, update presentation and the
1.x migration bridge remain later M5 work. An abrupt process termination can leave
temporary files until the next start removes them. See [privacy](../../../../docs/PRIVACY.md).
