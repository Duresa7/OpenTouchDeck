# Code signing policy

> **Draft: not in effect yet.** No Open TouchDeck program or installer carries an Authenticode signature today, and this project has not yet been accepted by a signing service. This page is the policy the project will follow once signing is on, written to meet the [SignPath Foundation conditions](https://signpath.org/terms). Until this notice is removed, a file that shows an Open TouchDeck publisher signature did not come from this project.

When signing is on, this line applies: free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

## What gets signed

Only files built from the source in this repository, for the Windows release of Open TouchDeck 2.0 and later:

| File | What it is |
|---|---|
| `td-companion.exe`, and any other program the app's own source builds | The app |
| `OpenTouchDeck-<version>-windows-x64.msi` | The installer, which holds those programs |

Every signed file names the product **Open TouchDeck** and carries the version of its release. The programs say so in their Windows version resource and the installer in its product name and version. The release workflow checks the programs' version resource before they are signed and the installer's name and version in its verify stage. With SignPath, the signing policy is also set to refuse a file that says otherwise.

Programs and libraries from other projects are never signed with this project's certificate. The installer may carry them unsigned, or signed by their own publishers.

Open TouchDeck 1.x releases are not signed and will not be.

## Where a signed build comes from

A signed file comes from one place: the **release** workflow in this repository ([`.github/workflows/release.yml`](../.github/workflows/release.yml)), run by GitHub for a `v2` tag.

- The tag is annotated and its commit is on `main`.
- Every job runs on a GitHub-hosted runner. No job restores a cache, so nothing from an earlier run reaches the build.
- The workflow uploads the unsigned files as a workflow artifact. With SignPath, the signing service fetches them from GitHub itself and checks that this repository's workflow built them from the tagged commit. With Azure Artifact Signing, the workflow sends only the files' digests to be signed, so these guarantees rest on the workflow itself: it signs only for a `v2` tag on `main`, in the `release` environment the maintainer approves.
- Nothing built on a developer's PC is ever signed. Runs for a pull request or from the Actions tab are dry runs: they never sign and never publish.
- The workflow ends with a draft release. The maintainer reviews the draft before publishing it.

[Packaging and releases](packaging.md#the-20-release-pipeline) describes each stage.

## Team roles

Open TouchDeck has one maintainer, who holds every role.

| Role | Who | What the role does |
|---|---|---|
| Author | [Duresa Kadi](https://github.com/Duresa7) | May change the source without another person's review. Nobody else has write access to the repository |
| Reviewer | [Duresa Kadi](https://github.com/Duresa7) | Reviews every pull request from anyone else before it merges. This includes changes to build scripts and workflows |
| Approver | [Duresa Kadi](https://github.com/Duresa7) | Approves each signing request by hand, one release at a time |

The maintainer signs in to GitHub and to the signing service with multi-factor authentication.

## Privacy

Open TouchDeck has no account, no telemetry, no analytics and no crash reporting. It sends nothing about you or this PC to the project or to anyone else. It keeps Layouts and settings on this PC, and Integration secrets in the system keychain.

It contacts other computers in these cases only:

- **Update checks.** While the window is open, Open TouchDeck asks GitHub for the latest release's update file, about every six hours and when you choose **Check for updates**. It downloads an update only when you choose **Update**. GitHub sees the request as it sees any download; see the [GitHub privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).
- **Icons you pick.** Picking a catalog icon that isn't on this PC yet, or downloading an Icon pack, fetches it from the [jsDelivr](https://www.jsdelivr.com/terms/privacy-policy-jsdelivr-net) content network or the [npm registry](https://docs.npmjs.com/policies/privacy).
- **Services you set up.** An Integration, an HTTP value or Brandfetch logos contacts that service, or the address you gave it, with the keys you gave it. Each is off until you turn it on.

Open TouchDeck starts when you sign in to Windows; **Start at sign-in** in Settings turns that off. To remove it, uninstall it from **Installed apps** in Windows Settings. [Packaging and releases](packaging.md) lists what an install adds and what an uninstall removes.

## The update signature is separate

Every release installer also has an update signature, made with the project's own updater key ([ADR 0005](adr/0005-updater-keeps-the-signing-key.md)). The app checks it before it runs an update. That key is not a code signing certificate and Windows does not look at it: it stays in use whether or not Authenticode signing is on. Its public half is committed at [`.github/release-signer/updater.pub`](../.github/release-signer/updater.pub).

## Report a problem

If a file signed for Open TouchDeck does something this policy rules out, or you think a signature was misused, report it privately with **Report a vulnerability** on the repository's [Security tab](https://github.com/Duresa7/OpenTouchDeck/security). The maintainer will look into it and work with the signing service, which can revoke the certificate.
