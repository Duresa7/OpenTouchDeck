# Companion local API, v1

This is the contract between the Companion and its clients: the Electron window, a browser tab, and tests. Every Companion, Electron and UI ticket builds against it. When the code and this page disagree, one of them has a bug. The `api_contract_is_documented` test fails the build when an endpoint, event or error code is missing here.

Related decisions: [ADR 0001](adr/0001-rust-companion-behind-electron.md) (Rust Companion behind Electron, loopback only), [ADR 0005](adr/0005-updater-keeps-the-signing-key.md) (updates stay in Electron) and [ADR 0006](adr/0006-command-approval-on-the-pc.md) (command approvals happen on the PC).

## At a glance

- JSON routes under `http://127.0.0.1:PORT/api/v1`, served by axum, bound to `127.0.0.1` only.
- One WebSocket at `/api/v1/events`. It opens with a `sync` snapshot of the shared state, then sends full-object deltas.
- The Companion serves the built UI from the same origin, so the window and a browser tab run the same page.
- Every call carries `Authorization: Bearer tds_…`, a session token. Session tokens are minted from a pairing token that rotates on every Companion start. There are no cookies and no CORS.
- Electron's main process checks an HMAC proof that the Companion knows the pairing token before it sends the token. Browsers pair with a one-time code.
- Devices are addressed by their USB-serial Device id. A New board keeps its id when it becomes a Device.
- Slow Device work is a Job, at most one per Device.
- Every error has one shape: `{"error": {"code", "message", "details"}}`.
- No session can approve a command. Approvals happen in a native dialog on the PC.
- Routes are declared once with a typed `Endpoint` trait. ts-rs generates the TypeScript client, and a test fails when the committed file is stale.
- Each generated wrapper takes an optional `AbortSignal` after its headers, so a client can drop a request it no longer needs. Query parameters set to `null` or `undefined` are left out of the URL.
- `/api/v1` changes only by addition.

## Transport

| Part | Rule |
|---|---|
| Requests | `GET`, `POST`, `PUT`, `PATCH` and `DELETE` on `/api/v1/...`. JSON bodies use `application/json`. Image uploads use `image/*` |
| Responses | JSON, or raw bytes with their media type for Asset files. `204` for an empty success |
| Events | `GET /api/v1/events` upgrades to a WebSocket. The server sends events. The client sends only `subscribe` and `unsubscribe` |
| Slow work | `202 Accepted` with a `Job`, then `job.updated` events. `GET /jobs/{id}` recovers after a reload |
| UI | Every other path serves the embedded UI. Unknown non-API paths return `index.html`, so client routes work after a reload |

We compared three shapes. REST plus one WebSocket won because Firmware bundles, Asset uploads and icon bytes travel as plain request and response bodies, and every route can be tested with `curl` or `reqwest`.

- **JSON-RPC 2.0 over one WebSocket.** It orders replies and events on one stream. It loses on binary data (base64 adds a third, and a 6 MB frame stalls console and progress events), on reconnects (every in-flight call fails), on middleware (Host and Origin checks per message) and on testing. The `sync` snapshot and full-object Job updates give REST the ordering where it matters.
- **HTTP plus Server-Sent Events.** `EventSource` can't send an `Authorization` header, so the token would go in a cookie or the URL. It flows one way, so changing subscriptions needs extra routes.

## Security model

### Threat model

A valid session can push a Layout and read Integration status. Running a program also needs a person to answer a native dialog on the PC ([ADR 0006](adr/0006-command-approval-on-the-pc.md)). Reviews still grade every auth defect as severe: it gives an attacker the user's Devices, Layouts and keys typed by key Tiles.

| Actor | Reaches `127.0.0.1:PORT`? | Defense |
|---|---|---|
| A web page in the user's browser (cross-site fetch, form post, WebSocket) | Yes, through the browser | No ambient credential, an Origin check that also covers the WebSocket upgrade, no CORS, `Sec-Fetch-Site` |
| DNS rebinding (`evil.example` resolves to `127.0.0.1`) | Yes, with `Host: evil.example:PORT` | Host allowlist on every request, static files included |
| Another Windows user on the same PC | Yes, loopback is shared | A runtime file only the owner can read, a pairing token that rotates per start, the HMAC proof before Electron sends the token, a lockout on wrong pairing codes |
| A process running as the same user | Yes | Out of scope. It can read the user's files and keyring anyway |
| Another machine on the network | No | The Companion binds `127.0.0.1` only and asserts `local_addr().ip().is_loopback()` at start. No setting changes the bind address (ADR 0001) |

### Secrets

All secrets come from the OS random source (`getrandom`). Tokens are base64url without padding. The prefixes let the pre-push hook and secret scanners recognize them.

| Secret | Format | Entropy | Lifetime | Stored |
|---|---|---|---|---|
| Pairing token | `tdp_` + 43 chars | 256 bits | Until the next Companion start or Reset pairing | `runtime/companion.json`, readable only by the user |
| Window session token | `tds_` + 43 chars | 256 bits | Until its window closes or the Companion exits | Companion memory and Electron main memory only |
| Browser session token | `tds_` + 43 chars | 256 bits | Until the Companion exits, 7 days idle, revoke or Reset pairing | Browser `localStorage`; `SHA-256(token)` only in Companion memory |
| Pairing code | 12 Crockford base32 chars shown as `XXXX-XXXX-XXXX` | 60 bits | 120 s, single use, at most one outstanding (a new code replaces the old) | Companion memory only |

- The Companion compares the pairing token with `subtle::ConstantTimeEq`.
- It looks up session tokens by `SHA-256(token)`. The hash input is secret, so lookup timing leaks nothing useful.
- Five wrong pairing codes in a row invalidate every outstanding code. That caps a local guesser at 5 tries against 60 bits per round. The tray says "Pairing was blocked after wrong codes. Start pairing again." An attacker can only make the user start again.
- Logs never contain tokens or codes. The `tracing` layer redacts `Authorization` and `Sec-WebSocket-Protocol`.

### Port and runtime file

**Port.** On first run the Companion picks a random free port in 49152 to 65535 and saves it in `companion-settings.json`, which isn't secret. Later starts reuse it so browser bookmarks keep working. `localStorage` is scoped to `127.0.0.1:PORT`, but its token no longer represents a valid session after a restart; the browser clears it on 401 and pairs again. If the port is taken, the Companion picks and saves a new one, and browsers must pair again. The tray says so.

**Location.** `%LOCALAPPDATA%\io.github.duresa7.touchdeck\runtime\companion.json`, built from `dirs::data_local_dir()` and `td_core::app_dirs::APP_ID`.

- It's under Local, not Roaming, because the token belongs to this PC and must not roam with a domain profile.
- `TOUCHDECK_APP_DATA_DIR` moves it to `<dir>/runtime` for tests and scratch runs.
- On Linux source builds it's `$XDG_RUNTIME_DIR/touchdeck/companion.json`, in a `0700` directory, as a `0600` file opened with `O_NOFOLLOW`.

**Contents.**

```json
{ "version": 1, "api": ["v1"], "port": 51234, "token": "tdp_…", "pid": 4242,
  "build": "<git sha>", "started_at_ms": 1767225600000 }
```

**Writing it on Windows** (crate `windows-sys`):

1. Read the user's SID from the process token (`OpenProcessToken`, `GetTokenInformation(TokenUser)`).
2. Create `runtime\` with `CreateDirectoryW` and `SECURITY_ATTRIBUTES` from `O:<SID>D:P(A;OICI;FA;;;<SID>)`, so it's the user's from the start. If `runtime\` already exists and is a reparse point (a junction or symlink), refuse to start. If another account owns it, refuse to start with a clear message; the Companion never takes ownership. Otherwise apply the protected DACL `D:P(A;OICI;FA;;;<SID>)` through `SetNamedSecurityInfoW` and `PROTECTED_DACL_SECURITY_INFORMATION`, leaving the owner alone, and verify the folder as step 5 verifies the file. Nothing inherits from `%LOCALAPPDATA%`. There's no Administrators or SYSTEM entry, because admins can take ownership anyway and backup tools use the backup privilege.
3. Create `runtime\.companion.json.<pid>.tmp` with `CreateFileW(CREATE_NEW)` and `SECURITY_ATTRIBUTES` from `O:<SID>D:P(A;;FA;;;<SID>)`. The file is private from its first byte.
4. Write it, call `FlushFileBuffers`, then `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`. A rename on one volume keeps the file's own security descriptor.
5. Read the final file's security with `GetNamedSecurityInfoW`. The DACL must be protected, hold exactly one allow entry for the user's SID, and the owner must be the user. Otherwise the Companion deletes the file, refuses to serve the API, and shows a tray error. A failed rename in step 4 only removes the temporary file; the previous file stays.
6. Write the file only after the listener is bound. Delete it on clean shutdown. Sessions are never written to disk.
7. Every 60 s, and on Reset pairing, check that the file exists with the expected contents and DACL, and rewrite it only if it drifted. The check runs off the async runtime under one lock. A failure is retried 3 times with backoff (0.2 s, 1 s, 3 s) before the Companion shuts down.

**Single instance.** An exclusive lock on `runtime\companion.lock`, taken after the folder is secured in step 2. Only the user can create files there, so another account can't hold the lock first, as it could squat a named mutex. A second Companion for the same folder exits with "The Companion is already running." Each user has their own folder, so two signed-in users get two Companions, each with its own port and file. Two sign-ins of the same user share one.

### Request rules

Tower layers apply these to every request, including static UI files and the WebSocket upgrade, before routing. The decision is one pure function, `admit(&RequestMeta, port) -> Result<(), SecurityError>`, with unit tests for each row.

| # | Check | Passes | Fails with |
|---|---|---|---|
| 1 | `Host` | Exactly `127.0.0.1:PORT`. A missing `Host` fails, and so does `localhost:PORT`: only `127.0.0.1` is bound, and `localhost` can resolve to `[::1]`, where another process may listen on the same port | 421 `security.host` |
| 2 | `Origin`, if present | Exactly `http://127.0.0.1:PORT`. `null` fails | 403 `security.origin` |
| 3 | `Sec-Fetch-Site`, if present, on `/api/*` | `same-origin` or `none` | 403 `security.origin` |
| 4 | `Sec-Fetch-Site`, if present, on static `GET`s | Anything with `Sec-Fetch-Mode: navigate`, otherwise `same-origin` or `none` | 403 `security.origin` |
| 5 | Auth on `/api/v1/*`, except the public routes below | A valid `Authorization: Bearer tds_…`, or the WebSocket subprotocol | 401 `auth.missing`, `auth.invalid`, `auth.revoked` or `auth.expired` |
| 6 | Body size, set per endpoint | JSON 1 MiB by default. Image uploads 16 MiB. Icon pack imports 32 MiB | 413 `request.too_large` |
| 7 | `Content-Type` | `application/json` for JSON bodies, `image/*` for uploads | 415 `request.media_type` |

- **Public routes.** `GET /system/ping` returns `{api: ["v1"]}`. `POST /auth/proof` returns the HMAC proof. `POST /auth/sessions` needs the pairing token. `POST /auth/pair` needs a pairing code. Features can't add public routes, because `Endpoint` has no auth switch. The auth module mounts these four itself, before the auth layer.
- **CORS.** The Companion never sends `Access-Control-*` headers. A preflight `OPTIONS` carries a foreign `Origin` and fails rule 2.
- **CSRF.** No credential is ambient, so a forged request carries no token. Requiring `application/json` also blocks plain cross-site form posts before auth runs.
- Requests without `Origin`, such as Electron main, tests and scripts, pass rules 2 to 4 and still need auth.

### Limits

These initial limits are reversible; changing them requires updating this contract and its saturation tests.

| Resource | Limit |
|---|---|
| Pending uploads | 8 per session, 64 MiB pending across the Companion; reserve count and bytes while receiving, free on consumption, expiry or session end |
| Decoded images | At most 4096×4096 pixels; GIFs also at most 300 frames and 64 MiB total decoded data |
| Asset render Jobs | 2 at a time across the Companion |
| Icon pack installs | 2 Jobs at a time across the Companion, one per pack. Downloads come only from `registry.npmjs.org`, at most 32 MiB, within 120 s |
| Connections | 128 open at once; the accept loop waits for a free slot |
| Request headers | 5 s to arrive, then the connection is dropped |
| Request body | 10 s to arrive, then 408 `request.timeout`. Public routes read at most 4 KiB |
| Whole request | 30 s, then 408 `request.timeout`. The WebSocket upgrade is exempt, since the socket runs after it. `firmware.pick` is exempt too, since it waits for the native file dialog; its header and body limits still apply |
| Shutdown drain | 5 s for open requests, then their connections are dropped |
| WebSockets | 4 per session, 16 across the Companion |
| Socket queues | 1024 frames or 4 MiB per socket, whichever comes first; overflow follows Backpressure |
| Pending approvals | 16 across the Companion, including the displayed request; deny and log new requests above the cap |
| Firmware consent dialog | 5 minutes, then the Job fails with `firmware.declined` |
| Picked Firmware bundles | 16 across the Companion, each kept 10 minutes; a pick above the cap gets 429 `limits.exceeded` |
| `approvals.show` | At most once every 5 s per request, shared across sessions |

Size and decode limits return 413 `request.too_large` (or that code in a render Job's error if decoding has begun). Count, aggregate capacity and rate limits return 429 `limits.exceeded`; Device-originated approvals above the cap are denied and logged with that code. Checks reserve capacity atomically before allocating work and release it on failure or completion. Device event floods must not starve Layout Jobs.

### Response headers

Every response, API and static, carries:

```
Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
  img-src 'self' blob: data:; connect-src 'self' ws://127.0.0.1:PORT;
  font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
Cache-Control: no-store
```

Hashed static files override `Cache-Control` with `public, max-age=31536000, immutable`. `index.html` keeps `no-store`. The UI makes no third-party requests unless you turn on Brandfetch: the Companion fetches and caches icon catalogs and downloads Icon packs, and the UI shows pack icons through the same-origin `icon_packs.thumbnail` route, so `connect-src` and `img-src` stay on `'self'`. While a Brandfetch client ID is saved, and only then, the policy adds `https://cdn.brandfetch.io https://asset.brandfetch.io` to `img-src` and `https://api.brandfetch.io` to `connect-src`, so the icon picker can search Brandfetch and show its thumbnails (see [Brandfetch](#brandfetch)). The page sets its policy when it loads, so saving or forgetting the client ID reloads the window. Without a client ID the header is byte for byte the one above. The UI never uses `dangerouslySetInnerHTML`.

### How the Electron window gets its session

1. Electron main is single-instance. It reads `companion.json`. If the file is missing or the proof in step 2 fails, it starts the Companion and waits up to 10 s for a file with a newer `started_at_ms`.
2. **It checks the Companion before sending the token.** It sends `POST /api/v1/auth/proof` with `{nonce}`, 32 random bytes, and receives `{proof}`, which is `base64url(HMAC-SHA256(pairing_token, "touchdeck-companion-proof\0" || nonce))`. Main recomputes the HMAC with Node's `crypto` and compares with `timingSafeEqual`. A process squatting on the port can't answer, so it never receives the pairing token and the window never loads its page.
3. **It mints a session.** `POST /api/v1/auth/sessions` with `Authorization: Bearer tdp_…` and `{kind: "window"}` returns `{id, token}`.
4. `BrowserWindow` loads `http://127.0.0.1:PORT/` with `contextIsolation: true`, `sandbox: true` and `nodeIntegration: false`. `will-navigate` and `setWindowOpenHandler` keep it on the Companion origin. Other URLs open in the default browser.
5. The preload exposes exactly `window.openTouchDeck = { version: 1, session(stale?: string): Promise<string | null>, onSessionChanged(cb), updates: { check, download, install } }`. `ipcMain.handle('session')` returns the token only when `event.senderFrame` is the top frame and its origin is the Companion origin. The renderer never sees the pairing token.
6. On a 401 with an `auth.*` code, or WebSocket close 4401, the client passes the rejected token to `session(stale)`. If it is still current, main re-reads the file, checks the proof, mints a session, revokes the old one, and fires `onSessionChanged`. Concurrent renewals share that operation; a later request naming the old token receives the replacement. Calls without a stale token only read the current session, including the call made on `onSessionChanged`. The client retries once.
7. When the window closes, main sends `DELETE /auth/sessions/self`.

### How a browser pairs

The user starts pairing from **Open in browser** in the tray or in the window. The Companion mints a pairing code and does three things with it:

1. It opens `http://127.0.0.1:PORT/pair#code=XXXX-XXXX-XXXX` in the default browser. The code is in the fragment, which browsers never send to a server or in a `Referer`.
2. It shows the code for typing by hand only in a native tray notification. The window never displays or copies the code or link. Pairing codes never appear in API responses; `POST /auth/pair-codes` returns only `{expires_ms}` after the Companion opens the default browser itself.
3. The Companion always opens the fragment form. It can't tell whether a browser dropped the fragment, so it never retries with another link; the code in the tray notification is the fallback. The `/pair` page reads no code from the query string, because the server sees a query and the browser keeps it in history. It removes the fragment and any query with `history.replaceState(null, '', '/pair')` before it connects or renders anything.

The `/pair` page says "Pair this browser again after the Companion restarts." It reads the code from the fragment or a text field where the user types it. Input ignores case, spaces and dashes, and reads `O` as `0` and `I` or `L` as `1`. The page sends `POST /api/v1/auth/pair` with `{code, label}`, where `label` comes from the user agent (for example "Edge on Windows"), and receives `{id, token}`. It stores the token in `localStorage` as `touchdeck.session`, then goes to `/`.

- A used, expired or wrong code returns 401 `auth.code_invalid`. The page says "This pairing code has expired or is wrong. Choose Open in browser from the tray icon again."
- After the lockout it returns 429 `auth.code_locked`.
- A browser that opens the Companion without a session gets 401 on `GET /api/v1/system` and shows the `/pair` page with the text field.
- `POST /auth/pair-codes` needs a window session. A browser session gets 403 `auth.forbidden`, so a stolen browser token can't mint more sessions that would outlive its revoke.

### Session lifecycle

| | Window session | Browser session |
|---|---|---|
| Minted by | Electron main, with the pairing token | `POST /auth/pair`, with a pairing code |
| Stored | Memory only, in the Companion and Electron main | Browser `localStorage`. The Companion stores its SHA-256 hash only in memory |
| Ends when | Its window closes (`DELETE /auth/sessions/self`), the Companion exits, or no WebSocket from it has been open for 60 s | The Companion exits, 7 days pass without a request, or it is revoked or pairing is reset |
| Can mint pairing codes | Yes | No |

- `last_used_ms` stays in Companion memory; no session state is persisted. At most one pairing code is pending, and Reset pairing invalidates it.
- **Settings → Sessions** lists every session (`GET /auth/sessions`) with its kind, label, created time and last use. The caller's own row is marked. Each row has **Revoke** (`DELETE /auth/sessions/{id}`). **Sign out all browsers** is `DELETE /auth/sessions?kind=browser`. **Reset pairing** is `POST /auth/reset`.
- Any session may revoke one session or reset pairing, because both only take access away. Forget all browsers requires a window session; a browser receives 403 `auth.forbidden`. It revokes all browser sessions under one auth lock, sending each browser socket `auth.revoked` followed by close 4401. Window sessions, including the caller, remain active.

### Rotation and revoke

| Trigger | Pairing token | Window sessions | Browser sessions | What clients see |
|---|---|---|---|---|
| Companion start | New | All gone | All gone | The window mints a new session silently. Browsers must pair again |
| **Reset pairing** in Settings or the tray | New; pending pairing codes are invalidated. The file is rewritten atomically and the old token is rejected at once | Revoked | Revoked | Every WebSocket gets `auth.revoked`, then close 4401. In-flight calls get 401 `auth.revoked`. The window mints a new session silently. Browsers show "This browser was signed out. Pair it again from the tray icon." |
| Revoke one session | Unchanged | That one, if it's a window session | That one, if it's a browser session | That client gets 4401 and the sign-out message. A revoked window mints again through Electron main |
| 7 days idle | Unchanged | Not affected | That one expires | 401 `auth.expired`, then the `/pair` page |
| `companion.json` deleted or changed while running | Unchanged | Not affected | Not affected | Nothing. The 60 s check rewrites it |

### WebSocket auth

Browsers can't set headers on a WebSocket. The client opens `new WebSocket(url, ['touchdeck.v1', 'touchdeck.bearer.' + token])`. The server:

- requires a valid `Origin` whenever one is present (rule 2),
- reads the token from the `touchdeck.bearer.*` entry,
- selects only `touchdeck.v1` in its reply, so the token is never echoed.

No token ever goes in a query string. An upgrade with an invalid, expired or revoked session is refused with HTTP 401 before any socket exists. A browser page can't read that status: it sees only close code 1006, the same as a network drop. So a client whose socket closes before its `hello` checks its session with `GET /system`, which answers 401 for a dead session. Close codes on an open socket:

| Code | Meaning |
|---|---|
| 4401 | The session became invalid, expired or was revoked while the socket was open |
| 4008 | The client is too slow (see Backpressure) |
| 1001 | The Companion is shutting down |

A refused `Host` or `Origin` never reaches a socket: the upgrade gets HTTP 421 or 403 with the error body.

### Command approvals happen on the PC

[ADR 0006](adr/0006-command-approval-on-the-pc.md) records the decision. The contract:

- **What needs approval.** Every action that starts a process: a command, a program launch, and opening anything other than an `http`, `https` or `mailto` URL. The custom command Integration (#101) follows the same rule. An approval is keyed by a hash of the action kind, program, arguments, working folder and shell.
- **The dialog.** The Companion's `ApprovalPrompter` shows a native task dialog it owns. It names the Device, the Page and the Tile, and shows the full program, arguments, working folder and shell. It quotes every value from the Device or its Layout and writes control, format, line separator and paragraph separator characters (such as U+202E) as visible `\u{202E}` escapes, so no value can hide or reorder text. The action that runs and its hash keep the raw values. The buttons are **Run once**, **Always allow** and **Deny**. Deny is the default. One dialog shows at a time. Others wait in order.
- **Failures deny.** A dialog that can't be shown, or an **Always allow** that can't be saved, denies the request and logs why.
- **Taps and transfers.** A tap reads the Device's Layout in the same per-Device queue as Layout and Asset Jobs, so a tap during a push waits for it. A tap that waits more than 10 seconds is logged as `failed` and dropped, never replayed.
- **Device input.** While any approval or Firmware confirmation dialog is showing, the Companion injects no key input from any Device. It drops that input rather than replaying it later, and the key mirror reports `outcome: "suppressed"` so Device input cannot answer the dialog.
- **Type-text decision (#76).** Keep physical key Tiles compatible: the current link carries keyboard reports, not a complete text intent that can be hashed and approved. A length threshold on reports is bypassable, so it is not an approval boundary. All mirrored input is dropped during dialogs. A future Firmware contract must carry text intent before long-text approval can be enforced.
- **Repeats.** Pressing the same Tile again while its request is pending adds no second request.
- **Expiry.** A request nobody answers expires after 5 minutes and is logged as `expired`.
- **Foreground.** Windows may keep a tray program's dialog from taking focus, so the tray also shows a notification. `POST /approvals/{id}/show` shows the dialog again.
- **What the API offers.** `GET /approvals` lists pending requests, and they're also in `sync`. `approval.requested` and `approval.resolved` events let every open client show and clear "Waiting for approval on this PC". `GET /actions/allowed` lists always-allowed actions. `DELETE /actions/allowed/{hash}` removes one. No route answers a request or adds an approval.
- **Tests.** Under `test-hooks`, a `FakePrompter` replaces the dialog and answers from a script set with `POST /test/approvals/script`. The test `release_router_answers_no_approvals` builds the release router and asserts it has no `/test/` route and no route that changes an approval except `DELETE /actions/allowed/{hash}`.

### Firmware files come from the PC

A session can install only two kinds of Firmware bundle: the one bundled with the Companion, or a `.tdfw` file the user picks in a native file dialog that the Companion opens. No route accepts bundle bytes.

- `POST /firmware/bundles/pick` takes `{device}` (the Device or New board id) and opens the dialog on the PC, filtered to `*.tdfw`. It returns `{bundle: BundleInfo | null}`, where `null` means the user cancelled. The Companion parses and checks the file with `Bundle::parse` and keeps it for 10 minutes under a single-use `BundleId` bound to that Device. A different target cannot use it; accepting a Job consumes it, even if the user later declines.
- A second pick while a dialog is open gets 409 `system.dialog_open`.
- Firmware routes take `{bundle: "bundled" | BundleId, channel?: "normal" | "radio"}`.
- **Two builds.** The Companion carries the normal build and, in a release, the opt-in radio build with Wi-Fi and Bluetooth. A Device's v2 `hello` says `radio: true` when it runs the radio build, and `FirmwareInfo.channel` reports `"normal"` or `"radio"`. Without `channel`, a route installs the build the Device runs, and a New board gets the normal build. Naming the other build in `channel` asks to switch, and a switch always waits for the native consent dialog, which says what the radio build turns on, or that Wi-Fi and Bluetooth setup stays stored but unused after switching back. A bundle whose manifest is not the build asked for, such as a picked radio `.tdfw` for a Device on the normal build, gives 409 `firmware.channel_mismatch` with `details {channel, bundle}`, and nothing is written.
- `firmware.install` on a New board, every `firmware.repair`, and a `firmware.update` to an older version wait for a native PC confirmation dialog owned by the Companion. It asks in plain words (install on this New board, repair this Device, or downgrade this Device) and names the exact target, its port, what it runs now if known, and the bundle version, each quoted with control, format and separator characters escaped. Consent is bound to that target and bundle; no API request can grant it. Until consent, no write starts and the Job state is `waiting_for_confirmation`. Declining fails the Job with `firmware.declined`.
- A consent dialog nobody answers closes after 5 minutes, as an approval does, and fails the Job with `firmware.declined`. Cancelling the Job while the dialog waits closes the dialog and ends the Job as `cancelled` at once; an answer that lands after the cancel or the deadline is ignored. Either way nothing is written, the Device leaves `updating`, Device input is no longer suppressed and the install lock can be taken again.
- Updating to the bundled build when it is the same or newer needs no confirmation and stays one click.

Implementation decisions for #77:

- `TOUCHDECK_FIRMWARE_BUNDLE` selects the `.tdfw` embedded at build time, and `TOUCHDECK_RADIO_FIRMWARE_BUNDLE` the radio build's. Source builds without them report `firmware.bundle_invalid` for the bundled selection of that build; they never substitute mock Firmware on real ports. When one is set, `build.rs` runs `Bundle::parse` on the file and fails the build if the bundle is malformed or is the other build: the normal variable refuses a manifest with `"radio": true`, and the radio variable refuses one without it. When `<variable>_SHA256` is also set, as the release recipe does, `build.rs` also fails unless the file has that SHA-256.
- Versions that cannot be ordered as semantic versions require confirmation unless the version strings match.
- The Device manager reserves the target, lends its existing session for OTA, closes the session before ROM recovery or reconnect, and resumes discovery on every exit path.
- A newly installed board uses `needs-setup`, overriding the ordinary discovery default of `ready`.

Image uploads for Assets stay on the API (`POST /uploads`). The Companion decodes and re-renders every image to the Device's Tile size, and nothing it uploads is ever run.

## Devices and New boards

### Identity

- **`DeviceId`** is the USB serial number from `serialport::UsbPortInfo.serial_number`, which is the chip's MAC on the ESP32-S3 USB-Serial/JTAG, lowercased with separators removed, such as `a0b1c2d3e4f5`.
  - It's an opaque string matching `^[0-9a-z]{1,32}$`. The `DevicePath` extractor checks it once, and handlers receive a typed `DeviceId`.
  - Protocol v2 `hello` also reports the id, and the Companion compares the two. If they disagree, the USB serial wins and the Companion logs a warning. v1 Firmware has no id in `hello`, and the USB serial still identifies it.
  - If a driver reports no serial number, the id falls back to `port` followed by the port name lowercased with everything but letters and digits removed, such as `portcom7`, and the summary sets `stable_id: false`. #74 dropped the hyphen this rule used to have, so the fallback matches the id pattern.
  - If two Devices' ids come out the same, the one that already holds it keeps it. The other gets the first 27 characters, `x`, and 4 hex digits of its port name's SHA-256, such as `a0b1c2d3e4f5x9c1e`. That id holds while the Device stays on that port, so its summary sets `stable_id: false`.
  - Mock Devices use `mock0001`, `mock0002` and so on.
- **The port** (`COM7`) is for diagnostics only. It appears in `DeviceSummary.link.port` and never in a route. Every command that took a `port` today takes a `DeviceId` instead.

### Collections

| Collection | Members | Lifetime |
|---|---|---|
| `/devices` | Every known Device: connected, offline, or needing setup | Saved in `devices.json` with its id, name and setup state. `DELETE /devices/{id}` forgets one |
| `/new-boards` | Ports with the Espressif USB VID and PID whose serial isn't a known Device and that don't answer as Open TouchDeck | Present only while plugged in |
| `/ports` | Raw probe results for troubleshooting | Live |

- **A New board becomes a Device under the same id.** After a `firmware.install` Job succeeds, the events are `new-board.removed {id}`, then `device.added {id}` with `setup: "needs-setup"`. Naming it and choosing a starting Layout then use the ordinary Device routes.
- **A known Device that stops answering stays a Device.** If a port's serial matches a known Device that won't say `hello`, its `link.state` is `unresponsive` and the UI offers `POST /devices/{id}/firmware/repair`. It never becomes a New board, so its name and Layout stay attached.
- A port held by another program is a New board with `probe: "busy"` when its serial is unknown, and the Device with `link.state: "busy"` when it's known.

### Link states

`LinkState` replaces today's `DeckInfo.online` and the `link-closed` event.

```rust
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum LinkState {
    Connected { port: String, since_ms: u64, protocol: u32 },
    Connecting { port: String },
    Offline,                                    // known, not plugged in
    Lost { reason: LostReason, detail: String, at_ms: u64 },  // dropped on its own, retrying
    Disconnected,                               // the user chose Disconnect, no auto-connect until replug or Connect
    Busy { port: String, detail: String },      // another program holds the port
    Unresponsive { port: String },              // known serial, no hello
    Updating { job: JobId },                    // a Firmware Job owns the port
}
pub enum LostReason { Unplugged, Timeout, IoError, Closed }
```

`Lost` and `Disconnected` are separate variants, so the UI can't confuse "Connection lost" with "Device not connected".

How the Device manager (#74) moves between them:

| From | Event | To |
|---|---|---|
| `offline`, `lost` with `unplugged` | Its port appears | `connecting` |
| `connecting`, `lost` | `hello` answers | `connected` |
| `connecting` | Still silent 3 s after the port appeared (the boot grace), or answers as other Firmware | `unresponsive` |
| `connecting`, `unresponsive` | The port is held by another program | `busy` |
| `connected` | The session ends on its own: missed pings, an I/O error, a closed stream, a ping that shows the Device restarted, or the Device dropping the session (a heartbeat `link.state` reads `online: false`, or a v2 request is refused `not-negotiated`) | `lost` with `timeout`, `io-error` or `closed`. The Companion keeps probing; silence keeps it `lost`. A probe that answers starts a new session with a fresh `hello`. After a link that lasted under 30 s, the next probe waits 250 ms, doubling for each such link in a row up to 10 s |
| `connected` | `devices.restart` | `connecting`, then `connected` with a fresh `hello` once it boots |
| `connected`, `lost` | Its port leaves | `lost` with `unplugged`, until replug or restart |
| `connecting`, `busy`, `unresponsive`, `disconnected` | Its port leaves | `offline`. Unplugging also ends a Disconnect |
| any | `devices.disconnect` | `disconnected` |
| `disconnected` | `devices.connect` | `connecting` if plugged in, else `offline` |

- The scan thread wakes on a USB change (`CM_Register_Notification` on Windows) and polls every second as a fallback. Each port is probed with `hello` at a 500 ms timeout. A port that isn't a Device is probed again after 0.25 s, doubling up to 10 s; `ports.rescan`, `devices.connect`, a lost link and a replug reset that.
- A probe that finds a Device keeps its session, so a Device connects with one open. The session and its blocking calls live on that Device's actor thread.
- A silent board stays unclassified during the boot grace, so a Device being plugged in doesn't flash `unresponsive` or a New board.
- On Windows a Device that restarts keeps its COM port open, so the session would outlive the restart and the Device would count the app as offline until the next `hello`. So `devices.restart` closes the link after sending `reboot`, and a ping whose uptime went backwards closes it too. Both start a fresh probe under a new boot grace.
- A Device found by `hello` is `setup: "ready"`. `needs-setup` is for a New board that just got Firmware (#77). Known Devices and their names are saved in `devices.json`; at start they load as `offline`.

```rust
pub struct DeviceSummary {
    pub id: DeviceId, pub rev: u64,
    pub name: Option<String>,                  // None until named
    pub setup: SetupState,                     // NeedsSetup | Ready
    pub link: LinkState,
    pub carrier: Option<Carrier>,              // usb | wifi while connected, else null; see Wi-Fi Carrier
    pub firmware: Option<FirmwareInfo>,        // {version, protocol, build, recovery, update_available: Option<String>, channel: "normal" | "radio"}
    pub diagnostics: Option<DeviceDiagnostics>,
    pub capabilities: Option<Capabilities>,    // v2 hello: screen, storage_free, tile_kinds, live_hz, live_total_hz, series, text_bytes. None on v1
    pub orientation: Option<Orientation>,      // the shown Layout's; null whenever `layout` is
    pub layout: Option<LayoutSummary>,         // what the Device shows; see below
    pub control_layout: ControlLayout,
    pub key_mirror: KeyMirrorState,            // {enabled, reason}
    pub stable_id: bool,
}
pub struct LayoutSummary {
    pub source: LayoutSource,                  // Builtin | Storage
    pub name: String,
    pub orientation: Orientation,
    pub pages: u32,                            // Page count
    pub page: Option<u32>,                     // index of the Page on screen
}
pub struct NewBoard { pub id: DeviceId, pub port: String, pub probe: BoardProbe, pub seen_ms: u64 }
pub enum BoardProbe { Silent, Busy { detail: String }, OtherFirmware { product: String, detail: String } }
```

`layout` is what the Device shows, from the Layout the live router reads from it
(below, under Rules for several Devices) and the Page the Device reports with
`v2.page` ([link v2](../schema/protocol/link-v2.md#page-on-screen)). `source` is
`builtin` when the Device runs its built-in Layout because none is stored for its
orientation, and `storage` otherwise. `name`, `orientation` and `pages` describe
that Layout. `page` is the index in `pages` of the Page on screen; it is null
until the Device reports a Page of this Layout, and always on Firmware that
doesn't send `v2.page`. `layout` is null on protocol 1, while offline and until
the first read of a session completes, which is within one live round of
connecting. A link change clears it. A Layout push or orientation change keeps
the previous summary until the next read lands, so it never goes null mid-push.
`orientation` is the orientation of that Layout, so it is null whenever `layout`
is. Changes increment `rev` and publish `device.updated`. `name` is the Device's
own and untrusted, like the strings below.

`diagnostics` is null until the Device has answered `hello`. Otherwise it is
`{ product: string, running_from: string, sinks: string[], current_layout: string }`,
from `hello.product`, `hello.running`, `hello.sinks` and `hello.layout`. `current_layout`
is the Firmware's Layout source as `hello` reports it, separate from `DeviceSummary.layout`.
Changes increment `rev` and publish `device.updated`. These strings, and the Firmware's
`version` and `build`, are the Device's own and untrusted. The API returns them as sent,
and clients show control, format and line or paragraph separator characters as escapes.

### Wi-Fi Carrier

A Device reaches the Companion over USB or Wi-Fi, one Carrier at a time, under one `DeviceId`. Wi-Fi uses net-v2 ([net-v2.md](../schema/protocol/net-v2.md)): the person pairs once with the code the Device shows, and each session after that is authenticated and encrypted.

- **One Device on both Carriers.** net-v2 pins the factory MAC, which is also the USB serial, so a Device paired over Wi-Fi has the same id as over USB. `DeviceSummary.carrier` is `usb` or `wifi` while connected and `null` otherwise. While connected over Wi-Fi, `link.port` is the `host:port` the Companion reached.
- **USB wins.** The scan thread still probes the USB port of a Device linked over Wi-Fi. When it answers, the USB link replaces the Wi-Fi one. When the port leaves, the Device goes `lost` with `unplugged` and the Wi-Fi thread tries it at once, so it comes back as `connected` over Wi-Fi without being forgotten.
- **The Wi-Fi thread.** It tries each paired Device that has no USB port, isn't linked and wasn't disconnected by the person: at once after pairing, a Wi-Fi link loss, an unplug or `devices.connect`; then after 0.25 s, doubling up to 10 s. A Device that answers `busy` (linked over USB or to another app) is `busy` with the address as `port` and is tried again at most every 2 s. A failure leaves the Device `offline` and records `problem` on its Pairing. Two failures stop the retries until the person connects again (`devices.connect`) or plugs the Device in: a `version` refusal, and a Device whose `hello` doesn't offer link-v2 or doesn't carry the paired id. Both mean its Firmware can't take this Companion over Wi-Fi, so `problem` says to update it over USB. The Companion never falls back to anything weaker. An `auth` refusal never deletes the key: `problem` says the Device no longer knows this Companion, and the person can forget the Pairing and pair again.
- **Discovery** runs only when `wifi.discover` asks: a 3 s DNS-SD browse for `_touchdeck._tcp` with the pure-Rust `mdns-sd` crate, which needs neither Bonjour nor Avahi and works beside an OS responder. Records are hints; the Pairing pins the id. A browse returns at most 32 Devices, drops loopback, link-local, unspecified, broadcast and multicast addresses, drops every character from a name that net-v2 refuses in a name, and ignores a `paired` count above 4. Where multicast is blocked it finds nothing, and the person types the address.
- **What a record says.** Outside pairing mode a Device's record names nothing: it carries a random nonce and four 8-byte hints ([Discovery privacy](../schema/protocol/net-v2.md#discovery-privacy)). The Companion computes each of its Pairings' hints for that nonce, with a discovery key derived from the Pairing's key in the keychain and then kept in memory only, and a match is that paired Device: `paired_here: true`, with the name the Companion knows. A Device in pairing mode also gives its id, name and Pairing count, and is listed with `pairing: true`. Any other record is someone else's Device, and `wifi.discover` leaves it out.
- **A paired Device at a new address**, as after a DHCP change, is tried there next. The new address replaces the Pairing's only once a session there works; if that try fails, the Wi-Fi thread goes straight back to the address that last worked. So a record someone replays from elsewhere costs one failed try.
- **Full Devices.** `wifi.pair_start` reads the `paired` count the last discovery saw at that address. Only a Device in pairing mode advertises a count. With 4, a `full` or `closed` refusal is 409 `wifi.full` with `details.paired: 4` and a message that says to remove one on the Device (paired apps screen) or over USB. With fewer, `full` means the Device still holds this Companion's own Pairing, so it is 409 `wifi.already_paired`. With no count, `wifi.full` says it may be either.
- **Records and unknown outcomes.** A session carries at most 2^24 records per direction. Once fewer than 1024 remain in either direction, the Companion ends the session before the next request, never in the middle of a transfer, and the Wi-Fi thread reconnects with fresh keys. A request whose session ends before its reply fails with 502 `device.unknown_outcome`: the Device may or may not have acted, so the person checks before trying again.
- **Keys.** The long-term key of each Pairing is in the OS keychain through the `keyring` crate: Credential Manager on Windows, the Secret Service on Linux, under service `touchdeck` and user `net-pairing:<device>`. With no keychain, pairing fails with 503 `wifi.keychain` and a message that says what to install or unlock; the key is never written to a file. `network.json` in the data folder holds the Companion id and each Pairing's Device id, address, name and whether it's confirmed, never a key.
- **Over Wi-Fi a session has no console.** `console.send` and `devices.restart` fail with `device.link`. Firmware updates need USB: `firmware.update` and `firmware.repair` give 409 `firmware.usb_only` for a Device connected over Wi-Fi. The Device refuses USB-only requests over a network session with `usb-only`.

```rust
pub enum Carrier { Usb, Wifi }                 // "usb" | "wifi"
pub struct WifiDevice { pub device: DeviceId, pub name: String, pub address: String, pub paired: Option<u8>, pub paired_here: bool, pub pairing: bool }
pub struct WifiPairing { pub device: DeviceId, pub address: String, pub name: Option<String>, pub confirmed: bool, pub paired_ms: u64, pub problem: Option<String> }
pub struct PairingStarted { pub attempt: String, pub device: DeviceId, pub address: String, pub remaining_ms: u64, pub attempts_left: u8 }
```

### Rules for several Devices

`control_layout` describes the active orientation's Control authorization. It is a union tagged by `state`:

- `unavailable`, with `reason`: `checking`, `offline`, `unsupported`, or `read-failed`.
- `no-pushed-copy`, `matches`, or `differs`, each with `orientation` and `has_controls`.

Missing, unreadable, and invalid pushed copies produce `no-pushed-copy`. `has_controls` checks all Pages using the touch path's renderer parts and settable numeric bar input rules. The built-in Layout has no controls. `matches` compares canonical Layout bytes with this Companion's pushed copy; it does not promise an Integration is currently available. A touch still binds only against a pushed copy.

Startup, link loss, a new session, and Layout/orientation replacement reset the assessment to `unavailable/checking`. A push writes the pushed copy before it releases the link, so a successful push goes from `checking` straight to `matches`, and a failed push leaves the pushed copy as it was and ends in a fresh assessment. An orientation change resets the assessment before `SettingsPut` and again once it is sent, even when a later step of the change fails. Subsequent live rounds report offline or unsupported Devices, read failures, or the comparison after reading the active Layout. A failed read is retried after 5 seconds, or at once after a push or orientation change. Every 5 seconds the Companion also checks the active Layout, with or without live values: one `layout.get` of the stored Layout the Device shows, or of both stored Layouts while it shows the built-in one. It reads the active Layout again only when a stored Layout changed, so a change made outside this Companion shows within about 5 seconds. Every assessment rereads the pushed copy, so a deleted or corrupt file is noticed at the next one. Changes increment the Device's `rev` and publish the full object through `device.updated`; GET Devices and `sync` expose that same object. Old sessions and rounds invalidated by a replacement cannot publish a stale assessment.

- Each Device has its own link session, Layout, key mirror, action resolution and Job slot. Its actor thread owns them, and the API reads `DeviceSummary` from that owner.
- A Job on one Device never blocks another Device.
- Approvals and action-log entries carry `device`.
- Integration values go to every connected Device whose Layout binds them. Integration status reports delivery per Device.

## Event stream

### Frames

```jsonc
{ "type": "hello", "seq": 0, "api": "v1", "build": "<sha>", "session": { "id": "…", "kind": "window" } }
{ "type": "sync",  "seq": 1, "data": { "devices": [], "new_boards": [], "approvals": [], "jobs": [], "integrations": [] } }
{ "type": "device.updated", "seq": 2, "device": "a0b1c2d3e4f5", "data": { /* full DeviceSummary */ } }
```

- `seq` counts up per connection, for debugging and gap detection.
- Feature snapshots fill the `sync` keys. On every reconnect the client replaces its state from `sync`. There's no replay buffer and no client-side merge.
- State events carry the full object (`DeviceSummary`, `Job`, `ApprovalRequest`), not a patch. Each object has a `rev` counter, and the client ignores an older `rev`. Applying an event twice or out of order is harmless.
- If `hello.build` differs from the build the page loaded with, the page reloads.
- Clients ignore event types they don't know.

### Topics

The client sends `{"op": "subscribe", "topic": "console", "device": "a0b1c2d3e4f5"}` or the same with `"op": "unsubscribe"`. Each socket keeps its own set of topic and Device pairs. A request with an unknown op or topic, an invalid Device id, or the `state` topic changes nothing, and the socket stays open. There's no reply, so a client that needs the first frame retries until it arrives.

| Topic | Default | Carries |
|---|---|---|
| `state` | Always on | `device.added`, `device.updated`, `device.removed`, `new-board.added`, `new-board.removed`, `job.updated`, `approval.requested`, `approval.resolved`, `action.logged`, `integration.status`, `auth.revoked`, `companion.stopping` |
| `console` | Opt-in, per Device | `device.console` |
| `key-mirror` | Opt-in, per Device | `device.key-mirror` |
| `device-events` | Opt-in, per Device | `device.event`, the raw Device event, for diagnostics |

### Event catalogue

| Type | Topic | Payload |
|---|---|---|
| `hello` | (first frame) | `{api, build, session: {id, kind}}` |
| `sync` | (second frame) | `SyncData`: one key per feature snapshot |
| `device.added` | `state` | `DeviceSummary` |
| `device.updated` | `state` | `DeviceSummary` |
| `device.removed` | `state` | `{id}` |
| `new-board.added` | `state` | `NewBoard` |
| `new-board.removed` | `state` | `{id}` |
| `job.updated` | `state` | `Job` |
| `approval.requested` | `state` | `ApprovalRequest` |
| `approval.resolved` | `state` | `{id, answer: "once" \| "always" \| "deny" \| "expired"}` |
| `action.logged` | `state` | `ActionLog` |
| `integration.status` | `state` | `IntegrationSummary` |
| `auth.revoked` | `state` | `{}`, followed by close 4401 |
| `companion.stopping` | `state` | `{reason}`, followed by close 1001 |
| `device.console` | `console` | `{device, line}` |
| `device.key-mirror` | `key-mirror` | `{device, seq, report, outcome, gap, errors, arrived_ms}` |
| `device.event` | `device-events` | `{device, event}` |
| `lagged` | (any opt-in topic) | `{topic, device, dropped}` |

### Backpressure

Each socket has a queue of at most 1024 frames or 4 MiB, whichever comes first (see Limits).

- Opt-in topics drop frames on overflow and send `lagged`.
- If a `state` frame would overflow, the server closes with 4008. The client reconnects and gets a fresh `sync`, so state stays correct and a slow tab never stalls a Device.

The server pings every 20 s and drops a client after 2 missed pongs.

## Jobs

**Rule.** An operation that transfers files to a Device, flashes it, or can take longer than about 2 s is a Job. Restart and settings aren't Jobs: they're one request and reply on the link.

```rust
pub struct Job {
    pub id: JobId,                 // "job_" + 16 random chars
    pub kind: &'static str,        // see the table below
    pub scope: JobScope,           // Device(DeviceId) | NewBoard(DeviceId) | Companion
    pub state: JobState,           // queued | waiting_for_confirmation | running | succeeded | failed | cancelled
    pub progress: Value,           // typed per kind in TypeScript
    pub result: Option<Value>,     // typed per kind
    pub error: Option<ErrorBody>,  // the same shape as an HTTP error
    pub cancellable: bool,         // whether the current phase allows DELETE
    pub created_ms: u64, pub finished_ms: Option<u64>, pub rev: u64,
}
```

| Kind | Progress | Result | Cancellable |
|---|---|---|---|
| `firmware.update`, `firmware.repair`, `firmware.install` | `{phase: "checking" \| "sending" \| "rebooting" \| "confirming" \| "recovering", pct?, bytes}`, where `bytes` is how many bytes the phase has written so far, `0` when it has no `pct` | `{message, version}` | While waiting for confirmation; during `checking` or `sending` only when the image goes to the inactive OTA slot |
| `layouts.push` | `{assets_sent, assets_total, bytes_sent, bytes_total}` | `PushReport {state, bytes, chunks, retransmits, elapsed_ms, assets: {sent, unavailable}}` | Before the Layout put starts |
| `assets.render` | `{frames_done, frames_total}` | `AssetRef {id, kind, width, height, bytes}` | Yes |
| `assets.prune` | `{removed, total}` | `{removed: AssetId[], kept_in_use: number}` | Yes |
| `icon_packs.install` | `{stage: "download" \| "verify" \| "extract" \| "done"}` | `PackInfo` | Yes, between stages |

- **One active Job per Device or New board scope**, including `waiting_for_confirmation`. A second one gets 409 `device.busy` with `details.job` set to the active Job, so a double click can attach to it. There's no queue. Companion-scoped Asset render Jobs instead share the two-Job limit in Limits.
- `GET /jobs` lists active Jobs and the last 50 finished ones, kept for 10 minutes. `GET /jobs/{id}` reads one. `DELETE /jobs/{id}` cancels one, or returns 409 `job.not_cancellable`.
- In TypeScript, `companion.jobs.wait(job)` resolves with the typed `result` on `succeeded` and rejects with the `ApiError` on `failed`. It follows `job.updated` events and re-reads `GET /jobs/{id}` after a reconnect.
- **As built in #75.** The generated `companion.jobs.wait<K>(job)` reads `GET /jobs/{id}` every 250 ms until the Job finishes; the UI's event client can resolve it from `job.updated` instead. A cancelled Job ends in `cancelled` with the error `job.cancelled`, and `wait` rejects with it. A Companion-scoped kind at its limit gets 429 `limits.exceeded`, not a queue.

Feature module `jobs`.

| Name | Method and path | Request → Response |
|---|---|---|
| `jobs.list` | `GET /jobs` | → `Job[]`: active Jobs, then the finished ones kept, oldest first. Also the `jobs` key of `sync` |
| `jobs.get` | `GET /jobs/{job}` | → `Job`. 404 `job.not_found` |
| `jobs.cancel` | `DELETE /jobs/{job}` | → `Job`, with its current state. The Job reads `cancelled` once its work notices. 409 `job.not_cancellable` when the phase or a finished Job forbids it |
- **Crash halfway.** Jobs aren't saved. OTA rollback covers a Firmware Job lost mid-flash: the Device reconnects on its old or new build, and the UI shows its version. A lost Layout push leaves the Device's previous Layout, because the put is atomic on the Device (`xfer.end`).
- **Uploads aren't Jobs.** `POST /uploads` returns an id after upload validation; `POST /firmware/bundles/pick` returns after the native file dialog closes, and Jobs then refer to that id.

## Routes

Paths are relative to `/api/v1`. Every request and response type is a Rust type with `#[derive(Serialize, Deserialize, TS)]`, generated into `api.gen.ts`. `→ ()` means `204 No Content`. `→ 202 Job` means the response is a `Job` and the work continues as `job.updated` events.

### Devices, ports and console

Feature modules `devices`, `ports` and `console`.

| Name | Method and path | Request → Response |
|---|---|---|
| `devices.list` | `GET /devices` | → `DeviceSummary[]` |
| `devices.get` | `GET /devices/{device}` | → `DeviceSummary` |
| `devices.connect` | `POST /devices/{device}/connect` | → `DeviceSummary`. Turns auto-connect back on after a Disconnect |
| `devices.disconnect` | `POST /devices/{device}/disconnect` | → `DeviceSummary` with `link.state: "disconnected"`. Releases the port until replug or Connect |
| `devices.forget` | `DELETE /devices/{device}` | → `()`. Only for a Device that isn't connected; a connected one gives 409 `device.connected` |
| `devices.restart` | `POST /devices/{device}/restart` | → `()`, then `device.updated` events |
| `devices.settings_get` | `GET /devices/{device}/settings` | → `DeviceSettings {idle_timeout_s, orientation, name?, brightness?, photo_frame?}`. v2 fields are `null` on v1 Firmware. `brightness` is a percentage from 0 to 100. `photo_frame` is `PhotoFrameSettings`, below, and `null` on Firmware without the photo frame |
| `devices.settings_patch` | `PATCH /devices/{device}/settings` | `DeviceSettingsPatch`, every field optional → `{settings, restart_required}`. A v2 field on v1 Firmware gives 409 `device.needs_firmware`. A `brightness` above 100 gives 400 `request.invalid` first. `photo_frame` takes `PhotoFramePatch`, below |
| `devices.wifi_get` | `GET /devices/{device}/wifi` | → `DeviceWifi`, below |
| `devices.wifi_set` | `PUT /devices/{device}/wifi` | `{ssid, password}` → `DeviceWifi`, usually `connecting`. Stores the network on the Device and starts joining it |
| `devices.wifi_forget` | `DELETE /devices/{device}/wifi` | → `DeviceWifi` with `state: "off"`. Erases the network on the Device |
| `devices.bluetooth_get` | `GET /devices/{device}/bluetooth` | → `DeviceBluetooth`, below |
| `devices.bluetooth_set` | `PUT /devices/{device}/bluetooth` | `{enabled}` → `DeviceBluetooth`. Stores the setting on the Device, then starts or stops Bluetooth |
| `devices.bluetooth_pairing_open` | `POST /devices/{device}/bluetooth/pairing` | → `DeviceBluetooth`. Opens the pairing window for 120 seconds |
| `devices.bluetooth_pairing_close` | `DELETE /devices/{device}/bluetooth/pairing` | → `DeviceBluetooth`. Closes the pairing window |
| `devices.bluetooth_forget` | `DELETE /devices/{device}/bluetooth/bonds/{address}` | → `BluetoothForgotten {forgotten, bluetooth}`. Forgets the computer or phone at `aa:bb:cc:dd:ee:ff`, in either case; `forgotten` is 0 when the Device kept no such bond |
| `devices.bluetooth_forget_all` | `DELETE /devices/{device}/bluetooth/bonds` | → `BluetoothForgotten {forgotten, bluetooth}`. Forgets every computer and phone |
| `new_boards.list` | `GET /new-boards` | → `NewBoard[]` |
| `ports.list` | `GET /ports` | → `PortProbe[]`, today's `Candidate` plus `usb_serial`. Never opens a port that a link holds |
| `ports.rescan` | `POST /ports/rescan` | → `()` |
| `console.send` | `POST /devices/{device}/console` | `{line}` → `()`. One line, at most 256 printable ASCII bytes, allowlist below |

`PhotoFrameSettings` is `{enabled, timeout_s, interval_s, folder}`. The Device keeps the first three, from [link-v2.md](../schema/protocol/link-v2.md) "Photo frame, idle, and clock": `enabled` turns photo idle mode on, `timeout_s` is the idle time before the photo frame or clock shows, from 10 to 86400, and `interval_s` is the time between pictures, from 5 to 3600. The Companion keeps `folder` per Device in `photo-frame.json` in its data folder: the full path of a folder on this computer, or `null`. `PhotoFramePatch` has the same fields, each optional. A `folder` of `""` clears it. A time out of range, or a folder that is not the full path of an existing folder, gives 400 `request.invalid`. Firmware without `photo-frame` and `clock` in `caps.tile_kinds` and a nonzero `caps.photo_bytes` gives 409 `device.needs_firmware` with `details.field` `photo_frame`.

The Wi-Fi routes work only on a Device whose Firmware has a Wi-Fi station, the radio build, which says so with `capabilities.wifi: true`. Any other Device gives 409 `device.needs_firmware` with `details.field` `wifi`, and the Companion sends it nothing. They follow [link-v2.md](../schema/protocol/link-v2.md) "Wi-Fi network", which takes them only over USB. `DeviceWifi` is `{configured, ssid, state, reason, ip, rssi}`: `state` is `off`, `connecting`, `connected` or `failed`, and `reason` is the Device's word for a failure, such as `not-found` or `wrong-password`. Poll `devices.wifi_get` while it says `connecting`. `ssid` is 1 to 32 UTF-8 bytes; `password` is empty for an open network, 8 to 63 printable ASCII characters, or 64 hex digits. Anything else gives 400 `request.invalid` before the Device hears of it, and a body that doesn't parse gets one fixed reason, so no part of it comes back. The password goes to the Device and nowhere else: no response, event, log line or `Debug` output carries it. A Device connected over Wi-Fi takes its network only over USB: the Companion refuses with 409 `device.usb_only` and sends it nothing. Pairing with this app over that network is the Wi-Fi Carrier, above.

The Bluetooth routes work only on a Device whose Firmware has the Bluetooth keyboard, the radio build, which says so with `capabilities.ble_hid`, the most computers and phones it keeps bonded. Any other Device gives 409 `device.needs_firmware` with `details.field` `bluetooth`, and the Companion sends it nothing. They follow [link-v2.md](../schema/protocol/link-v2.md) "Bluetooth keyboard (radio build)", which takes them only over USB: a Device connected over Wi-Fi gives 409 `device.usb_only`, and the Companion sends it nothing. `DeviceBluetooth` is `{enabled, state, pairing, pairing_ms, passkey, advertising, bonds, bonds_max, battery}`. `enabled` is the stored setting, off on a fresh Device. `state` is `off`, `starting`, `on` or `failed`; poll `devices.bluetooth_get` while it says `starting`. `pairing_ms` counts down the open window, and `passkey` says whether the Device shows the six-digit code a computer or phone types to pair; the code itself never leaves the Device. `advertising` is `off`, `pairing`, `fast` or `slow`. `bonds` lists `{address, random, connected}` for each computer or phone. The Device keeps them in every state, and the list may come in any state: the Firmware lists none while `off`, and still lists them while `starting` after a host reset. An address in the path may be in either case; the Device hears it in lower case. `battery` is the level the Device reports to hosts, fixed at 100. A body other than `{enabled: boolean}` or an address other than six hex bytes gives 400 `request.invalid` before the Device hears of it. The Device's own refusals keep their reason: 409 `bluetooth.off` (Bluetooth is off, so there is no window to open and no bond to forget), 409 `bluetooth.starting` (ask again), 409 `bluetooth.full` (`bonds_max` computers and phones are bonded; forget one first), 502 `bluetooth.radio` (Bluetooth did not start; the setting stays on) and 502 `bluetooth.storage` (the Device couldn't save the change). Each of them carries the state after the request in `details.bluetooth` when the Device sent it.

While a Device is connected with the photo frame on and a folder set, the Companion sends it one picture per interval while it is idle. It reads JPEG and PNG files at the folder's top level only, skips files it cannot read or decode, follows no links, and fits each picture to the screen as a JPEG within `caps.photo_bytes`. It sets the Device's clock on connect, every 10 minutes, and when the time zone changes.

The Companion owns this exact allowlist, derived from registrations and handlers under `firmware/components`. A line must contain 1 to 256 bytes in `0x20..0x7e`; CR, LF, tabs, DEL and all other control or non-ASCII bytes are forbidden. Split on ASCII spaces, require the exact command, subcommand and argument count below, and reject quotes, escapes and command separators. Anything else returns 403 `console.forbidden` before reaching the Device.

| Allowed form | Firmware handler source under `firmware/components` |
|---|---|
| `help`, `version`, `heap`, `uptime` (no arguments) | `td_console/td_console.c` |
| `link state` | `td_link/td_link_cmds.c` |
| `ui state`, `ui tile <id>`, `ui widget <tile>`, `ui dashboard <page>`, `layout state` | `ui/ui_cmds.c`, `ui/ui_layout_cmds.c` |
| `power state` | `power/power_cmds.c` |
| `recovery state` | `td_store/td_recovery_cmds.c` |
| `update state` | `td_update/td_update_cmds.c` |
| `settings get`, `store stat`, `store stat <path>`, `store ls <path>`, `store get <path>` | `td_store/td_store_cmds.c` |
| `backlight` (no arguments), `screenshot`, `screenshot 1`, `screenshot 2`, `screenshot 4` | `display/display_cmds.c` |

Each placeholder is one unquoted token. `<path>` is validated as a storage path by the Firmware; `<id>`, `<tile>` and `<page>` name existing Tiles or Pages. Only these read-only forms pass; the allowlist never grants an entire command family. All `keys`, `touch` and `perf` commands, mutating `ui` subcommands, storage and NVS writes, Layout changes and reboot commands are excluded. New Firmware commands are denied until explicitly reviewed and added here. The e2e harness uses `test.console` for the full console, only under `test-hooks`, on mock ports or with `--test-real-device-console`.

### Wi-Fi

Feature module `wifi`. See Wi-Fi Carrier above.

| Name | Method and path | Request → Response |
|---|---|---|
| `wifi.discover` | `POST /wifi/discover` | → `WifiDevice[]`, after a 3 s browse: this Companion's paired Devices it recognised, and Devices in pairing mode. 503 `wifi.discovery` when mDNS can't start |
| `wifi.pairings_list` | `GET /wifi/pairings` | → `WifiPairing[]` |
| `wifi.pair_start` | `POST /wifi/pairings` | `{address}` → `PairingStarted`. `address` is an IPv4 or IPv6 address or a host name, with an optional port that defaults to 47820. The Device must have its pairing window open (Pair new app on the Device), or 409 `wifi.window_closed`. 502 `wifi.unreachable`, 409 `wifi.full`, `wifi.already_paired`, `wifi.busy` or `wifi.version` |
| `wifi.pair_code` | `POST /wifi/attempts/{attempt}/code` | `{code}`, the six digits on the Device → `WifiPairing`, and the Device then connects over Wi-Fi. A wrong code gives 422 `wifi.wrong_code` with `details.attempts_left`; the third gives 429 `wifi.locked`, and the Device refuses pairing for 30 s, doubling. 409 `wifi.expired` when the window closed, 503 `wifi.keychain`, 404 `wifi.attempt_not_found` |
| `wifi.pair_cancel` | `DELETE /wifi/attempts/{attempt}` | → `()`. Closes the window on the Device, which counts it as a wrong code, as net-v2 does any window dropped before the code is right |
| `wifi.pairings_forget` | `DELETE /wifi/pairings/{device}` | → `()`. Deletes the key and closes a Wi-Fi link. The Device keeps its side until it's forgotten there. 404 `wifi.not_paired`. `devices.forget` also forgets the Pairing |

A started pairing holds one connection to the Device until its code is right, it's cancelled, the window closes or 130 s pass. The Device shows the Pairing as "Open TouchDeck on" and the computer's name, without the characters net-v2 refuses in a name, cut to 64 bytes.

### Layouts

Feature module `layouts`.

| Name | Method and path | Request → Response |
|---|---|---|
| `layouts.get` | `GET /devices/{device}/layouts/{orientation}` | → `{layout: Layout, revision}`. `revision` is the Device's CRC32 from `GetReport`, also sent as `ETag`. It reads the Device, so a Device that isn't connected gets 409 `device.not_connected` even when the Layout store holds a copy, which has no revision |
| `layouts.last_seen` | `GET /devices/{device}/layouts/{orientation}/last-seen` | → `{layout: Layout}`, the copy of the Device's Layout this PC last saw, from the Layout store. It works whether or not the Device is connected and never reads the Device, so the copy may be stale. It has no revision and no `ETag`, so it can't back an `If-Match`; read `layouts.get` before a push. 404 `layout.not_found` when the store holds no copy for that orientation, 404 `device.not_found` for an unknown Device. Home draws an offline Device's miniature from it |
| `layouts.push` | `PUT /devices/{device}/layouts/{orientation}` | `Layout` as typed JSON, optional `If-Match: <revision>` → 202 Job `layouts.push`. 422 `layout.orientation_mismatch` when the path and body disagree. 412 `layout.changed` when the revision moved |

The `Layout` body extractor runs `td_core::Layout` validation once. On failure it returns 422 `layout.invalid` with `details.issues: [{path, message}]`.

As built in #75:

- **Revision.** The CRC-32 of the stored bytes as 8 lowercase hex digits, such as `"1c291ca3"`. `ETag` sends it quoted. `PushReport.revision` gives the pushed Layout's revision, so the next push can send it without a read.
- **`If-Match`.** Quoted or bare revisions, comma-separated, or `*` for any stored Layout. The Companion reads the Device's revision before it accepts the push, and the Job reads it again before the put. A mismatch before the Job is 412 `layout.changed` with `details.revision` (the current one, or `null` for an empty slot). A mismatch the Job finds fails it with the same error.
- **An empty slot.** `layouts.get` returns 404 `layout.not_found`.
- **Issues.** Each issue also has `rule`, the rule key from `schema/README.md`. `path` is the JSON pointer of the value the rule refused, which the `td_core` validator records with the error, such as `/pages/0/tiles/3/action/program`. It's empty when the rule names no single value, such as `tile.overlap` or `id.duplicate`. Clients find the refused field from `path`. `message` is for people, and its wording can change.
- **The Layout store.** The Companion keeps each Device's Layouts as `layouts/<device>/<orientation>.json` under its data folder, in canonical form. A Layout lands there after a successful push or read. `layouts.last_seen` reads it back. The old app kept no Layout on the PC, so nothing is migrated.
- **Sessions.** Layout and Asset work runs on the session the Device manager keeps for each Device (#74), one operation at a time per Device, through `link::Links::with`. The port is never opened twice. The actor's own requests can interleave with it, since a session matches replies to requests by sequence number.
- **Assets on push.** The Job sends the icons the Layout names that the Device lacks over today's v1 `icon.*` messages, then puts the Layout. #93 replaces that step with the v2 Asset store.

### Firmware

Feature module `firmware`.

| Name | Method and path | Request → Response |
|---|---|---|
| `firmware.bundled` | `GET /firmware/bundled` | → `{version, build, radio: {version, build} \| null}` of the normal build shipped with the Companion, and of the radio build when it ships one |
| `firmware.pick` | `POST /firmware/bundles/pick` | `{device}` → `{bundle: BundleInfo {id, version, build, bytes, channel} \| null}`. Opens a native file dialog on the PC; single-use, target-bound, expires after 10 minutes |
| `firmware.update` | `POST /devices/{device}/firmware/update` | `{bundle: "bundled" \| BundleId, channel?}` → 202 Job `firmware.update`. 409 `firmware.channel_mismatch` when the bundle is not the build asked for |
| `firmware.repair` | `POST /devices/{device}/firmware/repair` | `{bundle}` → 202 Job `firmware.repair`, through the ROM bootloader |
| `firmware.install` | `POST /new-boards/{device}/install` | `{bundle}` → 202 Job `firmware.install`. Waits for native PC confirmation. 404 `new_board.not_found` when no New board has that id |

While a Firmware Job holds a Device, its `link.state` is `updating`.

### Assets

Feature module `assets`. Assets reach a Device through `layouts.push` and the internal live-cover delivery path. Neither path exposes arbitrary local files.

| Name | Method and path | Request → Response |
|---|---|---|
| `assets.builtin` | `GET /assets/builtin` | → `Builtin[] {name, label, tags, id}` |
| `assets.suggest` | `POST /assets/builtin/suggest` | `{action: TileAction}` → `Builtin \| null` |
| `assets.file` | `GET /assets/{asset}` | → the stored bytes with their media type: `application/vnd.touchdeck.tdi1`, `image/gif` or `image/png` |
| `assets.create` | `POST /assets` | `{source: {kind: "app", path} \| {kind: "upload", upload} \| {kind: "catalog", catalog, id} \| {kind: "brandfetch", brand, logo_type, theme, padding, background, tint}, fit}` → 202 Job `assets.render`. For `app`, the Companion reads only the icon resource of that program |
| `assets.upload` | `POST /uploads` | raw `image/*` (PNG, JPEG, GIF or SVG), at most 16 MiB → `{id, media_type, bytes}`, kept for 1 hour |
| `assets.catalog_search` | `GET /assets/catalog/search?q=&catalog=` | → `CatalogHit[]`, at most 500. Searches the bundled catalogs and installed Icon packs offline; catalog images are fetched on pick |
| `assets.on_device` | `GET /devices/{device}/assets` | → `{ids: AssetId[], storage_free}` |
| `assets.prune` | `POST /devices/{device}/assets/prune` | `{also_keep: AssetId[]}` → 202 Job `assets.prune`. Removes Assets no stored Layout uses |

As built in #75:

- `assets.create` takes `app`, `upload` and `catalog` sources. Uploads accept PNG, JPEG, GIF and SVG images. An SVG is drawn with nothing outside the file: an `<image>` draws nothing, whether it names a file, a URL or a `data:` image, and `currentColor` without a root `color` draws in the built-in icon ink `#f2f5fa`. An uploaded SVG with a `DOCTYPE` or `ENTITY` declaration, or gzip-compressed, gets 400 `request.invalid`. GIF headers are admitted without decoding; full GIF validation runs in the preparation Job. An expired, consumed or foreign upload returns `upload.not_found`. Catalog picks accept `variant` (`base`, `light`, `dark`), pixel `padding`, RGBA `background` and RGB `tint`. `fit` is `"contain"` (the default: the whole image inside the Tile) or `"cover"` (cropped to a centered square first). The result's `kind` is `tdi1`, 64 by 64, identified by the full SHA-256 of its encoded bytes. A third render while two run gets 429 `limits.exceeded`.
- `assets.prune` keeps the Assets that `also_keep` names, that the Companion's stored Layouts for the Device use, and that the Device's own stored Layouts use, as today's `remove_unused_icons` does. It also keeps live cover Assets the Companion last sent successfully or whose reply is still unanswered. Obsolete cached covers may be removed and are forgotten by the upload cache. A Device delete refusal with reason `in-use` keeps that Asset and continues; other delete errors still fail the Job. The result's `kept_in_use` counts distinct listed Assets protected as live covers or refused `in-use`, including on repeat runs. It excludes Assets kept only by Layouts or `also_keep`.
- `assets.file` reads built-in icons and the Companion's Asset files, kept as `assets/<id>.bin` under its data folder. The generated client returns a `Blob` for it.
- A `brandfetch` source renders a logo through the Brand API with the user's own key; see [Brandfetch](#brandfetch).

### Brandfetch

Feature module `brandfetch` (#127). Brand logos from [Brandfetch](https://brandfetch.com), with the user's own keys. Open TouchDeck ships no client ID and no key. Nothing reaches Brandfetch until the user saves keys in Settings, Brandfetch logos.

| Name | Method and path | Request → Response |
|---|---|---|
| `brandfetch.get` | `GET /brandfetch` | → `BrandfetchStatus {client_id, key_set, logos, need_refresh, expired, replaced}` |
| `brandfetch.client_id_put` | `PUT /brandfetch/client-id` | `{client_id}` → `BrandfetchStatus`. 1 to 128 of letters, digits, `-`, `_`, `.`; else 400 `request.invalid` |
| `brandfetch.client_id_delete` | `DELETE /brandfetch/client-id` | → `BrandfetchStatus`. Turns search off |
| `brandfetch.key_put` | `PUT /brandfetch/key` | `{value}` → `BrandfetchStatus`. Write-only; 1 to 512 printable ASCII characters |
| `brandfetch.key_delete` | `DELETE /brandfetch/key` | → `BrandfetchStatus`. Deletes the keychain entry |
| `brandfetch.key_test` | `POST /brandfetch/key/test` | → `BrandfetchStatus`. One Brand API call for `brandfetch.com`, which Brandfetch doesn't count. Only on request |

- **Search runs in the UI.** The client ID isn't a secret: Brandfetch puts it in every logo URL. The Companion keeps it in `brandfetch/settings.json` under its data folder and returns it. The icon picker calls `GET https://api.brandfetch.io/v2/search/{query}?c={client_id}&limit=5` straight from the page, at least 300 ms after the last keystroke, aborts a stale search, and never caches results. Thumbnails are hotlinked `<img>` elements with `referrerpolicy="origin"`, so Brandfetch sees which app shows them while the document keeps `Referrer-Policy: no-referrer`. The Companion never searches and never fetches a thumbnail.
- **The API key never comes back.** The Companion stores it in the system keychain (Windows Credential Manager on Windows, Secret Service such as GNOME Keyring or KWallet on Linux), entry `app:brandfetch:api_key` of service `touchdeck`, bound to the origin `https://api.brandfetch.io`. It's never written to a file, a log, a Layout or the provenance record. No route returns it or any part of it, only `key_set`. When no keychain is reachable, `brandfetch.key_put` fails with 503 `brandfetch.no_keychain`; there is no plain-text fallback. Its message names no platform: the UI words the problem from `system.get`'s `platform` and shows the message under Details. Mock mode (`--mock-devices`) keeps the key in memory, and only there may the Companion take `TOUCHDECK_BRANDFETCH_CLIENT_ID` and `TOUCHDECK_BRANDFETCH_API_KEY` from its environment.
- **A pick goes through the Companion.** `assets.create` with a `brandfetch` source calls `GET https://api.brandfetch.io/v2/brands/{brand}` with `Authorization: Bearer`, where `brand` is a domain or a Brandfetch brand id. The key travels only in that header, only over HTTPS (the agent refuses plain HTTP), and redirects aren't followed, so it never reaches another host. The Companion picks the requested `logo_type` (`icon`, `symbol` or `logo`), then the other types in the order icon, symbol, logo, so a brand without an icon still gives its symbol or logo. Within a type it goes by the requested `theme` (`light`, `dark` or `auto`, which prefers `light`), then prefers SVG, PNG, WebP and JPEG. It downloads the file unchanged and without the key from the `src` the answer gives, on `cdn.brandfetch.io` or `asset.brandfetch.io` only; it never builds a Logo API address itself. It tries at most 4 files, best first, and stops when the Job is cancelled. It draws an SVG by the rules for uploads (no DOCTYPE, ENTITY or compression, and no linked or embedded image is read) and a PNG, WebP or JPEG with the upload decode limits (4096 pixels a side, 64 MiB), then fits it like a catalog icon (`padding`, `background`, `tint`, `fit`) into a 64 by 64 `tdi1` Asset. A file that draws nothing fails the pick with `asset.render_failed`. Each pick uses one Brand API request; the free developer plan has about 100.
- **The 30-day rule.** Brandfetch's terms let an app keep a logo for 30 days. For each Brandfetch Asset the Companion records the brand, the options, the fetch time and the original file in `brandfetch/provenance.json` and `brandfetch/originals/`. A check runs at start and then every hour. It compares wall-clock times, so a PC that slept catches up when it wakes. From day 25 it calls the Brand API again for each due logo, at most 10 calls per run; the rest wait for the next run.
  - The same file moves the fetch time.
  - A changed file renders a new Asset, and `replaced` maps the old Asset id to the new one. A logo replaced twice maps straight to the newest. The Companion changes no stored Layout and pushes nothing: the editor swaps the new Asset into its drafts when it reads `brandfetch.get`, so the next Send to Device carries it.
  - A failure that may pass, such as no answer, tries again a day later. Used-up requests (`brandfetch.quota`) end the run.
  - A refused key stops calls for every logo until a key is saved. A brand Brandfetch no longer knows, or with no logo left, stops calls for that logo.
  - A logo whose Asset is gone from this PC is forgotten without a call.
  - A logo that goes 30 days without a successful fetch loses its cached original, is listed in `expired` and is never called again; picking it again starts over.

  Assets on a Device are never deleted. `need_refresh` counts logos at least 25 days old, expired ones included.
- **Errors** are Job errors on a pick and HTTP errors on the routes: `brandfetch.no_key` 409 (no key saved), `brandfetch.key_refused` 502 (401, 402 or 403 from Brandfetch), `brandfetch.quota` 429 (429, or 403 `no_api_credits`), `brandfetch.not_found` 404 (400 or 404, or a brand that is neither a domain nor an id), `brandfetch.no_logo` 422 (no file of that kind on an allowed host), `brandfetch.unavailable` 502 (no answer, a redirect or another status), `brandfetch.store` 500 (the settings or provenance file couldn't be written) and `brandfetch.no_keychain` 503. No message carries text from Brandfetch's answer.

### Icon packs

Feature module `icon_packs`. An Icon pack is an Iconify JSON set someone installs. Nothing is installed by default.

| Name | Method and path | Request → Response |
|---|---|---|
| `icon_packs.list` | `GET /icon-packs` | → `PackInfo[] {prefix, name, origin, installed, version, download_bytes, size_bytes, icons, license {title, spdx, url}, author {name, url}, attribution, brands, palette, samples}`: the curated packs in their offered order, then imported ones |
| `icon_packs.install` | `POST /icon-packs/{prefix}` | → 202 Job `icon_packs.install`, progress `{stage: "download" \| "verify" \| "extract" \| "done"}`, result `PackInfo`. 404 `icon_pack.not_found`, before any Job, for a prefix that isn't curated, imported packs included |
| `icon_packs.remove` | `DELETE /icon-packs/{prefix}` | → 204. Deletes the pack's folder. 404 `icon_pack.not_found` |
| `icon_packs.import` | `POST /icon-packs/import` | raw Iconify JSON as `application/json`, at most 32 MiB → `PackInfo`. 409 `icon_pack.too_many` once 20 imported packs are installed |
| `icon_packs.thumbnail` | `GET /assets/packs/{prefix}/{name}.svg` | → one icon as a standalone SVG, `image/svg+xml`, with the usual response headers except the policy: `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`. 404 `icon_pack.not_found` |

- **Curated list.** `app/crates/td-host/src/packs.json`, written by `app/scripts/generate_packs.py`, pins each `@iconify-json/<prefix>` npm package by version, tarball URL and `dist.integrity`. It offers logos, devicon, skill-icons, fa6-brands, mdi, tabler and lucide. Simple Icons is left out because it ships as a catalog.
- **Install.** The Job downloads the pinned tarball from `registry.npmjs.org` only, over HTTPS, following no redirect, and refuses it unless its SHA-512 matches the pinned integrity (`icon_pack.integrity`), and keeps only `icons.json`, `info.json` and the first license file (saved as `LICENSE`) in `<data>/packs/<prefix>/`. Nothing else in the package is unpacked or run. A tarball with a link entry, an absolute path, a drive or `:` in a path, a `..` component, a bad header checksum or no end marker is refused whole (`icon_pack.invalid`), on Linux and Windows alike. A leftover folder at `<data>/packs/<prefix>/` that holds no usable pack is replaced. Installing an installed pack succeeds without a download.
- **Validation.** Installed and imported files must be Iconify JSON with a valid prefix: lowercase letters and digits in hyphen-separated parts, so a pack's folder name means the same on case-sensitive and case-insensitive file systems. A prefix that Windows reserves for a device (`con`, `prn`, `aux`, `nul`, `com0` to `com9`, `lpt0` to `lpt9`) is refused on every OS with 422 `icon_pack.invalid`. Icons with an invalid name, an out-of-range box or a body that isn't self-contained are dropped. Each body is parsed as XML and checked against an allowlist: SVG-namespace elements only, no `script`, `foreignObject` or `style` element in any namespace, no `on*` attribute, no `xml:` attribute but `xml:space`, no DTD, every `href` must point at a `#fragment`, and after character references and CSS escapes are decoded every `url()` must too, with no `@import` or `javascript:`. Animations may not retarget an `href`. Pack and catalog SVGs may carry a `DOCTYPE` but no `ENTITY` declaration, and no embedded `data:` image is drawn. A file with no usable icon gets 422 `icon_pack.invalid`.
- **Import.** An imported pack's license reads "Custom (you provided it)". A prefix that an installed or curated pack uses gets 409 `icon_pack.exists`. At most 20 imported packs are installed at once; the next import gets 409 `icon_pack.too_many` until one is removed.
- **Search.** `assets.catalog_search` merges installed packs into the catalog ranking: exact, prefix, then substring matches on icon names and plain aliases, catalog hits first within a rank. A pack hit's `source` is `iconify:<prefix>`, its `id` is the icon name, and its `license`, `license_url` and `attribution` come from the pack. `catalog=iconify:<prefix>` searches one pack.
- **Memory.** A pack is parsed into memory on first use. Each search loads every installed pack, so they all stay loaded while the icon picker is in use. A pack not used for 5 minutes is dropped from memory, checked once a minute, so its memory is given back about 6 minutes after its last use. Its next search, thumbnail or render parses it from disk again, which takes about half a second for a pack of the largest size.
- **Render.** `assets.create` with `{kind: "catalog", catalog: "iconify:<prefix>", id: <name>}` builds a standalone SVG from the pack (box, `left`/`top` viewBox, aliases, rotation and flips) and draws it through the catalog SVG path, then padding, background and tint. Monochrome `currentColor` icons draw in `#f2f5fa`, the built-in Lucide ink.

### Actions and approvals

Feature module `actions`.

| Name | Method and path | Request → Response |
|---|---|---|
| `approvals.list` | `GET /approvals` | → `ApprovalRequest[] {id, device, page, tile, hash, kind, program, args, cwd, shell, created_ms}` |
| `approvals.show` | `POST /approvals/{id}/show` | → `()`. Shows the native dialog again. 404 `approval.not_found` if it was answered or expired |
| `actions.log` | `GET /actions/log?device=` | → `ActionLog[] {id, device, at_ms, page, tile, kind, status, detail, pid, exit_code}`, the last 100 |
| `actions.allowed` | `GET /actions/allowed` | → `AllowedAction[] {hash, kind, program, args, cwd, shell, granted_ms}` |
| `actions.allowed_remove` | `DELETE /actions/allowed/{hash}` | → `()`. The next press asks again. 404 `action.not_found` |

### Integrations

Feature module `integrations`. CLIProxyAPI is the Integration with id `cliproxyapi`. Integration and config-field ids must match `[a-z0-9_-]+` at registration and secret-store boundaries. Duplicate Integration ids are rejected.

Each stored secret records only the origin (scheme, host and effective port) it was entered for. `integrations.secret_put` binds it to the current configured origin. When `integrations.config_put` changes that origin (or adds, changes or clears a source's `tls_pin`), the Companion deletes the affected secrets before making any request under the new settings. `integrations.get` then reports `set: false`; the user must enter the secrets again. Changes to a path on the same origin keep them. A first-setup draft key becomes bound when the configuration file is successfully saved, even if vault promotion fails. Retrying the same origin may promote it; changing origin discards it and requires re-entry.

Requests carrying a credential never follow a redirect to another origin. They send no credential over plain `http` unless the host is a loopback or private-network address the user typed explicitly; a public hostname resolving to a private address is not this exception. This transport exception is a reversible policy choice. Configuration changes and in-flight requests must not forward an old origin's credential to a new origin.

| Name | Method and path | Request → Response |
|---|---|---|
| `integrations.list` | `GET /integrations` | → `IntegrationSummary[] {id, name, icon, enabled, status, config_schema, presets}`. `status` reports delivery per Device as `{device, sent, error}` |
| `integrations.get` | `GET /integrations/{integration}` | → `{summary, config, secrets: {[field]: {set}}, detail}` |
| `integrations.config_put` | `PUT /integrations/{integration}/config` | Config, checked against the Integration's schema → the same view as `integrations.get`. A config object without `enabled` keeps the stored value |
| `integrations.enabled_put` | `PUT /integrations/{integration}/enabled` | `{enabled}` → the same view as `integrations.get`. Turns the Integration on or off and leaves its other config and its secrets as stored. Any other body gives 400 `request.invalid` |
| `integrations.secret_put` | `PUT /integrations/{integration}/secrets/{field}` | `{value}` → `{hint}`. Write-only. The value never comes back and is never in `config` |
| `integrations.secret_delete` | `DELETE /integrations/{integration}/secrets/{field}` | → `()` |
| `integrations.refresh` | `POST /integrations/{integration}/refresh` | → `()` |

Polling Integrations run upstream work outside the shared registry lock, with at most one poll in flight per Integration. `integrations.refresh` schedules a poll and returns before its result is available, so the next `integrations.get` may still show the earlier result. A refresh sent while a poll runs starts one more poll as soon as that one ends; CLIProxyAPI still keeps its 15-second spacing. There is no completion event for a refresh. A finished CLIProxyAPI poll sets a new `detail.polled_ms`, increases `summary.revision` and publishes `integration.status`, so a client waits for that event or re-reads `integrations.get` until `detail.polled_ms` changes. Custom and This PC polls change Live tile values; their summary changes, and `integration.status` fires, only when the status message changes. Disabled or edited configurations, and credential changes, discard results from earlier polls. A poll that panics is dropped and runs again when due. When the last Integrations service owner is dropped, it stops scheduling and waits for active upstream calls to finish. This in-process drop can take about 160 seconds for Custom; CLIProxyAPI depends on its account count and request deadlines. Normal process exit does not join these workers, but a last-owner drop on the shutdown path can still wait for them.

CLIProxyAPI config is `{enabled: boolean, url: string, interval_s: integer}`. The URL is trimmed and the interval has a 60-second floor. Unknown fields, including `management_key`, are rejected in config. Its write-only secret field is `management_key`; the secret-write hint is the constant `Set`, including for short keys. Identical config writes preserve the polling deadline. Background polls, manual refreshes, config resets and secret writes share a 15-second minimum spacing between polling attempts.

`IntegrationSummary` also includes `streams: {id, kind, unit, settable}[]` (`DataStream.settable` defaults to false) and `actions: string[]`. Its `status` is `{state: "ok" | "error" | "not_configured", message, delivery: {device, sent, error}[]}`. Delivery entries are supplied by the Device scheduler; an Integration without a bound Device has an empty delivery list. Presets contain `{id, name, action}` with the existing Layout action shape. Live tile readings stay inside the Companion.

CLIProxyAPI `detail` is read-only and null until the first poll (and after config or secret resets). Otherwise it is `{accounts, polled_ms}`, with a Unix-millisecond poll timestamp. Each account is `{provider, account, status, success, failed, recent_10m, plan, five_hour, seven_day, error}`. `account` is the display label; `plan` and `error` are nullable strings. Each window is null or `{pct, resets_at_ms}`, with a nullable Unix-millisecond reset timestamp. Failed polls retain the last account data, with the failure in summary status. No credential, token, auth index or secret hint is included.

`IntegrationSummary.revision` increases when detail changes, including resets. The existing `integration.status` event carries this revision so clients can refetch `integrations.get`; sync carries the current summary revision.

CLIProxyAPI Preset tiles have this fixed order and copy names:

| id | name |
|---|---|
| `status` | Account status |
| `success` | Requests OK |
| `failed` | Requests failed |
| `recent_10m` | Requests 10 min |
| `five_hour_pct` | 5-hour used |
| `seven_day_pct` | 7-day used |
| `five_hour_reset` | 5-hour reset in |
| `seven_day_reset` | 7-day reset in |
| `plan` | Plan |

With `test-hooks`, mock Integrations use real HTTP only for explicitly configured `http://127.0.0.1:PORT` stub URLs. Other mock URLs retain the empty fixture transport. This permits local stub tests with mock Devices and memory-only credentials.

Each Integration's Preset tiles are in `presets`, each `{id, name, action, render?, streams?, input?}`. `PresetTile.streams` declares the renderer's named stream types; omission falls back to `{value:"number"}` for a live renderer. `input:true` enables bar dragging and is otherwise omitted. A media Preset requires its renderer in `tile_kinds`: show "Needs Firmware {x.y}" and retain the last compatible Layout if unsupported. Never downgrade an interactive Preset to a plain widget. Live readings remain internal to the Companion ([ADR 0004](adr/0004-firmware-draws-live-tiles.md)).

#### Live tile bindings

A schema 2 Tile with `streams` binds them through its `widget` action. `source` names the Integration and `account` the account, empty for an Integration with one. The Tile's `value` stream reads the Integration stream named by `field`. Any other Tile stream reads the Integration stream with the same id. A graph Tile takes a `numeric` Integration stream as `number`. A Tile with another action, or one that names an unknown Integration or stream, gets no values and shows its unavailable marker.

The Companion's live router (`td_companion::live`) reads each connected v2 Device's active Layout over the link: `layout state`, then the stored bytes, whose SHA-256 is the `layout` of every `v2.live.set`. It reads only the Integration streams that some Device's Layout binds. Integrations report typed samples through `Integration::live`. Each sample carries a counter that rises with every new sample, even one equal to the last, because a graph advances on each, and a `ttl_ms`.

- **Changes only.** The router sends a Tile stream when its Integration has a newer sample than the Device holds. It keeps only the newest sample per stream, so samples that arrive between rounds coalesce and nothing queues.
- **Pace.** Polling streams wait `max(900 ms, 1000 / live_hz)` between values; Control streams use `max(500 ms, 1000 / live_hz)`. The Device's requests in any one second stay within `live_total_hz`, counting each request as one more for every 256 bytes past the first 256.
- **`rate`.** The router waits `retry_ms` before it sends the Device anything else, and doubles the wait for each further `rate` in a row, up to 8 seconds.
- **Sequence.** `n` rises for every new value sent to a Device. A send whose reply never came goes again with the same `n` and value, so a value the Device already took isn't added twice.
- **Resend.** A new link session, a Layout push or an orientation change sends every bound value again. A `layout`, `page`, `tile`, `stream` or `type` refusal makes the router read the Device's Layout again.
- **Turned off.** A stream whose Integration is turned off and reports no sample is sent once more with its last value and the shortest `ttl_ms` (1000), so the Device draws it stale within a second instead of waiting out the original `ttl_ms`. Nothing more is sent for it while the Integration is off, and the first new sample after it is turned on goes as usual. A drag on a bar whose Integration is turned off is dropped and logs nothing. The Device ends that drag when the value goes stale, so it sends no further move or end for it.
- **Values a Tile can't take.** A series is cut to its newest `series` numbers and text to `text_bytes`. A value of the wrong type, or a number outside ±1e9, is never sent.

Each Integration's `status.delivery` counts the values each Device accepted and carries the last refusal.

#### Control and live cover delivery

`Integration::control() -> Option<Arc<dyn Control>>` defaults to None. A Control
is held beside the Service mutex, outside the registry lock. Its `live()` returns
newest samples without blocking, `subscribe(wake)` registers a wake callback,
`act(account, action, timeout)` completes within the timeout, `set(account,
stream, value)` queues without blocking, and `picture(id)` returns owned source
bytes. ControlError is UnknownAccount, UnknownAction, NotSettable, OutOfRange,
or Unavailable(message). A wake starts a live round immediately; `LiveCaps`
requires any supported live kind, not specifically graph.

Control MUST NOT start a process. No public API route reaches act or set.
Only Device touch events (or test-hook MockDeck events) reach them. Process
launching Integrations continue through ADR 0006 approval. Resolve the Tile's
source/account from the Companion's pushed copy of the active Layout; require input and a settable
stream for sets, drop non-finite values, clamp to 0..1, and coalesce moves.
A tap passes the press `part` unchanged as `Control::act`'s `action`.
A drag passes the Integration stream id bound by the Tile's `value` stream
(the widget `field`, `level` for Volume) to `Control::set`, not `value`.
Other Tile streams bind the same-named Integration stream. Check `settable`
on that Integration stream.
Limit actions to ten per Device per second and a two-second timeout. The action
log gains `kind:"integration"`, status `ok` or `failed`, and copy.md detail.
A completed gesture logs once, never per move. No new API endpoint is added.

As built in step C, per Device link session, before anything binds:

- Only a Device connected over link v2 reaches a Control. `page` and `tile`
  must be Layout ids (`^[a-z0-9_]{1,15}$`), `part` must match `^[a-z_]{1,15}$`,
  `seq` must be a u32 that rises (taps and drags count separately), and a drag's
  `stream` must be `value`. Anything else is dropped without a log entry.
- Taps with a `part` pass a gate of ten per second at arrival. A separate
  worker accepts one at a time; excess taps are dropped and counted, without
  occupying the ordinary tap queue. The Companion enforces a two-second wait.
  A timed-out call remains in flight until it returns; no second call starts
  for that Device, including after reconnect. A hung call cannot be forcibly
  cancelled; it disables further control taps for that Device until it returns
  or the Companion restarts, while ordinary taps remain available.
- Drags coalesce at arrival into a mailbox of at most 64 Tile streams plus
  64 completed snapshots. An end survives later moves and is applied first.
  Once completed capacity is exhausted, new gestures are rejected while
  existing ends remain intact. A batch runs at most every 20 ms; completed
  gestures log at most ten times per second. Layout changes clear pending
  gestures and failures; a new link session starts a new hub. There is no wire
  start phase: processing an end clears the previous gesture failure. A Layout
  change conservatively cancels pending gestures on all connected Devices.
  Sequence numbers do not wrap under link-v2: sending u32::MAX exhausts only
  that Device's session, and reconnecting starts fresh sequence tracking.
- Both touch paths use only the Companion's pushed Layout for that Device
  and orientation, after the router's active copy matches its canonical bytes
  (and therefore its canonical CRC revision). No touch fetches a Layout.
  Device-read Layouts and old cache files do not authorize controls: the pushed
  copies live separately in `layouts/pushed-v1`. Push an existing Layout once
  to authorize its controls. Unmatched control taps fail with
  "This Tile's control isn't available." A completed drag on an unmatched Layout
  logs the same failure; an unknown Tile within a matching Layout does nothing.
- A drag requires a `bar` Tile with `input: true`, a numeric stream and the
  Integration's `settable` flag. Changes invalidate the cache before sending
  a Layout or orientation; an older round cannot restore an invalidated entry.
- A tap binds only a part its renderer draws: `previous`, `play_pause` or
  `next` on `now-playing`, `mute` on a `bar` that declares `muted`, on a
  `widget` action with a source. A part never runs any other action type.

The Companion computes a Tile's pixel rect from the active Layout's grid,
position/span and `fullScreen`, and the Device's screen size in the Layout's
orientation, with `td_core::live::tile_rect`:

```rust
pub fn tile_rect(layout: &Layout, page_id: &str, tile_id: &str, screen: ScreenSize) -> Option<PixelRect>;
pub fn cell_rect(screen: ScreenSize, full_screen: bool, grid: [u8; 2], cell: [u8; 4]) -> Option<PixelRect>;
pub fn art_size(tile_w: u16, tile_h: u16) -> Option<u16>;
```

`ScreenSize { w: u16, h: u16 }` and `PixelRect { x: i32, y: i32, w: u16,
h: u16 }` live in `td_core::live`. Missing/non-grid Pages, missing Tiles,
invalid grids/spans or zero screen dimensions return None. The caller supplies
the screen dimensions after applying the active orientation, not the panel's
unrotated dimensions. The Firmware is the truth, because it draws the Tile:
`ui_geometry` in `firmware/components/td_ui_core/ui_geometry.c` and
`ui_tile_rect` in `ui_pages.c`, with a 36 px pager strip at the 600 px design
scale, so a full-grid 1024×600 Tile is `(28,76,968,468)`.
`schema/live/tile-geometry-vectors.json` holds the shared cases. `cell_rect`,
the app's `app/ui/src/lib/deck/geometry.ts` and the Firmware host test
`firmware/tools/test_tile_geometry.py` each reproduce all of them. `art_size`
turns a Tile's size into its cover side by `schema/live/now-playing-vectors.json`;
the router also caps it at `caps.image`, and a Tile with no art box gets
`cover: null`.

Image samples name source pictures owned by Control, never arbitrary URLs.
On a track change publish cover null at once with new metadata. Discard stale
metadata, thumbnail and encoding completions by session/track generation. The
router enforces both on its side too: before it sends a Tile's new `title`, it
clears a cover that Tile shows. The clear obeys cover pacing; the Tile's title,
artist and state wait for it. It sends a Control's sample, or a cover made
from one, only while the Control still reports that sample.
Decode at most 4 MiB source, 4096 by 4096 pixels and 64 MiB decoded memory;
center-crop to the shared Tile art box and encode baseline 8-bit three-component JPEG (no grayscale) at most 32 KiB
using the photo quality ladder. Cache decode failures by source and art-box size. Upload by SHA-256 with put_asset before sending
the image reference. Text goes first; encoding stays outside the link lock.
Keep at most four cover Assets per Device/session, deleting oldest unreferenced
covers first, and sweep leftover JPEG covers at session start. A referenced
Asset remains protected by the Device's in-use refusal. If all four covers are
pinned, clear the replacing Tile and defer its upload until a later round. On asset refusal forget
the known id and upload again; busy and other upload failures retry after the
cover gap, doubling up to 8 s. On full, remove covers
except the current ones and retry once; then report the out-of-space error.
The sweep continues across rounds in batches of at most 16 pages. Failures use
the same backoff; until the listing ends no cover goes, and
other values still do. A cover still being made leaves the art box on its
placeholder (`cover: null`) and never holds up the Tile's text. Control
streams wake a round at once; the cover thread encodes without any lock and
wakes a round when a cover is ready.

Before push, exceeding live_images across all Pages returns HTTP 422
`layout.live_image_limit`, details `{limit,actual}`. Firmware independently
refuses with size. The check runs in `features/layouts.rs` before the push Job starts.
Caps absence means zero images. Covers never cross the UI API.

#### Volume

Integration id `volume`, name "Volume", config `{enabled}` default true, no
secrets. Empty account follows Windows default playback endpoint eRender /
eMultimedia; other accounts return UnknownAccount. Streams: `level` numeric
0..1, settable true; `muted` numeric 0/1, settable false. Action `mute` toggles.
A level above zero unmutes. Core Audio callbacks only post to the owning MTA
worker; unregister and release there before COM uninitialization.

Preset `volume_bar`, "Volume bar", uses widget source volume, account empty,
field level, render bar, streams `{value:"number",muted:"number"}`, input true.
Publish actual OS readback, including our own changes, and heartbeat every 10 s
with ttl_ms 30000. No playback endpoint is an error; non-Windows reports
"Volume needs Windows." Disabling stops collection and clears samples.
Settings persist in `volume.json` beside `sources.json`. Pending level writes
coalesce to the newest value; a default-endpoint change discards queued writes
for the previous endpoint. Mute requests expire after at most two seconds.

With `test-hooks` and `--mock-devices`, `test.volume_write`
(`POST /test-hooks/volume`) accepts
`{level,muted}` to script an external change (finite level in 0..1).
`test.volume_read` (`GET /test-hooks/volume`) returns `{level,muted,sets}`; `sets` counts provider
level writes. These hooks never control real audio and are absent from the
release router. Run `cargo test -p td-host real_os_volume_smoke -- --ignored
--nocapture` on Windows to check Core Audio callbacks; it briefly changes the
level by 1% and restores the original level and mute state.

#### Media

Integration id `media`, name "Media", config `{enabled}` default true, no
secrets. Empty account follows GSMTC GetCurrentSession; others return
UnknownAccount. Streams title/artist are text, cover image, state numeric:
0 idle, 1 paused/stopped, 2 playing. None are settable. Actions are previous,
play_pause and next. Preset `now_playing`, "Now playing", uses widget source
media, empty account, field title, render now-playing and all four required
streams. Field is unused because there is no value stream.

A separate MTA worker owns WinRT objects and event subscriptions; callbacks
only mark dirty/wake it. Session changes revoke old handlers. Bound async waits
and discard late generations. False command results are failures. Do not fetch
track-supplied URLs. No session is idle, not an error. With nothing playing,
Media publishes all four streams: `state` 0, `title` "", `artist` "" and
`cover` null. The Device and preview draw "Nothing playing" from copy.md,
ignore title and artist while idle, and hide controls. No stream is left unset.
Paused retains metadata; GSMTC Changing keeps the last state value. Recheck thumbnail
at 300 ms, 1 s and 3 s after title changes; never publish an old generation.
Heartbeat every 10 s with ttl_ms 30000. Non-Windows reports "Media needs Windows."
Settings persist in `media.json` beside `sources.json`. Disabling revokes event
subscriptions, cancels pending work and clears samples and cached pictures.
Re-enabling keeps sample sequence numbers ahead of earlier delivery.

Windows 10 1809 or later is required. Thumbnail streams are capped at 4 MiB
before allocating a read buffer; the returned length and stream size are checked
again after the read. Source pictures use SHA-256 ids and an eight-entry cache.
The Companion delivery layer owns image decoding and Device Asset conversion;
Integration API responses contain neither track metadata nor thumbnail bytes.
Commands have a maximum two-second deadline, including queue time. A timeout or
false result reports "The player didn't respond." and is never retried.

For an OS-only smoke check, run `cargo test -p td-host media_os_smoke -- --ignored --nocapture`
from `app/`. It reports playback state and thumbnail presence without printing
private metadata; with a current player, it toggles play/pause twice.

These sections define steps B through I; step A adds contracts and mocks only.

The UI fake (`app/ui/src/lib/companion/fake/fake-live.ts`) stands in for steps B, C, E
and F in demo mode. It lists Volume and Media with the Presets above, and keeps
a Volume level (0.64, unmuted) and one track ("Night Drive" by "Tidal Lines",
playing). A connected mock Device whose `tile_kinds` include a Tile's renderer
holds that Tile's values: `GET /test/devices/{device}/sets` returns
`{tile, stream, type, value}[]`, with text through the shared `fit_text` rules
and `cover` always null. `POST /test/devices/{device}/input` takes a `v2.input`
body (`page`, `tile`, `stream`, `phase`, `value`) for a bar Tile with `input`
bound to Volume's `level`, clamps the value to 0..1, and sets the level, which
unmutes above zero; other Tiles get 409 `input.unbound`. `GET
/test/integrations/volume` returns `{level, muted}`. It doesn't pace, coalesce
or log actions; the Companion's own tests cover those.

#### Custom HTTP and command

`custom` starts with `{enabled: false, sources: []}` and persists non-secret
settings in `custom.json`, beside `sources.json`. If `custom.json` is corrupt or
fails validation at startup, the Companion moves it aside to
`custom.corrupt.json` (or `custom.corrupt-2.json`, `custom.corrupt-3.json` and so on when an earlier backup exists, so no backup is replaced), starts `custom` empty (`{enabled: false, sources: []}`),
and reports `error` status `"Couldn't read custom.json. Kept it as custom.corrupt.json and started empty."`
(naming the file it wrote) without blocking other Integrations. If `custom.json` cannot be read or renamed,
it leaves the file in place, starts empty, and reports `"Couldn't read custom.json. Started empty."`.
Saving valid settings writes a new `custom.json` and clears that startup error.
The Integrations form exposes
`sources` as a JSON editor. Unknown fields are rejected. Each of at most 16
sources has `{id, name, interval_s, kind, input}`:

- `id`: a unique `[a-z0-9_-]+` stream id, at most 48 bytes.
- `name`: a nonempty display name, at most 128 bytes.
- `interval_s`: an integer from 1 to 3600.
- `kind`: `number` or `text`. Streams report `numeric` or `text`, with no unit.
- `input`: one of the shapes below. `account` is always the empty string.

HTTP example (GET is the only method; there is no configurable method field):

```json
{
  "enabled": true,
  "sources": [{
    "id": "temperature",
    "name": "Temperature",
    "interval_s": 5,
    "kind": "number",
    "input": {
      "type": "http",
      "url": "https://example.invalid/metrics",
      "value_path": "$.readings[0].value",
      "headers": {"Authorization": "temperature_auth"},
      "tls_pin": null
    }
  }]
}
```

`headers` maps HTTP header names to unique vault field ids, **never header
values**. Saving settings creates the corresponding write-only secret controls.
Write each value through `integrations.secret_put`. The vault stores only the
configured URL's origin with each secret, not `tls_pin`; secrets are deleted when
the field is removed, the origin changes, or `tls_pin` is added, changed or
cleared, so a new pin never receives a secret entered under the old trust. Same-origin
path changes retain them. Because of that, a `text` source pointed at a
same-origin endpoint that echoes request headers can read its own secret back as a
live value. This is accepted: the user configures both the endpoint and the
secret. Credentials use the existing `credential_transport`
policy: HTTPS, or an explicitly typed loopback/private-address HTTP exception.
URLs cannot contain user information, query strings or fragments; put credentials
in secret headers. Host, Content-Length, Transfer-Encoding and Connection cannot
be overridden. Header values cannot contain control characters.

HTTP sources use the shared `Fetch::request` transport:
one 10-second deadline covering connect, TLS, headers and the whole body, a 1 MiB
body limit, and no redirects. Without a pin, TLS uses the platform verifier.
`tls_pin`, when set, is exactly 64 hexadecimal digits representing the server leaf
certificate's SHA-256 fingerprint, and needs an `https://` URL; a pin with an
`http://` URL is rejected with `integration.config_invalid` and an issue on the
`sources` path. With a pin, that leaf certificate is the only one accepted,
including a self-signed one; a different certificate is rejected even if the
system trusts it. TLS handshake signatures are still verified. No global
verification setting is changed. Certificate rotation requires updating the pin.

The value path grammar is `$` followed by `.key` or `[index]` segments, with at
most 32 segments. Keys use ASCII letters, digits and underscores and cannot start
with a digit; indexes are unsigned decimal integers. For example,
`$.a.b[0].c`. Wildcards, recursive descent, filters, slices and quoted keys are
unsupported. Missing keys, wrong container types and out-of-range indexes are
errors. Number values must be JSON numbers, finite and within ±1e9; strings such
as `"42"` are not coerced. Text values must be JSON strings of at most 4096 bytes.

Command input is `{type: "command", program: <absolute path>, args: [<string>],
value_path: <path or null>}`. Windows accepts executable `.exe` and `.com` paths,
not batch files. Arguments are passed directly, without a shell. There is no
stdin, stderr is discarded, stdout must be UTF-8 and is capped at 64 KiB. The
command runs in its own Windows Job object (on Unix, its own process group). When
the run ends, on success, timeout (10 s) or oversized output, every process in it
is stopped, including background processes the command started, and the command
is reaped. Output is complete when stdout closes, so a background process that
keeps stdout open makes the run time out at 10 s. A value path
parses stdout as JSON using the same rules. With `value_path: null`, trimmed
stdout is parsed as a number or used as text. Programs inherit the Companion's
working folder and environment; do not put credentials in command arguments.

Commands reuse the native approval service and `approvals.list`, with kind
`custom` and a synthetic `custom` context in the Device field. No API session,
including the Electron window, can grant approval. The hash covers the exact
program, arguments, action kind, working folder and shell setting. Run once
permits one collection before the pending request's five-minute deadline; an
unused Run once past that deadline asks again. Always allow persists through the
existing grant store and can be revoked with `actions.allowed_remove`, after
which the next collection asks again. Argument changes require approval. Denied or expired
requests never run; their decisions remain blocked for the Companion session.
Restart the Companion to request the same denied command again. The Companion keeps
at most 256 of these answers. When the list is full, a new command drops the oldest
answer, so it can still ask; a command whose answer was dropped asks again.
While a source waits for approval it isn't failing: it rechecks every second
without backing off, so a Run once answer is used well before its deadline.
Denied and expired commands report `"Denied on this PC."` and `"Approval expired."`
and back off like other failed polls instead of rechecking every second.
`revision` increases when settings are saved or the Integration's status changes,
so repeated polls with an unchanged status do not republish `integration.status`.

Polling is serial within the Integration. Manual refresh and configuration
changes reset due times while preserving a one-second request floor. Each failed
source backs off exponentially, capped at one hour. Waiting for approval doesn't
count as a failure. The last good value remains until its Device
TTL expires. Successful samples have rising sequence numbers and a TTL of three
times the configured interval, clamped to 1000–60000 ms. Disabling clears live
output. Presets follow source order: `{name} value`, then `{name} graph` for each
number source. Text has no Presets because current Firmware advertises
`text_bytes: 0`; text Tiles remain a follow-up.

Errors use fixed messages for access refusal, timeout, oversized response,
certificate failure, redirects, invalid values and command failure. Upstream
bodies, transport diagnostics and stdout/stderr are never included in status,
logs or API responses. Values are delivered internally through `readings()` and
`live()`.

#### This PC

`this_pc` has config `{enabled: boolean}` (default `true`), no secrets and the `refresh` action. Unknown fields are rejected. Disabling clears readings and stops collection; re-enabling resets the rate baseline. The configuration is saved beside sources.json as this-pc.json.

All streams have kind `numeric`. Streams and Preset tiles use this fixed order; the Preset id matches the stream id:

| Stream id | Preset tile name | Unit |
|---|---|---|
| `cpu_pct` | CPU usage | `%` |
| `ram_pct` | RAM usage | `%` |
| `ram_used_bytes` | RAM used | `bytes` |
| `ram_total_bytes` | RAM total | `bytes` |
| `gpu_pct` | GPU usage | `%` |
| `disk_active_pct` | Disk activity | `%` |
| `disk_read_bytes_s` | Disk read | `bytes/s` |
| `disk_write_bytes_s` | Disk write | `bytes/s` |
| `network_receive_bytes_s` | Network receive | `bytes/s` |
| `network_send_bytes_s` | Network send | `bytes/s` |

Every Preset uses `{type: "widget", source: "this_pc", account: "", field: <stream id>}` and `render: "graph"`. Windows opens one PDH query and adds its CPU, disk and GPU counters once. Collection runs at most once per second, including manual refresh. CPU uses Processor Information total Processor Utility; disk activity is 100 minus PhysicalDisk total idle time. GPU usage sums instances by engine type and takes the busiest type, capped at 100%. RAM comes from GlobalMemoryStatusEx. Disk and network rates use cumulative-byte deltas divided by elapsed monotonic time; the first sample and counter resets emit no rate. Network sums physical adapters. The adapter list is read again every 10 seconds: an adapter that appears gets counters and starts counting from its next reading, so its earlier traffic never shows as a spike, and an adapter that goes away has its counters removed.

A missing counter omits only its readings; descriptors and Presets stay discoverable. Status is `error` with `Unavailable: {names}.` using the Preset names in order. Healthy streams continue. Non-Windows builds emit no readings and report `This PC needs Windows.`. Values stay inside the Companion for Device delivery, never in HTTP responses. Each collection is one live sample per stream with `ttl_ms` 5000, so a Device shows This PC values stale five seconds after collection stops.

### System

Feature module `system`.

| Name | Method and path | Request → Response |
|---|---|---|
| `system.ping` | `GET /system/ping` | → `{api: ["v1"]}`. Public |
| `system.get` | `GET /system` | → `{version, build, api: "v1", mock, data_dir_override, platform}`. `platform` is `windows`, `linux` or `other`; the UI names the keychain from it |
| `system.lock` | `POST /system/lock` | → `{lock}`, where `lock` is `lock_` and 16 random characters. Window sessions only; owner-bound 10-minute lease. 409 `system.busy` while any Job is active or another session holds the lock. Once held, new Jobs fail with 503 `companion.installing` |
| `system.lock_renew` | `PUT /system/lock/{lock}` | → `()`. Owner window session only; renews the lease for 10 minutes |
| `system.unlock` | `DELETE /system/lock/{lock}` | → `()`. Owner window session only |
| `system.shutdown` | `POST /system/shutdown` | → `()`, then `companion.stopping`. Window sessions only |

App updates aren't API routes. They stay in Electron's preload as `openTouchDeck.updates.check()`, `download(onProgress)` and `install()` (ADR 0005). The lock is released when its owner session ends or is revoked (including Reset pairing), or its lease lapses. Browser sessions and non-owner sessions get 403 `auth.forbidden` on lock operations. So does renewing or releasing a lock whose lease has lapsed, since nobody owns it any more; take a new one with `system.lock`. Before installing, Electron takes `system.lock`, renews it with `system.lock_renew` while needed, then stops the Companion with `system.shutdown` and runs the installer.

### Electron signed updater (87A)

The preload calls always resolve with data, except successful `install()`, which quits without resolving. `UpdateCheck`, `UpdateProgress`, `UpdateDownload` and `UpdateInstall` in `app/ui/src/lib/companion/session.ts` are the UI seam. Each update IPC handler uses the same top-frame, window and Companion-origin check as `session`. The renderer supplies no URL, path, key or installer arguments.

The Windows x64 feed is `UPDATE_FEED` in `app/electron/src/updates/index.ts`. Only a strictly newer SemVer version is offered. Manifest bodies are limited to 1 MiB and 15 seconds; installers to 256 MiB and 5 minutes, including redirects and reading the body. Every redirect is checked for HTTPS. Unpackaged builds alone honor `TOUCHDECK_UPDATE_TEST_FEED=http://127.0.0.1:PORT/latest.json`; this also permits numeric-loopback HTTP artifact URLs. Packaged builds ignore it.

For #91 and #108: set `TOUCHDECK_UPDATER_PUBKEY` at build time to the existing base64-wrapped public key. Every Electron build generates the ignored `src/updates/key.gen.ts` before TypeScript compiles it. Unset means disabled, including ordinary source builds. No runtime environment, file or IPC supplies the trusted key. Release packaging must use this build entry point, supply the real public key, and protect app.asar integrity (including the Electron integrity fuse); that wiring belongs to #91/#108. The existing Tauri packaging scripts do not package Electron.

Verification follows [`tauri-plugin-updater` 2.12.0](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.12.0/plugins/updater/src/updater.rs) and [`minisign-verify` 0.2.5](https://github.com/jedisct1/rust-minisign-verify/blob/0.2.5/src/lib.rs): decode the base64-wrapped minisign packet, match the eight-byte key id, verify Ed25519 over either the original bytes (`Ed`) or their BLAKE2b-512 hash (`ED`), and verify the global signature over the 64-byte signature followed by the trusted comment. Both algorithms are required for existing release compatibility. The manifest carries the same signature as the installer's `.sig`; no second signature fetch is required. Like the existing `requireSignedVersion: true` setting, production downloads also require a matching `version:` field in the authenticated comment, including build metadata (an optional leading `v` is ignored). The v0.1.0 compatibility test verifies the cryptographic signature independently of that newer version requirement.

Downloads use an exclusive file in a random `update-*` folder inside the Companion's secured runtime folder, inheriting its protected DACL. Runtime and staging folders must be directories, never symlinks or junctions, before writing and before launch. Startup removes leftover staging folders. Terminal download and install failures clear ready state and remove the whole staging folder; replacing a ready download does too. Busy retains the verified download for retry. Install verifies before acquiring `system.lock` and again after renewal and shutdown, immediately before launch. One check, download or install runs at a time. Lock calls pin the window session for the entire attempt. Busy returns the error's `details.device` string, or null if absent. A successful lock is renewed immediately before shutdown; each API call times out after 5 seconds, within the 10-minute lease. Failure releases only a lock this attempt acquired.

After `system.shutdown` acknowledges, Electron launches the verified installer directly with `/S --updated --force-run` (silent, update, start the app afterwards), the flags the electron-builder NSIS installer takes, with no shell and no inherited CLI arguments, then quits. Before it replaces files, the installer waits for the Companion in its install folder to exit and ends it only after 15 seconds (#91). A failed spawn returns `installer`; starting the Companion again is the normal next-launch recovery.

### Auth

The auth module mounts these itself. Features can't import it.

| Name | Method and path | Request → Response |
|---|---|---|
| `auth.proof` | `POST /auth/proof` | `{nonce}` → `{proof}`. Public |
| `auth.session_create` | `POST /auth/sessions` | Bearer pairing token, `{kind: "window", label?}` → `{id, token}`. Public, but needs the pairing token |
| `auth.pair` | `POST /auth/pair` | `{code, label}` → `{id, token}`. Public, but needs a pairing code |
| `auth.pair_code` | `POST /auth/pair-codes` | → `{expires_ms}` only. Window sessions only. The Companion opens `/pair#code=…` in the default browser and shows the manual code only in a native tray notification; replaces any pending code |
| `auth.sessions` | `GET /auth/sessions` | → `SessionInfo[] {id, kind, label, created_ms, last_used_ms, current}` |
| `auth.session_delete` | `DELETE /auth/sessions/{id}` | → `()`. `self` means the caller's own session |
| `auth.sessions_delete` | `DELETE /auth/sessions?kind=browser` | → `()` (204). Window only; 403 `auth.forbidden` for browsers. Missing or other `kind`: 400 `request.invalid` |
| `auth.reset` | `POST /auth/reset` | → `()`. Reset pairing |

### Test hooks

Feature module `test_hooks`, compiled only with the `test-hooks` Cargo feature and mounted only when the Companion runs on mock ports (`--mock-devices N`). Release builds don't contain it: `package_windows.py` builds `td-companion` without features and refuses a binary that holds a test route or test argument. Its types go into a separate `api.test.gen.ts`, so the product UI can't import them.

One exception lets a board pass drive the real Device while the Companion owns its port. A `test-hooks` Companion started with `--test-real-device-console` (the only argument) also serves `test.console` for Devices on real ports. Every other test route stays `route.not_found` there, and without the argument `test.console` is `route.not_found` for real ports too. A release build rejects the argument as unknown.

| Name | Method and path | Request → Response |
|---|---|---|
| `test.console` | `POST /test/devices/{device}/console` | `{line, wait_ms?}` → `{reply, lines}`. Full console for the e2e harness and board pass. Waits up to `wait_ms` (default 3000, at most 15000, else 400 `request.invalid`) for the `@td` reply whose `cmd` is the line's first word. `reply` is that JSON or `null`; `lines` holds the Device's console lines from the send to the reply or the deadline, at most 256 |
| `test.mock` | `POST /test/devices/{device}/mock` | `{action: "no-confirm" \| "rollback" \| "normal-update" \| "stop" \| "hang-up"}` → `()` |
| `test.press` | `POST /test/devices/{device}/press` | `{page, tile, kind}` → `()` |
| `test.sets` | `GET /test/devices/{device}/sets` | → `Value[]` |
| `test.record` | `GET /test/devices/{device}/record` | → `{received, lines, settings, reboots, assets, layouts}` |
| `test.emit` | `POST /test/devices/{device}/emit` | `{events, repeat?, every_ms?}` → `()`. Events: `key.report`, `key.resync`, `press` (with an optional renderer `part`), `v2.input` (`page`, `tile`, `stream`, `phase`, `value`) and `drop` |
| `test.input_log` | `GET /test/input-log` | → `string[]` |
| `test.ports` | `POST /test/ports` | `{plug: {id, kind: "device" \| "device-v1" \| "device-radio" \| "silent" \| "other-firmware" \| "busy"}}` or `{unplug: id}` → `()`. `device-radio` is a v2 Device on the radio build: its hello says `radio`, and it has a Wi-Fi station and the Bluetooth keyboard |
| `test.approvals_script` | `POST /test/approvals/script` | `{answers: ("once" \| "always" \| "deny" \| "fail" \| "never")[]}` → `()`. Sets what the `FakePrompter` answers next. `fail` acts as a dialog that can't be shown |
| `test.clock` | `POST /test/clock` | `{advance_ms}` → `()`. Moves the Companion's clock for expiry tests |
| `test.firmware_script` | `POST /test/firmware/script` | `{answers: ("accept" \| "decline")[]}` → `()`. Replaces the native Firmware dialogs with a scripted prompter that gives these consent answers in order and waits once they run out. Its file picker returns the test bundle |

`test.media` (`POST /test-hooks/media`) scripts the in-memory Media provider.
Its body is `{session: null}` for idle, or `{session: {title, artist, state, cover?}}`.
State is 0, 1 or 2; title and artist are at most 4096 UTF-8 bytes each; cover is
base64 source image bytes (at most 4 MiB decoded), never a URL. The request limit
is 6 MiB. Disabled Media rejects scripts. This endpoint requires test hooks and
mock Devices; it cannot control a real player.

## Mapping from today's Tauri commands and events

History: release builds of the removed `app/src-tauri` crate registered 33 commands. `--features e2e` adds 7, marked e2e. Each one maps to a route above, moves to Electron, or goes away.

| # | Today | Replacement | Notes |
|---|---|---|---|
| 1 | `app_update_check` | Electron preload `openTouchDeck.updates.check()` | ADR 0005 |
| 2 | `app_update_download` | Electron preload `openTouchDeck.updates.download(onProgress)` | The progress Channel becomes an IPC callback |
| 3 | `app_update_install` | Electron preload `openTouchDeck.updates.install()` | Takes the owner-bound `system.lock` lease first and renews with `system.lock_renew`, replacing `AppUpdateGate` |
| 4 | `e2e_app_update` (e2e) | Electron main's scripted updater, behind a test flag | App updates live in Electron |
| 5 | `update_firmware` | `firmware.update`, `firmware.repair` or `firmware.install`, with `firmware.pick` | Bytes no longer travel as `number[]`. `recover=true` on a known Device is `firmware.repair`, on an unknown board `firmware.install`. `firmware.pick` takes `{device}`; install, repair and downgrade wait for native confirmation. The global `updating` flag becomes the per-Device Job slot |
| 6 | `list_decks` | `devices.list`, `new_boards.list`, `ports.list` | Detection is automatic |
| 7 | `connect` | Automatic. `devices.connect` turns auto-connect back on | By `DeviceId`, not port |
| 8 | `disconnect` | `devices.disconnect` | Per Device |
| 9 | `deck_info` | `devices.get` | `online` becomes `link.state` |
| 10 | `console_send` | `console.send`; `test.console` for the e2e harness | Per Device; release route permits only the read-only allowlist |
| 11 | `e2e_mock` (e2e) | `test.mock` | Per mock Device |
| 12 | `e2e_mock_press` (e2e) | `test.press` | Per mock Device |
| 13 | `e2e_mock_sets` (e2e) | `test.sets` | Per mock Device |
| 14 | `push_layout` | `layouts.push` | Typed `Layout` body instead of a string. Returns a Job whose result is `PushReport` |
| 15 | `remove_unused_icons` | `assets.prune` | Takes Asset ids to keep, not Layout texts |
| 16 | `builtin_icons` | `assets.builtin` | |
| 17 | `default_icon` | `assets.suggest` | |
| 18 | `icon_file` | `assets.file` | Raw bytes with a media type |
| 19 | `take_app_icon` | `assets.create` with `{source: {kind: "app", path}}` | A Job |
| 20 | `pull_layout` | `layouts.get` | Adds `revision` |
| 21 | `settings_get` | `devices.settings_get` | Per Device |
| 22 | `settings_put` | `devices.settings_patch` | `reboot_required` is now `restart_required` |
| 23 | `reboot_deck` | `devices.restart` | |
| 24 | `e2e_mock_record` (e2e) | `test.record` | `icons` is now `assets` |
| 25 | `mirror_state` | `DeviceSummary.key_mirror` | Per Device |
| 26 | `set_action_layout` | Removed | Presses always resolve against the Layout each Device reports. Importing a file happens in the UI: parse, then `layouts.push` |
| 27 | `action_layout` | `DeviceSummary.layout` | Per Device |
| 28 | `pending_approvals` | `approvals.list`, also in `sync` | Read-only. Entries carry `device` |
| 29 | `action_log` | `actions.log` | Entries carry `device` and an `id` |
| 30 | `approval_answer` | Removed. The native dialog on the PC answers | ADR 0006. `approvals.show` shows the dialog again. `actions.allowed_remove` removes an approval |
| 31 | `focus_window` | Removed from the API | The renderer calls `window.focus()`. If the key monitor needs the window in front, a preload call goes through security review |
| 32 | `e2e_emit` (e2e) | `test.emit` | Per mock Device |
| 33 | `e2e_input_log` (e2e) | `test.input_log` | |
| 34 | `sources_view` | `integrations.get` | `config_file` leaves the API |
| 35 | `sources_status` | `integrations.list` and the `integration.status` event | Delivery per Device |
| 36 | `sources_save` | `integrations.config_put` | Normalizing (trimming the URL, the 60 s floor) stays in the Integration |
| 37 | `sources_set_key` | `integrations.secret_put` | Write-only |
| 38 | `sources_clear_key` | `integrations.secret_delete` | |
| 39 | `sources_refresh` | `integrations.refresh` | |
| 40 | `build_sha` | `system.get` | The response includes `build` |

| Today's event | Replacement | Topic |
|---|---|---|
| `console-line` | `device.console {device, line}` | `console` |
| `deck-event` | `device.event {device, event}` | `device-events` |
| `link-closed` | `device.updated` with `link.state` set to `lost` or `disconnected` | `state` |
| `key-mirror` | `device.key-mirror` | `key-mirror` |
| `action-layout` | `device.updated` with `layout` | `state` |
| `action-log` | `action.logged` | `state` |
| `approval-request` | `approval.requested`, plus `approval.resolved` so every client clears it | `state` |
| `firmware-update` | `job.updated` | `state` |
| `Channel<DownloadProgress>` in `app_update_download` | The Electron IPC callback passed to `updates.download(onProgress)` | none |

The UI's other Tauri calls go too. `getCurrentWindow().title()`, which showed the build SHA, becomes `system.get`, and the window title is the static "Open TouchDeck". `getVersion()` becomes `system.get`.

## Errors

### One shape

Handlers, extractors, middleware and `Job.error` all return this body and nothing else:

```json
{ "error": { "code": "layout.invalid", "message": "The Layout has 2 problems.",
             "details": { "issues": [ { "path": "pages[0].tiles[3].colSpan", "message": "exceeds the grid" } ] } } }
```

- `code` is stable, and the UI switches on it. Codes are `<namespace>.<snake_name>`.
- `message` is one plain sentence. It's for logs, developer tools and the **Details** disclosure. The UI words user-facing text itself, keyed by `code`.
- `details` is optional and typed per code.
- An unknown code is a generic error to the client.

Each feature owns an error enum. The kernel takes the message from `Display` and the status from `ApiError::status()`.

```rust
#[derive(Debug, thiserror::Error, Serialize, TS)]
#[serde(tag = "code", content = "details")]
pub enum LayoutError {
    #[serde(rename = "layout.invalid")]
    #[error("The Layout has {} problems.", issues.len())]
    Invalid { issues: Vec<LayoutIssue> },
    #[serde(untagged)]
    #[error(transparent)]
    Common(#[from] CommonError),
}
```

The generated TypeScript has `type ApiErrorBody = LayoutError | DeviceError | … | CommonError`, each with a literal `code`, so `isApiError(e, 'layout.invalid')` narrows `e.details`. `ErrorCode` is the union of every code. That replaces today's string matching for "denied", "busy", "timed out" and "rolled back".

### Error code catalogue

Features add codes as they land. This list is the starting set.

| Code | HTTP | When |
|---|---|---|
| `security.host` | 421 | The `Host` isn't allowed |
| `security.origin` | 403 | The `Origin` or `Sec-Fetch-Site` isn't allowed |
| `auth.missing` | 401 | No token |
| `auth.invalid` | 401 | The token is unknown |
| `auth.revoked` | 401 | The session was revoked, its window closed, or pairing was reset. The 60 s check then forgets a session with no open WebSocket, and its token gets `auth.invalid` |
| `auth.expired` | 401 | A browser session passed 7 days idle |
| `auth.forbidden` | 403 | The session kind or lock ownership forbids this route, such as a browser minting a pairing code or taking the install lock |
| `auth.code_invalid` | 401 | The pairing code is wrong, used or expired |
| `auth.code_locked` | 429 | Too many wrong pairing codes. Start pairing again |
| `request.invalid` | 400 | Malformed JSON, path or query. `details: {part, reason}`. Wraps axum rejections |
| `request.too_large` | 413 | The body or decoded image passed its size or frame limit |
| `request.timeout` | 408 | The request headers, body or handling took longer than the Request limits allow |
| `limits.exceeded` | 429 | A pending upload, render Job, WebSocket or approval limit was reached |
| `console.forbidden` | 403 | The line is not printable ASCII within 256 bytes or is not on the read-only allowlist |
| `request.media_type` | 415 | The `Content-Type` is wrong for this route |
| `route.not_found` | 404 | No route with that path |
| `route.method_not_allowed` | 405 | The path exists with another method |
| `device.not_found` | 404 | No Device with that id |
| `device.not_connected` | 409 | The Device is known but offline, disconnected, busy or unresponsive. `details.link` |
| `device.connected` | 409 | `devices.forget` on a connected Device. Disconnect it first |
| `device.busy` | 409 | A Job holds the Device. `details.job` |
| `device.timeout` | 504 | The Device didn't reply in time |
| `device.link` | 502 | Link I/O failed or the Device refused. `details.reason` |
| `device.unknown_outcome` | 502 | The link closed after the request went out and before its reply, so whether the Device acted is unknown. `details.reason` |
| `device.needs_firmware` | 409 | The feature needs newer Firmware. `details.field` names the setting; `details.version` is the version that adds it, or `null` while no released Firmware has it (brightness today) |
| `device.usb_only` | 409 | The Device is connected over Wi-Fi, and this setting changes only over USB. Nothing was sent |
| `bluetooth.off` | 409 | Bluetooth is off on the Device. `details.bluetooth` |
| `bluetooth.starting` | 409 | Bluetooth is still starting on the Device; ask again. `details.bluetooth` |
| `bluetooth.full` | 409 | The Device keeps `bonds_max` computers and phones; forget one first. `details.bluetooth` |
| `bluetooth.radio` | 502 | Bluetooth didn't start on the Device; the setting stays on. `details.bluetooth` when the Device sent it |
| `bluetooth.storage` | 502 | The Device couldn't save the change. `details.bluetooth` when the Device sent it |
| `new_board.not_found` | 404 | No New board with that id |
| `layout.invalid` | 422 | The Layout fails validation. `details.issues` |
| `layout.live_image_limit` | 422 | The Layout declares more image-stream Tiles than the Device's `live_images`. `details {limit, actual}` |
| `layout.orientation_mismatch` | 422 | The path and body orientations differ |
| `layout.changed` | 412 | The Device's Layout changed since it was loaded |
| `layout.stored_invalid` | 502 | The Layout stored on the Device fails validation |
| `layout.not_found` | 404 | The Device stores no Layout for that orientation |
| `firmware.bundle_invalid` | 422 | The picked file isn't a valid bundle |
| `firmware.bundle_not_found` | 404 | The `BundleId` expired, was consumed, belongs to another target or never existed |
| `firmware.declined` | Job error | The user declined native confirmation, or didn't answer it within 5 minutes; no Firmware write occurred |
| `firmware.channel_mismatch` | 409 | The bundle is the normal build where the radio build was asked for, or the other way round. `details {channel, bundle}`; no Firmware write occurred |
| `firmware.failed` | Job error | The update failed. `details.reason` |
| `firmware.rolled_back` | Job error | The Device rolled back to its previous Firmware |
| `firmware.confirm_timeout` | Job error | The new Firmware didn't confirm in time |
| `firmware.usb_only` | 409 | The Device is connected over Wi-Fi; Firmware updates need USB |
| `wifi.unreachable` | 502 | Nothing answered at the address, or it isn't an address. `details.reason` |
| `wifi.window_closed` | 409 | The Device isn't pairing, or pairing is locked out. Open Pair new app on the Device |
| `wifi.wrong_code` | 422 | The code is wrong. `details.attempts_left` |
| `wifi.locked` | 429 | The third wrong code; the Device stopped pairing for a while |
| `wifi.full` | 409 | The Device holds four Pairings, or with no advertised count, may still hold this Companion's. `details.paired` |
| `wifi.already_paired` | 409 | This Companion already has a Pairing with the Device |
| `wifi.expired` | 409 | The pairing window closed or was cancelled on the Device |
| `wifi.busy` | 409 | The Device is linked over USB or to another app |
| `wifi.attempt_not_found` | 404 | No started pairing with that id |
| `wifi.not_paired` | 404 | No Pairing with that Device |
| `wifi.keychain` | 503 | The OS keychain is missing or refused the key. The message says what to do |
| `wifi.version` | 409 | The Device answered `version`: its Firmware can't pair with this Companion. Update it over USB |
| `wifi.failed` | 502 | The Device broke net-v2 or a proof failed. `details.reason` |
| `wifi.discovery` | 503 | mDNS couldn't start; type the address instead |
| `asset.not_found` | 404 | No Asset with that id |
| `asset.render_failed` | Job error | The image or icon couldn't be rendered. GIF failures carry `details.reason` (Animated GIF Assets) |
| `asset.store_full` | 507, Job error | Storing a prepared Asset would pass the Asset store quota and nothing can be swept. `details: {limit, actual, unit: "bytes"}` (Asset store) |
| `upload.not_found` | 404 | The upload expired or never existed |
| `icon_pack.not_found` | 404 | No offered or installed Icon pack has that prefix, or it has no such icon |
| `icon_pack.exists` | 409 | An installed or curated Icon pack already uses the imported file's prefix |
| `icon_pack.busy` | 409 | That Icon pack is being installed |
| `icon_pack.too_many` | 409 | 20 imported Icon packs are installed already |
| `icon_pack.invalid` | 422, Job error | The file isn't usable Iconify JSON, or the package holds none |
| `icon_pack.integrity` | Job error | The downloaded tarball doesn't match its pinned SHA-512, so nothing was unpacked |
| `icon_pack.download_failed` | Job error | The tarball didn't download or passed 32 MiB |
| `job.not_found` | 404 | No Job with that id |
| `job.not_cancellable` | 409 | The Job's current phase can't be cancelled |
| `job.cancelled` | Job error | The Job was cancelled |
| `approval.not_found` | 404 | The request was answered or expired |
| `action.not_found` | 404 | No always-allowed action with that hash |
| `integration.not_found` | 404 | No Integration with that id |
| `integration.config_invalid` | 422 | The config fails the Integration's schema. `details.issues` |
| `integration.credentials` | 500 | The keyring failed |
| `brandfetch.no_key` | 409, Job error | No Brandfetch API key is saved |
| `brandfetch.key_refused` | 502, Job error | Brandfetch refused the API key |
| `brandfetch.quota` | 429, Job error | The Brandfetch account's requests are used up |
| `brandfetch.not_found` | 404, Job error | Brandfetch doesn't know the brand |
| `brandfetch.no_logo` | 422, Job error | The brand has no logo of that kind on an allowed host |
| `brandfetch.unavailable` | 502, Job error | Brandfetch didn't answer, redirected or answered another status |
| `brandfetch.store` | 500, Job error | The Brandfetch settings or provenance file couldn't be written |
| `brandfetch.no_keychain` | 503, Job error | No system keychain is reachable, such as Linux without a running Secret Service |
| `system.busy` | 409 | A Job is active or another session holds the install lock |
| `system.dialog_open` | 409 | A native file dialog is already open |
| `companion.installing` | 503 | The install lock is held |
| `internal` | 500 | A bug. The message is generic, and the detail goes only to the log |

## Types from Rust

### One declaration per route

Each route is one `impl Endpoint`. The trait fixes the name, method, path, path parameters, query, body, success type and error type in one place. The axum route, the typed Rust test client and the generated TypeScript client all read that declaration.

```rust
pub trait Endpoint: Sized + Send + Sync + 'static {
    const NAME: &'static str;                  // "layouts.push" → companion.layouts.push()
    const METHOD: Method;
    const PATH: &'static str;                  // relative to /api/v1, axum 0.8 `{param}` syntax
    const STATUS: StatusCode = StatusCode::OK; // 202 when Ok = JobRef
    const BODY_LIMIT: usize = 1 << 20;
    type Path: DeserializeOwned + TS + Send;   // () or a struct such as DevicePath { device: DeviceId }
    type Query: DeserializeOwned + TS + Send + Default;
    type Body: RequestBody;                    // Json<T> | Upload | NoBody
    type Ok: ResponseBody;                     // a JSON type | Bytes { media_type } | JobRef | ()
    type Error: ApiError;
    fn handle(ctx: Ctx, req: Req<Self>) -> impl Future<Output = Result<Self::Ok, Self::Error>> + Send;
}
pub struct Req<E: Endpoint> { pub path: E::Path, pub query: E::Query, pub body: E::Body, pub client: ClientInfo }
```

`ClientInfo` holds the session id and kind, for the audit log and for routes limited to window sessions.

### Generated TypeScript

ts-rs derives each type. The Companion's generator writes `<ui>/src/lib/companion/api.gen.ts` with every type, an endpoint table, typed wrappers `companion.<feature>.<action>()`, the `EventMap`, the `JobMap`, the `ErrorCode` union and `SyncData`. Each wrapper takes optional request headers after the bearer, such as `If-Match`. A response that isn't JSON comes back as a `Blob`. The file starts with `// Generated from td-companion. Do not edit.` and uses LF endings. `pnpm gen:api` runs `cargo run -p td-companion --features test-hooks -- --emit-ts <path>`.

`td-core` gains an optional `ts` feature that adds `#[cfg_attr(feature = "ts", derive(TS))]` to `Layout`, `Hello`, `Settings`, `Orientation` and the other types the API carries.

### Freshness rule

- **`api_bindings_are_current`** renders `api.gen.ts` in memory and compares it byte for byte with the committed file. On a difference it fails with "run `pnpm gen:api`". It runs inside `cargo test --workspace`, which CI already runs.
- **`pnpm typecheck`**, part of `pnpm lint`, compiles the UI against the generated file, so a UI calling a removed or renamed route fails CI.
- **`api_contract_is_documented`** asserts that every endpoint `NAME`, event type and error code appears in this page.
- **`test_registry_is_consistent`** fails when an endpoint's `NAME` doesn't start with its feature's name, when two endpoints share a method and path, or when `Path`'s fields don't match the `{params}` in `PATH`.

Rejected options:

| Option | Why not |
|---|---|
| ts-rs alone | Types only. Paths, methods and request and response pairing stay hand-written in TypeScript and can drift |
| specta 2 | Still a release candidate. Its main draw, tauri-specta, leaves with Tauri |
| schemars plus `json-schema-to-typescript` | Two toolchains. Tagged enums become `oneOf`, which gives noisy TypeScript. Routes stay hand-written |
| utoipa plus `openapi-typescript` | Response types live in an attribute the compiler never checks against the handler, so the drift that matters most stays possible |
| aide | Pinned to an old schemars while the ecosystem has moved on. Pre-1.0 with a small maintainer base |

The trait and generator are about 300 lines we own. There's no OpenAPI document in v1. The registry knows every method, path and type, so it can emit one later if third-party clients appear.

## Feature modules

**The rule: a feature is one module that registers itself, and adding one edits no shared file.** A feature lives in `crates/td-companion/src/features/<name>.rs` or `<name>/`, and declares its endpoints, events, Job kinds, error enums and `sync` snapshot in one place:

```rust
pub fn feature() -> Feature {
    Feature::new("devices")
        .endpoint::<List>()
        .endpoint::<Get>()
        .endpoint::<Restart>()
        .event::<DeviceUpdated>()
        .snapshot("devices", |ctx| ctx.devices.summaries())
}
```

Two mechanisms can collect the features. #71 tries the first with a short spike and records its choice:

- **A build-script registry.** `build.rs` lists `src/features/*` and writes `$OUT_DIR/features.rs` with one `mod` per file and `pub fn all() -> Vec<Feature>`. `src/features.rs` holds only the `include!`. Keep it only if rust-analyzer resolves the generated modules: go to definition, completion and errors inside a feature file must all work.
- **Distributed registration with `inventory` (or `linkme`).** Each feature submits its `feature` function with `inventory::submit!`, and the kernel collects them with `inventory::iter`. Rust still needs a `mod` line per feature. If #71 can't generate those lines, they sit in `src/features/mod.rs`, one per line in sorted order, and that line is the only shared edit a feature makes.

**#71 chose `inventory`.** The build-script spike wasn't finished inside its time box, so the fallback applies. Each feature ends with `inventory::submit! { Registration(feature) }`, and `src/features/mod.rs` holds one `mod` line per feature in sorted order. `api::registry()` sorts features by name, so route order and the generated file don't depend on link order.

Either way:

- An `Endpoint` that's declared but never passed to `.endpoint::<X>()` is a never-built struct, so `dead_code` warns, and the zero-warning build fails.
- `Ctx` is the one shared struct on purpose. It lists domain services (the Device manager, Integrations, the Asset store, actions, Jobs, events), not features, and changes only when a new service appears.
- `api` exports only `Endpoint`, `Req`, `Feature`, `Ctx`, `ApiError`, `CommonError`, `Event`, `JobKind`, `Jobs` and the body kinds. `api::auth` is private, so a feature can't reach tokens or declare a public route.
- Blocking `td-link` work runs on each Device's actor thread or in `spawn_blocking`. Handlers never block the runtime.

## HTTP framework

axum 0.8 on tokio. tokio, hyper, tower and tower-http are already in `Cargo.lock` through reqwest, and axum itself is there through the WebDriver plugin. The Companion adds axum's own small crates, `tungstenite` for the WebSocket, and the tower-http `limit` and `set-header` features. Removing Tauri drops much more than that. The scaffold ticket measures the release binary's growth and records it. The budget is 1.5 MB.

**Measured in #71** with Rust 1.99.1 on Windows x64. Every row is a release build without test hooks:

| Binary | Default release profile | LTO, one codegen unit, `panic = "abort"` |
|---|---|---|
| Empty tokio program | 434,176 B | 249,856 B |
| tokio and `td-link` discovery | 914,432 B | 508,416 B |
| The same plus a minimal axum server with a WebSocket | 1,798,144 B | 957,952 B |
| The Companion with the #71 slice | 2,596,864 B | 1,353,216 B |
| Growth over the empty tokio program | 2,162,688 B (2.06 MiB) | 1,103,360 B (1.05 MiB) |

Under the workspace's default release profile the growth is over the 1.5 MB budget. With the size settings it's under. The workspace profile also builds the Tauri app, so #71 leaves it alone, and the packaging ticket picks the Companion's shipping profile.

**Measured again in #74.** The Device manager brings in `td-host` for the key mirror. The Companion is 4,566,016 B under the default release profile and 2,059,264 B with the size settings. That's 1,809,408 B (1.73 MiB) over the empty tokio program, so it's now over the budget even with the size settings. Most of the growth is `td-host`. The packaging ticket decides whether to trim it or raise the budget.

Rejected: actix-web (a second runtime layered on tokio), warp (filter types make per-feature routes and error mapping awkward), poem (a much smaller community, and its OpenAPI story uses the attribute-declared responses rejected above), tiny_http with tungstenite (a thread per connection, and hand-built upgrade, limits and middleware), and raw hyper (every layer above it would be ours).

**Embedded UI.** `build.rs` embeds `<ui>/dist` as an `include_bytes!` table. In development, the Vite dev server proxies `/api`, including the WebSocket, to the Companion and rewrites `Host` and `Origin` to the Companion's origin. The Companion has no dev-only origin exception that could ship.

## Test seam

`td_companion::testing::TestCompanion` is an in-process Companion, compiled under `cfg(test)` or the `test-hooks` feature. It binds `127.0.0.1:0` with a temporary data folder, in-memory keys, a fake input sink, a `FakePrompter` and `MockPorts`.

```rust
let td = TestCompanion::start().await;
let a = td.plug_device("mock0001").await;              // a mock Device with that USB serial and a v2 hello
let b = td.plug_device("mock0002").await;              // a second Device at the same time
td.plug_device_v1("mock0003").await;                   // v1 Firmware, no capabilities
td.plug_board("mock00ff", BoardKind::OtherFirmware).await;
td.unplug("mock0002").await;
td.prompter().answer_next(Answer::Once);
let job = td.call::<layouts::Push>(path(a, Landscape), layout("A")).await?;  // typed client from the Endpoint
let mut ev = td.events().await;                        // a WebSocket already past hello and sync
td.raw().get("/api/v1/devices").header("Host", "evil.example:1").send().await;  // for security tests
td.clock().advance(Duration::from_secs(121));
```

`MockPorts` implements the Device manager's `PortSource` trait, as the real port enumeration does. It replaces today's single `MOCK_PORT` and `TOUCHDECK_MOCK=1`. The Companion binary takes `--mock-devices N` under `test-hooks` for Playwright, and `TOUCHDECK_APP_DATA_DIR` for isolation.

Required integration tests in `crates/td-companion/tests/`:

1. **Two Devices.** Both connect within 2 s of `plug`. Different Layouts pushed to each stay separate. A press on one runs only its own action. A Firmware Job on one doesn't block a Layout push on the other. Unplugging one leaves the other connected.
2. **Hot-plug.** Unplug gives `link.state: lost` with `unplugged`. Replug gives `connected` within 2 s. After `devices.disconnect`, the Device stays disconnected until replug or `devices.connect`.
3. **New board.** Other Firmware gives `new-board.added`. `firmware.install` waits for native confirmation; declining fails with `firmware.declined` and writes nothing. A successful install gives `new-board.removed`, then `device.added` with `setup: "needs-setup"` and the same id.
4. **Jobs.** A second POST gives 409 `device.busy` with the first Job's id. `jobs.wait` sees `succeeded` with a typed result. The mock's `rollback` mode gives `failed` with `firmware.rolled_back`.
5. **Approvals.** A command Tile press creates one `approval.requested` and runs nothing. A second press of the same Tile adds no request. The scripted `once` runs it and sends `approval.resolved`. After 5 minutes of test clock, an unanswered request resolves as `expired`. `actions.allowed_remove` makes the next press ask again. `release_router_answers_no_approvals` passes.
6. **Auth attacks.** Each request is rejected with the stated code and never reaches a handler:
   - no token, a wrong token, a revoked token, an expired browser session (7 days of test clock),
   - a foreign `Host`, a missing `Host`, `localhost:PORT` and `[::1]:PORT`,
   - a foreign `Origin` and `Origin: null`,
   - `Sec-Fetch-Site: cross-site`,
   - a WebSocket with a foreign `Origin`, and one without the bearer subprotocol,
   - a pairing code used twice, after 120 s, and the sixth try after 5 wrong codes,
   - a browser session calling `auth.pair_code`, `system.lock`, `system.lock_renew`, `system.unlock` or `system.shutdown`,
   - a cross-site form post with `Content-Type: text/plain`,
   - an `OPTIONS` preflight, which gets no CORS headers,
   - Reset pairing, which closes open sockets with 4401 and rejects the old pairing token.
7. **Sessions.** A window session ends when its WebSocket has been closed for 60 s. All sessions end on Companion exit; after restart neither a browser token retained in `localStorage` nor a window token authenticates, and the browser must pair again. No session state is saved to disk.
8. **Runtime file**, Windows only (`#[cfg(windows)]`). After start the DACL is protected, has one entry, and the owner is the user. A junction planted at `runtime\` makes start fail. The file is gone after a clean shutdown.
9. **Contract.** `api_bindings_are_current`, `api_contract_is_documented` and `test_registry_is_consistent`.

10. **Console and input.** Each allowlisted form passes; `keys`, `touch tap`, `ui hold`, state changes, unknown commands, extra arguments, control bytes and lines over 256 bytes get `console.forbidden` and never reach the Device. None inject PC input or answer an approval. While a native dialog shows, input from every mock Device is dropped with key-mirror `outcome: "suppressed"`, never replayed. Full console works only through `test.console`; the release router lacks it, and a Companion on real ports serves it only with `--test-real-device-console`.
11. **Integration origins.** Changing scheme, host or effective port clears the old secret and reports `set: false`; same-origin path changes keep it. No credential reaches the new destination before re-entry, including concurrent refresh. Cross-origin redirects and public plain-HTTP destinations receive no credential. Explicit loopback/private-address HTTP destinations work; a hostname resolving privately does not gain the exception.
12. **Pairing delivery.** `auth.pair_code` returns only `{expires_ms}`, with no code or URL in any API response. Fake native browser and tray adapters capture delivery for tests. A new code invalidates the previous one, Reset pairing invalidates the pending code, and the Electron renderer gets no code to display or copy.
13. **Firmware consent.** Install, repair and downgrade stay `waiting_for_confirmation` with no writes until the fake native dialog accepts the exact target and bundle. Decline, or 5 minutes of test clock without an answer, gives `firmware.declined`; cancelling while waiting closes the fake dialog and writes nothing. Same-or-newer bundled updates need no confirmation. A picked bundle fails on another target, on reuse and after 10 minutes.
14. **Saturation.** Flood uploads, WebSockets and Device events separately; assert the per-session and global limits, 413/429 errors, bounded queues and approval denial/logging. Assert decoded dimensions, GIF frames/bytes, two concurrent renders and the 5 s `approvals.show` rate. After each flood, a Layout push still succeeds and capacity is reclaimed.
15. **Install lock.** A browser gets 403 `auth.forbidden`; another window cannot renew or release the owner's lock. Owner renewal extends the 10-minute lease. Session end, revoke, Reset pairing or lease expiry releases it and allows new Jobs.

Playwright then drives the Electron window and a browser page against `--mock-devices 2`. One spec pairs a browser through the link captured by the fake native browser opener; another types the code captured from the fake native tray notification. Neither obtains a pairing code from an API response.

## Versioning

- **The path prefix is `/api/v1`.** The runtime file lists supported versions (`"api": ["v1"]`), and `GET /system/ping` returns them without auth.
- **The UI and API ship together.** The UI is embedded in the Companion binary, so the first-party UI always matches its API. Two boundaries remain:
  - A tab left open across an update sees a new `hello.build` and reloads.
  - Electron's preload contract has `openTouchDeck.version = 1`. Electron checks `GET /system/ping` and shows "Restart Open TouchDeck to finish updating" if it doesn't support the Companion's API version.
- **Inside v1, changes only add.** Allowed: new routes, new optional fields, new event types, new error codes and new Job kinds. Clients ignore unknown fields and event types, and treat an unknown error code as generic. Not allowed: renaming or removing a field, changing a type, or changing what a status code means. The generated file shows such a change in review as a removed or changed line.
- **A breaking change** adds `/api/v2` next to v1 for one release, but only if third-party clients exist. Otherwise it replaces v1 in the same release, because every first-party client ships with it.
- **The Firmware protocol version** (Companion to Device, Protocol v2) is separate and never appears in routes. The API shows it only as `DeviceSummary.firmware.protocol` and in `device.needs_firmware` errors.

## How to add an endpoint

1. Find the feature module under `crates/td-companion/src/features/`. If your route belongs to a new feature, add a new module there and register it the way the feature-module rule says.
2. Declare the request and response types with `#[derive(Serialize, Deserialize, TS)]`. Name fields with the words the rest of the API uses.
3. Write `struct X; impl Endpoint for X` with `NAME` set to `<feature>.<action>`, the method, the path under `/api/v1`, the types and the feature's error enum. Slow Device work returns `JobRef` with `STATUS = 202` and a registered `JobKind`.
4. Add `.endpoint::<X>()` to the feature's `feature()`. Add `.event::<E>()` for any new event.
5. Add new error variants to the feature's enum with a `code` and a status.
6. Run `pnpm gen:api` and commit the changed `api.gen.ts`.
7. Add the route, its events and its error codes to this page. Changes only add (see Versioning).
8. Write an integration test through `TestCompanion` that calls the route with `td.call::<X>()` and checks what a caller sees.
9. Run `cargo test --workspace` and `pnpm lint`. The freshness and contract tests tell you what you missed.

Never add a public route, a CORS header, a route that answers approvals, or a route that takes Firmware bytes. Each of those needs a new ADR and a security review.

## Risks

| Risk | Status |
|---|---|
| Any auth bug lets an attacker run programs as the user, because a session could push a command Tile and approve it | **Resolved** by ADR 0006. Approvals happen only in the native dialog on the PC. An auth bug still exposes Devices, Layouts, Integration settings and key Tiles |
| A pushed Layout can hold key Tiles that type text, such as a command line followed by Enter, into whatever window is focused. They don't need approval | **Reduced.** Someone must physically tap the Tile on the Device: the console cannot fake a tap, and Device key input is suppressed during native approval dialogs. #76 should consider approval for key Tiles that type long text |
| A bearer token in `localStorage` can be stolen through an XSS bug | **Reduced.** Browser sessions end on Companion exit or after 7 days idle. A strict CSP, no `dangerouslySetInnerHTML`, the session list with Revoke, and Reset pairing. A stolen browser session can't mint pairing codes, so revoking it ends the access |
| A raw Firmware upload could flash an attacker's build | **Reduced.** Only the bundled build or a single-use, target-bound file picked on the PC; install, repair and downgrade also require native confirmation |
| Another Windows user on a shared PC squats on the port | **Reduced.** Rotation and the HMAC proof protect the window. Browser sessions exist only in Companion memory and end on exit, so a squatter that receives a stale token after restart cannot use it on the real Companion. Browsers pair again after each restart |
| Raw console commands bypass native approvals | **Resolved.** Exact read-only allowlist, printable ASCII and length limits; Device input is suppressed while a native dialog shows |
| An Integration destination change or redirect leaks credentials | **Resolved.** Secrets are origin-bound, deleted on origin change, and never forwarded across origins; public plain HTTP receives none |
| A stolen window session reads a code to mint browser access | **Resolved.** No pairing code in API responses; only native browser opening and tray delivery, one pending code, cleared on reset |
| Upload, decode, socket or approval floods exhaust the Companion | **Reduced.** Aggregate and per-session limits, bounded queues, render concurrency and saturation tests |
| A session holds the install lock indefinitely | **Reduced.** Window-only owner-bound 10-minute lease, released on session end or expiry |
| A local process guesses a typed pairing code | **Reduced.** 60-bit codes, 120 s, single use, and a lockout after 5 wrong tries |
| Browsers may drop the URL fragment when the Companion opens the link | **Resolved.** The user can type the code from the tray notification |
| A tray program's dialog may not get the foreground on Windows | **Reduced.** The tray notification and `approvals.show`. The approval waits up to 5 minutes |
| `assets.create` with `kind: "app"` takes any path, so a session can learn whether a file exists | **Accepted.** The Companion returns only a rendered icon, never file contents |
| The build-script registry may confuse rust-analyzer | **Resolved.** #71 uses `inventory` (see Feature modules) |
| `td-link`'s blocking `Session` under async handlers needs one actor thread per Device | **Resolved** in #74. Each Device's actor thread owns its session and runs requests one at a time; probes run on the scan thread |
| With Fast User Switching, two Companions try to open the same COM ports | **Reduced** in #74. A COM port opens exclusively, so the Companion that holds a Device keeps it; the other user's Companion shows it `busy` and retries with backoff up to every 10 s, taking it over only once the first lets go. Each user has their own runtime folder, file, lock and port, so the two Companions never share a pairing token |
| A browser tab can't show the app update icon, because updates live in Electron | **Open.** The browser shows nothing for now |
| The key monitor's `focus_window` may need to come back | **Open.** If so, one preload call, through security review |

### Tray and startup implementation decisions (#79)

The Companion owns the Windows tray on a dedicated message-loop thread. Open starts
`Open TouchDeck.exe` beside the Companion. Development may point Open elsewhere with
`TOUCHDECK_WINDOW_BIN`; a Companion started with `--packaged` ignores it.
Electron's single-instance handler focuses an existing window. Closing that window
revokes its session and leaves the detached Companion running.

Electron passes `--packaged` only for installed builds. The first packaged start
registers the current Companion executable with `--packaged` in the per-user Run
value `Open TouchDeck`, then writes a non-secret `startup-initialized` marker.
Later starts respect the registry, including an absent value after disabling startup.
Development and mock starts never enable startup automatically.

| Name | Method | Path | Request | Response | Access |
|---|---|---|---|---|---|
| `system.startup_get` | GET | `/system/startup` | — | `{at_sign_in: bool}` | Any session |
| `system.startup_put` | PUT | `/system/startup` | `{at_sign_in: bool}` | `{at_sign_in: bool}` | Any session |

The Windows shell receives the fragment URL directly. The Companion can't detect a
dropped fragment, so it never retries with another link; the code in the tray
notification is the fallback, as How a browser pairs says. Pairing rejection uses the
contract's combined wrong-or-expired message because both share `auth.code_invalid`.
Mock runs capture native delivery in memory. Only test-hook builds register
`test.pairing_delivery`, which requires a window session and consumes the captures;
its types appear only in `api.test.gen.ts`.

## Animated GIF Assets

An animated Asset has `kind:"gif"`, media type `image/gif` and an id equal to
SHA-256 of its normalized encoded bytes. GIF uploads and `assets.render` use the
following fitting contract; the PNG/JPEG and tdi1 implementation above remains
available. A mock Device can opt into GIF capabilities for integration tests;
real Firmware and mock Devices default to GIF off. Absence of `caps.anim` means
off even when `formats` includes `gif`.

Source limits are 16 MiB encoded, 4096 by 4096 pixels, 300 frames and 64 MiB
cumulative decoded RGBA, counted while streaming. Preparation proceeds in order:

1. Compose source frames, including disposal 2 and 3.
2. Fit with contain or cover to the actual Tile region, capped at `anim.image`.
3. Resample time to at least `min_delay_ms` per frame, preserving total duration.
4. Quantize to one global palette.
5. Encode normalized GIF as specified in `schema/protocol/link-v2.md`.

If encoded bytes exceed `asset_bytes`, step the canvas down. Enforce
`gif_pixels` as well. Refuse output exceeding `gif_frames` as too long; never
truncate silently. Identical input and target produce identical bytes and hash.
Refit after a resize, orientation change or target Device change.

Before Layout push, check every Page against `anim.page_tiles` and
`anim.page_pixels` (sum of animated canvas areas, once per Tile even for reused
Assets). Refuse the push before transferring or replacing the current Layout.
Firmware independently enforces the same budgets and validates complete Assets.

| Error code | Meaning | `details` |
| --- | --- | --- |
| `asset.too_large` | Source or fitted GIF exceeds a bound | `{limit, actual, unit, field}` with numeric limit/actual and field naming the source or capability limit. The message follows `field`: `gif_frames` and `source_frames` get the copy guide's Too long row, `decoded_rgba` its Decoded source too large row, and every other field its Too large row |
| `asset.too_short` | A multi-frame loop plays for less than `anim.min_delay_ms` in total | `{field: "duration_ms", limit, actual, unit: "ms"}`; `limit` is `min_delay_ms` and `actual` the loop's length |
| `asset.render_failed` | The GIF can't be prepared | `{reason}`: `not_gif`, `corrupt`, `timed_out` (the 30-second deadline), `invalid_limits` (the Device's `anim` caps can't be met) or `invalid` (the prepared file failed store admission). The message is always the copy guide's Preparation failed row |
| `asset.unsupported` | GIF unavailable or encoding unsupported | `{limit, actual, field, reason}`; limit/actual identify the required and observed format or capability, reason retains the wire reason when present |
| `layout.animation_limit` | A Page exceeds animation count or area | `{page, field, limit, actual}`; field is `page_tiles` or `page_pixels` |

These are Job errors for preparation/push failures. Source upload admission
continues to use the documented HTTP 413 `request.too_large`. Wire `size`,
`format`, `memory` and `full` remain typed through the API; storage and allocation
failures must not be reported as generic link text. Raw reasons go under Details.

### GIF preparation and push

`POST /assets` starts `assets.render` with:

```json
{"source":{"kind":"gif","upload":"<upload id>","device":"<Device id>","target":{"width":128,"height":128,"background":[20,30,40]}},"fit":"contain"}
```

The source variant requires an upload owned by this session, a connected Device
and its negotiated GIF capabilities. `target` names the actual Tile canvas in
pixels and its RGB background. `fit` defaults to `contain`; `cover` crops the
centre. Width and height are from 1 to 4096. Anything else gives 400
`request.invalid` with `details.part` `body` before a Job starts. The normalizer
caps them at the Device's animation canvas. The background fills pixels that turn
transparent between frames, preventing trails with disposal 1.

The Job consumes the upload once and returns the existing `AssetRef` shape with
`kind: "gif"` and the fitted width, height, byte count and SHA-256 id. The stored
file is served as `image/gif`. Cancellation before storage leaves no Asset.
There are at most two concurrent render Jobs, and only one of them prepares a GIF
at a time, because one worst-case GIF peaks near 280 MiB of memory. A second GIF
render Job waits in `queued` without holding a render slot until the first ends,
leaving the second slot open for icon renders, and can be cancelled while it
waits without consuming its upload. A cancelled render stops at its next source frame, fitted frame,
encoded frame or size attempt, so its slot frees at once. Source input is capped at 16 MiB,
300 frames, 4096 pixels per side and 64 MiB cumulative full-canvas decoded RGBA.
The normalizer checks a 30-second cooperative deadline between bounded stages
and source frames; an expired deadline fails with `asset.render_failed` and
`details.reason` `timed_out`.
The decoded cap includes every full source canvas, even for small frame patches:
a 300-frame 1024-square GIF exceeds it. Fitted RGBA is separately capped at
64 MiB. Only fully decoded normalized GIFs enter the Asset store.

Device summaries expose optional `anim`, `gif_frames` and `gif_pixels`.
Absence disables GIF preparation. Before a push sends any Asset, the Companion
validates each animated Asset's encoding, dimensions, bytes, frame count and
delay against that Device, then checks each Page's count and canvas area.
Reusing the same Asset in several Tiles counts once per Tile. A static Tile
cannot reference a GIF and a GIF Tile cannot reference tdi1. Unsupported Devices
fail with `asset.unsupported`. A push never silently converts GIFs to static
images. The link declares the detected Asset format.

`asset.refused` retains Device refusals such as `memory`, `full`, `storage` and
`busy` in `details.reason`. `format` maps to `asset.unsupported` and `size` to
`asset.too_large`. A wire size refusal cannot report the missing measurement:
its `limit` and `actual` are null, `field` is `device_limit` and `reason` is `size`.
Locally measured limits retain numeric values as specified above. Link timeouts
remain `device.timeout`. All these failures are visible on the Job.

### Asset store

The Companion keeps every prepared Asset, tdi1 or GIF, as `assets/<id>.bin` in
its data folder, and sweeps leftover `.*.tmp` files from interrupted writes at
startup. Together they may use at most 256 MiB: 1024 GIFs at the
proposed 256 KiB `asset_bytes`, or about 32 000 icons. A fixed quota gives the
same bound on every disk, and a few Devices' Layouts need far less. When a new
Asset would pass the quota, the Companion first removes the least recently used
Assets that nothing needs. It never removes an Asset that:

- a Layout the Companion stores for any Device names,
- a running Layout push is sending,
- was stored or read in the last hour. This covers the Asset a render Job just
  returned and the Assets of a Layout being edited but not yet sent.

Assets left behind by a refit or a removed Tile are swept first, oldest use
first. Files from an earlier run count as older than anything used since the
Companion started. If the sweep can't free enough room, the render Job fails
with `asset.store_full` and the Asset is not stored. Pending uploads are not
Assets; they have their own limit (Request limits).

The UI fake accepts GIF uploads and produces a small looping placeholder GIF:
the target size capped at 128 by 128, eight frames at 10 fps, coloured by the
upload, target and fit so a refit gets a new id. It does not implement the Rust
normalizer or Device admission; integration tests run those paths against the
Companion and mock Device.

### Tile key destinations

Key and consumer Tile actions accept optional `sink: "link" | "bluetooth"`. Omission
means `link` (the Companion). Device diagnostics expose the Firmware's `sinks`;
only Devices advertising `bluetooth` offer the Bluetooth destination. Layout push
checks this before transferring Assets and fails with `layout.invalid`, issue
`action.sink`, if the destination is unsupported. Bluetooth consumer usages are
limited to 1–1023 by the existing report map. Other action types have no sink.
