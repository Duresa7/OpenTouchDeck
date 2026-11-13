# Windows installer and app updates

The 2.0 updater lives in [`td-updater`](../app/crates/td-updater/README.md). On
Windows, `td-app` checks after it starts and then about once a day, unless the
user turned checks off. It selects a stable 2.x GitHub release, downloads its MSI
and verifies it with the existing updater key. Installing re-verifies the MSI, runs
Windows Installer and quits the app. The opt-out preference and what a check sends
are described in [PRIVACY](PRIVACY.md). The update UI remains M5 work.

The updater defends against a network attacker and against a release asset that
the updater key did not sign. Code already running as the same Windows user is
outside its threat model: such code can replace the staged MSI between verification
and Windows Installer, but it can just as well replace the installed `td-app.exe`,
which lives in the same user's writable folders. Other users can't reach either,
because the staging folder is in the user's own `%TEMP%`.

End users download the [Windows x64 installer](https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/OpenTouchDeck-windows-x64-setup.exe) from the latest GitHub release and run it. It installs for the current user only and needs no administrator rights, Node, Rust, Python or ESP-IDF.

The installer on Releases today is the 1.x installer. Its source, recipe and release workflows are not on `main`: they live on the [`release/1.x` branch](https://github.com/Duresa7/OpenTouchDeck/blob/release/1.x/docs/packaging.md), whose copy of this page describes what the installer holds, how its update is signed and how a 1.x release is made ([ADR 0012](adr/0012-electron-and-react-leave-main-before-cutover.md)). A `vX.Y.Z` tag pushed on that branch runs its **windows release** workflow from the tagged commit.

The rest of this page describes Open TouchDeck 2.0, which ships as an MSI.

## Replacing the maintainer's 1.x install

The maintainer has the only 1.x app install ([ADR 0013](adr/0013-replace-1x-by-hand-for-2x.md)). On Windows 11, replace it in this order:

1. Quit Open TouchDeck, then uninstall 1.x from Windows **Installed apps**. When the uninstaller asks whether to delete Layouts and settings, choose **No**. Do not pass its `--delete-app-data` option.
2. Download the 2.0 MSI from its GitHub release and run it as the same Windows user. The MSI refuses to install while the 1.x uninstaller remains in the shared `%LOCALAPPDATA%\Programs\Open TouchDeck` folder.

The 1.x uninstaller keeps the app's local and roaming data folders when deletion is declined. The 2.0 Companion uses the same `io.github.duresa7.touchdeck` folders for its data and configuration, so the saved files remain in place; there is no copy or conversion step. Integration secrets and Pairing keys use the same `touchdeck` entries in Windows Credential Manager and stay there when 1.x is uninstalled. The uninstaller removes the old start at sign-in marker, so 2.0 sets up its own startup preference on first packaged launch. This replacement has no automatic updater path. Windows 10 users stay on 1.x because the MSI requires Windows 11.

## The 2.0 release pipeline

Open TouchDeck 2.0 is one Rust program, installed on Windows 11 by an MSI. Its releases come from **release** (`.github/workflows/release.yml`), which a `v2` tag on `main` starts. A 1.x tag starts the 1.x workflow on `release/1.x`.

The MSI ships `td-app.exe`, which runs the Companion in-process; there is no separate Companion program. Its Start menu shortcut passes `--packaged` and carries `io.github.duresa7.touchdeck` as its AUMID. The app registers start at sign-in on its first packaged start. Uninstall removes its Run and StartupApproved values and leaves user data alone. The MSI refuses coexistence with 1.x in the shared installation folder.

### Stages

```text
build -> sign hook (programs) -> package -> sign hook (installer) -> updater signature -> verify -> publish
```

| Stage | Job | What it does |
|---|---|---|
| 1. Build | `build`, windows-latest | For a tag: checks that the tag is annotated, is `v` plus the version of `td-app`, and that its commit is on `main`. Installs the Rust toolchain `app/rust-toolchain.toml` pins, runs `cargo build --release --locked -p td-app`, and checks the program's version resource and that it is a Windows GUI program (`app/scripts/check_exe.ps1`) |
| 2. Upload | same job | Uploads the unsigned program as the workflow artifact `unsigned-binaries` |
| 3. Sign the programs | `sign hook (programs)` | Has the selected provider sign every program, or passes them through. Uploads `binaries` |
| 4. Package | `package`, windows-latest | Builds `OpenTouchDeck-X.Y.Z-windows-x64.msi` with WiX from `app/installer/OpenTouchDeck.wxs` and the programs in `binaries`. Uploads `unsigned-installer` |
| 5. Sign the installer | `sign hook (installer)` | The same hook for the MSI. Uploads `installer` |
| 6. Update signature | `updater signature`, ubuntu-24.04 | Signs the MSI with the updater key ([ADR 0005](adr/0005-updater-keeps-the-signing-key.md)) through `.github/release-signer` |
| 7. Verify | three jobs | See [What verify proves](#what-verify-proves) |
| 8. Publish | `publish a draft release`, ubuntu-24.04 | For a tag only: uploads the MSI, its `.sig` and `SHA256SUMS` to a **draft** release with the tag's message as its notes, then checks the uploaded names and sizes. It never publishes the draft |

Rules every job keeps, so a signing service can trust the result:

- **GitHub-hosted runners only.** No job runs on anyone's own PC.
- **No caches.** A release job restores no Rust or tool cache, so nothing from an earlier run reaches a release. The pull request checks use caches; the release doesn't call them with caches on.
- **Releases come only from tags on `main`.** `build` refuses any other tag.
- **Third-party actions are pinned to a full commit SHA**, with the version in a comment. `build` uses none besides GitHub's own checkout and artifact actions.
- **Each job has the least permissions it needs.** Only `publish` may write to the repository. Only the update signature job, and a sign hook with a provider selected, enter the `release` environment, which holds the secrets, allows only `main` and `v*` tags, and waits for the maintainer's approval.

### A release and a dry run

A **release** is a push of a `v2` tag:

```sh
git switch main
git pull --ff-only
git tag -a v2.0.0 -m "<notes>"
git push origin v2.0.0
```

Approve the `release` environment when the run asks, then review the draft on the Releases page. If the source needs a fix, ship the next version instead of moving the tag.

A **dry run** runs the same jobs and never publishes. Start one from the Actions tab or with:

```sh
gh workflow run release.yml --ref <branch>
```

A pull request that changes the pipeline's own files (the two workflows, `app/installer/`, `.github/release-signer/` and the two check scripts) starts a dry run too. A dry run differs from a release in three ways: it never signs with Authenticode, it makes the update signature with a throwaway key that is deleted in the same step, and it has no publish job. It needs no secret and no approval, and its artifacts expire after three days.

### Sign hooks

`.github/workflows/sign-hook.yml` is the Authenticode step. The pipeline calls it twice, for the programs and for the MSI, because Windows checks both: the installer when it runs, the programs each time they start.

The repository variable `CODE_SIGNING_PROVIDER` selects the provider. It is unset today, so both hooks pass their files through unsigned and say so in the run summary: 2.0 is unsigned until a provider accepts the project. To turn signing on, set the variable and uncomment that provider's block in the hook. Nothing else in the pipeline changes.

| `CODE_SIGNING_PROVIDER` | Provider | What the block needs |
|---|---|---|
| unset | none | Nothing. Files stay unsigned |
| `signpath` | [SignPath Foundation](https://signpath.org), free for open-source projects | Secret `SIGNPATH_API_TOKEN` in the `release` environment. Variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`. Two artifact configurations in SignPath, `programs` and `installer`. SignPath fetches the unsigned artifact from GitHub by its id, so the upload in stage 2 comes before signing |
| `azure` | [Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) | Variables `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT_NAME`, `AZURE_CERTIFICATE_PROFILE_NAME`, and `id-token: write` on the job. It signs in with OpenID Connect, so there is no stored secret |

With a provider selected, the hook fails unless every program and installer comes back with a valid signature. Setting the variable without uncommenting the block therefore stops the release instead of publishing unsigned files as signed. The [code signing policy](code-signing.md) says what may be signed and who approves it.

### The MSI

`app/installer/OpenTouchDeck.wxs` is the whole installer.

- **For the current user.** The package has `Scope="perUser"`: it installs to `%LOCALAPPDATA%\Programs\Open TouchDeck`, registers with Windows Installer for the current user only, needs no administrator rights and shows no elevation prompt.
- **Windows 11 only.** Windows 11 starts at build 22000. Windows Installer's own version properties report Windows 8.1 on every later Windows, so the package reads `CurrentBuildNumber` from the registry and refuses a lower build with "Open TouchDeck needs Windows 11, and this PC runs an older Windows. Open TouchDeck 1.x works on Windows 10: download it from the Releases page."
- **Not beside 1.x.** Open TouchDeck 1.x installs to the same folder with its own installer. The package refuses to install when it finds the 1.x uninstaller there, and says to uninstall 1.x first.
- **One name and version.** The product name is `Open TouchDeck`, the manufacturer `Duresa Kadi` and the version the version of `td-app`: the same values as the program's version resource (`app/crates/td-app/build.rs`). An MSI version is three numbers, so a version such as `2.0.0-rc.1` installs as `2.0.0`.
- **Updates.** A newer package removes the older one first, and an older package refuses to install over a newer one. Windows finds the installed copy by the package's `UpgradeCode`. Never change it.

### WiX version and licence

The package job installs [WiX](https://wixtoolset.org) **7.0.1** as a .NET tool, pinned by version, and builds with `wix build -acceptEula wix7`.

WiX's source is open (MS-RL). Since WiX 6 its official binaries come under the [Open Source Maintenance Fee](https://opensourcemaintenancefee.org) agreement, and WiX 7 refuses to build until that agreement is accepted, which is what `-acceptEula wix7` does. The fee applies only to users who use WiX as part of revenue-generating activities **and** have an annual gross revenue of US$10,000 or more. Open TouchDeck is a free, Apache-licensed project with no revenue, so it is exempt and owes no fee.

- **If that changes.** If the project, or whoever builds its releases, starts earning US$10,000 or more a year from work that uses WiX, sponsor the WiX project as the agreement asks, or build WiX from its source, which the agreement leaves free.
- **Why not an older WiX.** WiX 5.0.2 is the last version from before the fee and needs no acceptance, but it is no longer maintained. A free, current, maintained tool with one explicit switch is the better trade.
- **Anyone else who builds the installer** accepts the agreement for themselves and must check their own position.

### What verify proves

Three jobs run on the signed installer, on runners that built none of it:

- **`verify the update signature`** (ubuntu-24.04) checks the `.sig` with the `minisign` program from Ubuntu, a second implementation, so the repo's signer does not check itself. A release must verify against the committed public key, `.github/release-signer/updater.pub`, the key every installed copy trusts. A dry run must verify against its throwaway key, and the committed key must reject it. Either way the signature must name this file and this version, and a copy with one byte changed must fail.
- **`verify install and uninstall (windows-latest)`** checks the Authenticode state (valid with a provider, unsigned without), then runs `app/scripts/msi_check.ps1`. It reads the package's name, version, `UpgradeCode` and per-user flags; expects the refusal beside a 1.x install; installs with `msiexec /qn` and a log; checks the installed program byte for byte and its version resource; checks that Windows Installer lists the product as installed for the current user and not for the whole PC; then uninstalls and expects the folder, the registration and the registry keys gone.
- **`verify an older Windows refuses it (windows-2022)`** runs the installer on Windows Server 2022, build 20348, which is below 22000 as every Windows 10 is. It expects exit code 1603, the Windows 11 message in the log, and nothing installed.

`msi_check.ps1` refuses to run outside GitHub Actions, because it writes the real per-user registry. It uploads a summary and logs with the runner's user and computer names removed.

### The updater public key

The public half of the updater key is committed at `.github/release-signer/updater.pub`, with the same value as the repository variable `TAURI_UPDATER_PUBLIC_KEY` that the 1.x build reads. It is public by design, and committing it lets anyone check a release's `.sig` from a clone. The private half stays a secret in the `release` environment. Never regenerate either: every installed copy trusts this key.

### Not done yet

- **The 1.x to 2.0 move.** A 2.0 release has no `latest.json`. Installed 1.x copies read that file and run what it names as the 1.x installer, so publishing one for an MSI would break their updates. The maintainer replaces 1.x by hand as described above.
- **Update UI.** The native host checks after startup and about daily on Windows, with status in the Companion snapshot. `updates().install_now(&handle)` reverifies the staged MSI, launches Windows Installer and requests the normal bounded shutdown. The update UI is separate work.
- **Firmware.** The 1.x release builds the Firmware and ships `touchdeck.tdfw`. The 2.0 pipeline doesn't yet.
- **Pre-release versions.** Two release candidates of one version get the same MSI version, so one does not replace the other.
- **A signing provider.** Apply to one, then set `CODE_SIGNING_PROVIDER`. Once files are signed, remove the draft notice from the [code signing policy](code-signing.md) and link the policy from the release notes.
