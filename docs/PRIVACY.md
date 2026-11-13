# Privacy and update checks

Open TouchDeck keeps Layouts, Devices, Pairings, settings and Integration secrets on
your PC. It has no analytics, telemetry, advertising or automatic crash reporting.
It does not upload those records, your files, Tile actions or usage history.
Your own Integrations can send requests and data to services you configure on your
behalf; those services have their own privacy policies.

## Native 2.0 update checks

Automatic update checks default to on. The Rust updater checks on its first enabled
poll, then at most once every six hours while the app runs. Failed checks count
toward that interval. Restarting the app resets the interval. This M5 foundation
provides the polling API and stored preference, and the native Settings screen has
the switch for it (Settings > This PC > Check for updates); connecting the updater to
native startup is later work. It makes no requests merely by being built or
loaded, and it does not install updates.

An enabled check sends HTTPS GET requests to
`https://api.github.com/repos/Duresa7/OpenTouchDeck/releases?per_page=100&page=1`.
It can request further numbered pages to find the newest stable 2.x version (up to
ten pages; reaching the limit without finishing fails the check). Drafts,
prereleases and other major versions are ignored.

Requests carry `User-Agent: OpenTouchDeck/<installed app version>`. The API request
also carries `Accept: application/vnd.github+json`; downloads use
`Accept: application/octet-stream`. There is no request body, authentication token,
cookie, Device identifier, PC name or persistent app identifier. GitHub sees the
app version, request URL and the public IP address of your connection, plus normal
HTTPS connection information. Your network's DNS resolver can see hostname lookups.

When a newer release exists, the same operation downloads its `.msi.sig` and MSI
from this repository's GitHub Releases download URLs. GitHub can redirect downloads
to its release asset hosting. These requests disclose which release and installer
you download and use the same User-Agent. The updater verifies the MSI and its
signed filename and version with the public key built into the app. Temporary
downloads are removed when the update result is dropped. It refuses unsigned,
altered, differently signed or older updates.

## Turn checks off

The native preference is `automatic_update_checks` in `app-settings.json` in the
app's configuration folder. Turn off Settings > This PC > Check for updates; the
switch writes that field and keeps any other fields in the file. If the file is
damaged, the switch is off and says so, and nothing rewrites the file. You can also
close the app and create or edit that file, preserving any other fields:

```json
{ "automatic_update_checks": false }
```

On Windows the default path is
`%APPDATA%\io.github.duresa7.touchdeck\app-settings.json`. Source builds use
`td_core::app_dirs::Dirs::current().app_settings()`; on Linux the default folder is
`$XDG_CONFIG_HOME/io.github.duresa7.touchdeck` (normally
`~/.config/io.github.duresa7.touchdeck`). Development configuration overrides are
described in [setup](setup.md).

When this setting is false, the native updater makes no update requests, including
downloads. It checks the setting before every poll. Turning it off cannot recall
a request that has already been sent. Unreadable or corrupt settings return an
error instead of silently turning checks back on.

## Current 1.x app and other requested downloads

The frozen Electron 1.x app still checks `latest.json` on GitHub at startup and
every six hours, as described in [packaging](packaging.md). The new native
preference does not change the frozen Electron updater.

Apart from update checks and your Integrations, nothing leaves the PC
automatically. Features that you explicitly ask to fetch online content also make
requests: for example, picking an online icon downloads it from its provider, and
installing a curated icon pack downloads it from the npm registry. Offline icon
pack searches and local imports stay on the PC. These requested downloads are
separate from the update-check setting. Open TouchDeck does not send Layouts,
settings or usage data with them.
