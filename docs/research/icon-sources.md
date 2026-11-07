# Icon sources for the icon picker

Research for [#70](https://github.com/Duresa7/OpenTouchDeck/issues/70), under the overhaul spec [#63](https://github.com/Duresa7/OpenTouchDeck/issues/63). It tells the icon-search tickets what Open TouchDeck may use from Simple Icons, selfh.st icons, Brandfetch and Flaticon, and how.

Researched on 2026-10-28. Figures come from the files and docs as they stood that day: Simple Icons `simple-icons@16.36.0`, selfh.st `selfhst/icons` at `main` (last commit 2026-10-28), Brandfetch docs as published.

Conventions: every fact has a source URL and a short exact quote. "Not found" means I looked and found no primary source. "Inferred" marks my reading, not a source's words. Sizes and counts marked "measured" are my own measurements of the downloaded files. Nothing here is legal advice.

## Summary

| | Simple Icons | selfh.st icons | Brandfetch | Flaticon |
| --- | --- | --- | --- | --- |
| Verdict | Ship it | Ship it | Opt-in only, with the user's own keys, and caveats | Do not use |
| Catalog license | CC0 1.0, with per-icon licenses on 224 of 3,467 icons | CC BY 4.0 | None. Terms of service only | Per-user license, attribution for free tier |
| Attribution | None required (inferred from CC0). Some individual icons carry their own license | Required: creator, license, link, and a note that we modified the icon | None asked | Required on free tier |
| Trademark | Not licensed. User must seek permissions | "Patent and trademark rights are not licensed" | Logos remain third-party property | Not licensed |
| Index you can bundle | Yes, 457 KB JSON | Yes, 879 KB JSON | No. Search API only, no persisting | No |
| Images you can bundle | Yes, 5.0 MB of SVG | Yes, 6.1 MB of base SVG, 26.7 MB of base WebP | No | No |
| Account or key | None | None | Client ID (search) and API key (images), both from a free account | API key |
| Cadence | Weekly | Several commits a month, no releases | Live | Not relevant |

## Simple Icons

### License and trademark

The repository is CC0. The package declares the same.

- Source: <https://raw.githubusercontent.com/simple-icons/simple-icons/develop/LICENSE.md>. Quote: "# CC0 1.0 Universal". The waiver covers use "for any purpose whatsoever, including without limitation commercial, advertising or promotional purposes".
- Source: <https://registry.npmjs.org/simple-icons/latest>. Field `license`: `CC0-1.0`.

CC0 covers Simple Icons' own work. It does not clear the brands' trademarks or every individual icon.

- Source: <https://raw.githubusercontent.com/simple-icons/simple-icons/develop/DISCLAIMER.md>. Quote: "Simple Icons is released under CC0 - though that doesn't mean to imply that all icons within the project are also CC0. Please see individual licenses where available."
- Same file. Quote: "Simple Icons cannot be held responsible for any legal activity raised by a brand, or users of the package. We ask that our users seek the correct permissions to use the icons relevant to their project."
- Same file. Quote: "We ask our users read these guidelines and ensure their usage of the brand's icon is in accordance with them."
- Same file, on the license data: "the absence of license data for a particular icon does not imply that the icon is not released under a license."
- CC0 itself: "No trademark or patent rights held by Affirmer are waived, abandoned, surrendered, licensed or otherwise affected by this document." (`LICENSE.md`, section 4.1.)

What the data holds (measured on `data/simple-icons.json`, 3,467 icons): 839 have a `guidelines` URL and 224 have a `license` object. The 224 are mixed. Counts by type: Apache-2.0 49, MIT 29, CC-BY-SA-4.0 29, custom 23, CC-BY-4.0 19, CC0-1.0 12, CC-BY-SA-3.0 11, GPL and AGPL variants 17 in total, and a tail that includes CC-BY-NC-4.0 (4), CC-BY-NC-ND-4.0 (3), CC-BY-NC-SA-4.0 (3) and CC-BY-ND-4.0 (2). Example: Spotify has a `guidelines` URL at `developer.spotify.com`, Proxmox one at `proxmox.com/en/news/media-kit`.

Inferred: the app can show every icon, but the 224 licensed ones need their license and `source` link surfaced. The NC and ND ones are the risky ones for an Apache-2.0 app. The recommendation below hides them by default.

### Attribution

CC0 has no attribution condition. I found no attribution requirement in the README or the disclaimer, so none is required for the unlicensed-in-data majority (inferred). A courtesy credit is harmless and the recommendation includes one. Icons with their own `license` (CC-BY-4.0, CC-BY-SA, and others) carry that license's own attribution terms.

### Getting the catalog

- Index: `data/simple-icons.json`, a JSON array. Fields per icon: `title`, `hex` (brand color), `source`, and optionally `guidelines`, `license` (`type`, optional `url`), `aliases` (keys seen: `aka` 102, `loc` 29, `dup` 15, `old` 14) and `slug` (13 icons). Measured: 457,114 bytes, 121,376 bytes gzipped.
- Slug: most icons have no `slug` field. The package derives it with `titleToSlug(title)`. Source: <https://registry.npmjs.org/simple-icons/-/simple-icons-16.36.0.tgz>, `sdk.mjs`: `export const getIconSlug = (icon) => icon.slug || titleToSlug(icon.title);`. The full table is also published as <https://raw.githubusercontent.com/simple-icons/simple-icons/develop/slugs.md> (98 KB).
- Where it lives: the npm package `simple-icons`, the GitHub repo, and jsDelivr. Verified HTTP 200: `https://cdn.jsdelivr.net/npm/simple-icons@16/data/simple-icons.json`.
- Individual images: one SVG per icon at `icons/<slug>.svg`. Source: <https://raw.githubusercontent.com/simple-icons/simple-icons/develop/README.md>. Quote: `<img height="32" width="32" src="https://cdn.jsdelivr.net/npm/simple-icons@v16/icons/[ICON SLUG].svg" />`. Each SVG is one monochrome path in a 24x24 box (measured, e.g. `spotify.svg` is 697 bytes: `<svg role="img" viewBox="0 0 24 24" ...><path d="..."/></svg>`). Tinting is therefore a fill change.
- Pinned version URLs are immutable: `https://cdn.jsdelivr.net/npm/simple-icons@16.36.0/icons/spotify.svg` returns `Cache-Control: public, max-age=31536000, s-maxage=31536000, immutable` (measured).

### APIs

There is no first-party API and no key. The README names two CDNs. Source: README. Quote: "Icons can be served from a CDN such as jsDelivr or unpkg."

- jsDelivr terms. Source: <https://raw.githubusercontent.com/jsdelivr/jsdelivr/master/README.md>. Quote: "There are no bandwidth limits or premium features, and it's completely free to use." It calls itself "a free CDN for open-source files".
- Caching and hotlinking: jsDelivr is a public CDN and the README tells users to link to it, so hotlinking and caching are allowed (inferred). I found no per-user rate limit in the README ("not found" for a stated limit).
- `cdn.simpleicons.org` is a separate colorizing service run by a third party (<https://raw.githubusercontent.com/LitoMore/simple-icons-cdn/main/README.md>). A plain `curl` returned HTTP 403 when I tried it. Its rate limits and terms: not found. Do not depend on it.

### Offline options

- Index: bundle it. 457 KB raw, about 121 KB gzipped.
- Images: bundle all if wanted. The package holds 3,467 SVGs totalling 4,993,624 bytes (measured). The tarball is 6,515,454 bytes (measured). The registry reports an `unpackedSize` of 16,110,004 for the whole package, which also holds the JS and metadata files. Fetch-on-pick is smaller.

### How current

- Source: <https://raw.githubusercontent.com/simple-icons/simple-icons/develop/VERSIONING.md>. Quote: "_Minor_ releases and _patches_ are scheduled on a weekly basis and are generally released on a Sunday." and "_Major_ releases are scheduled on a half-year basis, mainly to remove old SVGs."
- Measured from <https://api.github.com/repos/simple-icons/simple-icons/releases>: 16.36.0 on 2026-10-25, 16.35.0 on 2026-10-11, 16.34.0 on 2026-10-04. Gaps happen (no release between 2026-08-02 and 2026-08-29).
- Removed icons and changed slugs land only in a major release. The same `VERSIONING.md` says "_Major_ | Removed icons; Slug changed icons; Breaking API changes".
- README warns about floating versions. Quote: "You can use `@latest` instead to receive updates indefinitely. However, this will result in a `404` error if the icon is removed."
- To update the bundled index: bump the `simple-icons` version, regenerate the index from its `data/simple-icons.json`, record the version.

## selfh.st icons

### License and trademark

- Source: <https://raw.githubusercontent.com/selfhst/icons/main/LICENSE>. Quote: "Attribution 4.0 International". The GitHub API reports `spdx_id: CC-BY-4.0` for the repository (<https://api.github.com/repos/selfhst/icons>).
- Same file, section 2(b)(2). Quote: "Patent and trademark rights are not licensed under this Public License."
- Per-icon license or brand-guideline data: not found. The index has no such field.
- The README describes the content as "a collection of logos and icons for self-hosted (and non-self-hosted) software" (<https://raw.githubusercontent.com/selfhst/icons/main/README.md>). The logos are third-party marks, so the CC BY grant covers selfh.st's rights only (inferred).
- Compatibility: CC BY 4.0 on image data does not touch the Apache-2.0 on our code (inferred, because the icons are separate data files, not code).

### Attribution

The app, the README and the picker must credit selfh.st. Source: `LICENSE`, section 3(a)(1). Quote: "If You Share the Licensed Material (including in modified form), You must: a. retain the following if it is supplied by the Licensor with the Licensed Material: i. identification of the creator(s)..., ii. a copyright notice; iii. a notice that refers to this Public License; iv. a notice that refers to the disclaimer of warranties; v. a URI or hyperlink to the Licensed Material to the extent reasonably practicable; b. indicate if You modified the Licensed Material...; and c. indicate the Licensed Material is licensed under this Public License, and include the text of, or the URI or hyperlink to, this Public License."

- Section 3(a)(2) allows a short form. Quote: "You may satisfy the conditions in Section 3(a)(1) in any reasonable manner based on the medium, means, and context in which You Share the Licensed Material. For example, it may be reasonable to satisfy the conditions by providing a URI or hyperlink to a resource that includes the required information."
- selfh.st supplies no copyright notice or named creators in the repo beyond the README (not found), so the credit is the name, a link, the license and a modification note.
- Open TouchDeck pads, fits and tints icons, so it must say it modified them (inferred from the Companion's behaviour in #63).

### Getting the catalog

- Index: `index.json` at the repo root, a JSON array of 2,980 entries. Fields: `Name`, `Reference` (the file slug), `SVG`, `PNG`, `WebP`, `Light`, `Dark` (each `Yes` or `No`), `Category`, `Tags`, `CreatedAt`. Measured: 879,075 bytes, 62,137 bytes gzipped. No aliases and no brand color: not found. `tags.json` lists 43 tag names (715 bytes).
- Compact variant: `index-consolidated.json`, 241,856 bytes, 43,622 gzipped, each entry a positional array with no field names (measured). The field order is undocumented, so prefer `index.json` and compact it at build time.
- Where it lives: GitHub and jsDelivr. Verified HTTP 200 with `access-control-allow-origin: *`: `https://cdn.jsdelivr.net/gh/selfhst/icons@main/index.json`.
- Individual images. Source: README. Quote: "served to users directly from this repo using the jsDelivr content delivery network." Formats, same source: "SVG | Most", "PNG | All", "WebP | All", "AVIF | All", "ICO | All". Paths in the repo tree (measured): `svg/<Reference>.svg`, plus `-light` and `-dark` variants, for example `svg/2fauth.svg`, `svg/2fauth-light.svg`, `svg/2fauth-dark.svg`. Raster files sit in `png/`, `webp/`, `avif/` and `ico/` with the same names.
- SVG is missing for 469 of 2,980 icons (2,511 have it, measured from the index). PNG exists for all.

### APIs

No API and no key. The wiki documents a self-hosted container that serves the same files with colorizing (<https://raw.githubusercontent.com/wiki/selfhst/icons/Home.md>). Its default source is the jsDelivr URL: `REMOTE_URL | https://cdn.jsdelivr.net/gh/selfhst/icons@main`. Rate limits for the jsDelivr path are those of jsDelivr (see above). selfh.st states no rate limit or caching policy of its own (not found).

Colors: README says "Users looking to display the icons using colors of their choice can become a paid member of selfh.st or deploy the collection themselves". We tint locally in the Companion, which needs neither. Because the license allows modification with notice, that is permitted (inferred).

### Offline options

All sizes measured from the repo tree (<https://api.github.com/repos/selfhst/icons/git/trees/main?recursive=1>):

| Set | Files | Bytes |
| --- | --- | --- |
| Index (`index.json`) | 1 | 879,075 |
| SVG, all variants | 7,331 | 13,855,950 |
| SVG, base only (no `-light`/`-dark`) | 2,516 | 6,113,850 |
| WebP, base only | 2,986 | 26,720,914 |
| PNG, base only | 2,986 | 88,281,545 |
| Whole repo (API `size`) | n/a | about 394 MB |

Bundle the index. Do not bundle images except, optionally, the base SVGs (6.1 MB). The largest single SVG is 163,449 bytes (`viseron.svg`).

### How current

- Measured from <https://api.github.com/repos/selfhst/icons/commits>: "Icon updates" commits on 2026-10-28 (three), 2026-10-14, 2026-10-13, 2026-10-05, 2026-10-01, 2026-09-23. Index `CreatedAt` newest value is 2026-10-28.
- The repo has no tags and no releases (measured: `/tags` and `/releases` return empty arrays). `build/VERSION` holds `4.0.3`, which is the container's version, not the icon set's.
- Consequence: pin by commit SHA. jsDelivr caches branch URLs for a week. Source: <https://raw.githubusercontent.com/jsdelivr/jsdelivr/master/README.md>. Quote: "**Version aliasing** - 7 days." and "**Static Versions and commit hashes** - Effectively forever." (Branch `@main` counts as aliasing: inferred.)
- To update the bundled index: fetch `index.json` at a recorded commit SHA, regenerate, and record the SHA.

## Brandfetch

### License and trademark

There is no content license. Brandfetch sells access, and its terms say the logos are not theirs to license.

- Source: <https://brandfetch.com/terms>, section 5.1. Quote: "this license does not extend to the original logos, images, or other brand assets, which are displayed in the Content and remain property of third parties as part of the Third-Party IP."
- Same section. Quote: "Brandfetch grants the Customer a limited, non-exclusive, non-transferable, non-sublicensable license to the Services and the Content."
- Same terms, section 2.6. Quote: "Customer shall use the Services through its own employees, agents and/or duly authorised representatives only and for its own business purposes". Same section: "Customer is expressly prohibited from using the Services on behalf or for the benefit of any third-parties, or to sublicense the Services to any third parties". So each user must use their own account. Open TouchDeck must not ship a shared client ID or key (inferred). The "own business purposes" wording is also a possible snag for hobby users. Brandfetch does not say, so that stays an open question.
- Trademark. Source: <https://docs.brandfetch.com/guides/logo-fair-use.md>. Quote: "Logos are the property of their respective trademark owners. Logo usage must comply with the principles of \"fair use,\" which allow referencing a brand without endorsement, misrepresentation, or alteration." The same guide says: "Commercial Use: Using a third party logo to on your own products or services is not fair use."
- Product-shape limit. Source: <https://docs.brandfetch.com/logo-api/overview.md>. Quote: "You cannot replicate the core user experience of Brandfetch." The listed bad case is "A logo search that only returns logos from Logo API." An icon picker that is one feature of a larger app, where Brandfetch is one optional source, fits the good examples (inferred).

### Attribution

None. Source: <https://docs.brandfetch.com/logo-api/overview.md>. Quote: "Logo API is free to use and we don't ask for any attribution." Same sentence in <https://docs.brandfetch.com/brand-search-api/overview.md>: "Brand Search API is free to use and we don't ask for any attribution."

### Getting the catalog

There is no downloadable index and no names, slugs or colors dump. Search is a live call.

- Brand Search API. Source: <https://docs.brandfetch.com/reference/brand-search-api.md>. `GET https://api.brandfetch.io/v2/search/{name}?c={clientId}` returns an array of `icon` (URL), `name`, `domain`, `claimed`, `brandId`. Per the overview, `limit` above 5 gives a 400: "A query parameter is unknown, or `limit` is above 5."
- Logo API. Source: <https://docs.brandfetch.com/logo-api/overview.md>. `https://cdn.brandfetch.io/domain/nike.com?c=BRANDFETCH_CLIENT_ID`, WebP by default, with `.png`, and `svg` for `logo` and `symbol`. `type` is `icon`, `symbol` or `logo`. Size is clamped "between 16 and 2048 pixels" (<https://docs.brandfetch.com/logo-api/parameters.md>).
- Brand API. Source: <https://docs.brandfetch.com/brand-api/overview.md>. Needs an API key sent as a Bearer token. It returns `logos` and shortcut image fields. Quote: "The client ID is required, and a request without it is refused. Its `t` parameter changes when the image does, so you can cache the image for as long as you like. The image does not count towards your quota. It works in an `<img>` tag, from a browser, or from your server."

### APIs: account, rate limits, caching, hotlinking

| | Logo API | Brand Search API | Brand API |
| --- | --- | --- | --- |
| Credential | Client ID from a free account | Client ID from a free account | API key from the Developer Portal |
| Free allowance | 1,000,000 requests a month | 500,000 requests a month | "When you sign up for a free developer account, you get 100 free requests." |
| Throughput | 1,000 per 5 min per IP, 2,400 per 5 min per customer | 200 per 5 min per IP, 6,000 per 5 min per client ID | "sustained 100 requests/second ... 30,000 requests / rolling 5 minute hard limit" |
| Programmatic fetch | Not permitted | n/a | Yes, with the key |
| Caching | Not without a custom deal | Not allowed | Image URLs cacheable per the docs, 30 days per the terms |

Sources and quotes:

- Logo API, from <https://docs.brandfetch.com/logo-api/overview.md>: "**Programmatic access** to logo images is not permitted and may result in rate limiting or blocking. Scraping logos will also lead to a block." and "If your use case requires caching, please contact us for a custom setup." and "We require logo links to be directly embedded in your applications." A request that "looks programmatic rather than a logo embedded in a page" is answered with a 302 (`x-bf-error: automated_traffic`). Rate limits: "**1,000,000 requests per month** for free", "1,000 requests every 5 minutes per IP", "2,400 requests every 5 minutes per customer".
- Brand Search, from <https://docs.brandfetch.com/brand-search-api/overview.md>: "The API should be used directly as is, with all data fetched live and not altered or persisted. Your users should make requests to the API directly from their browsers." and "**The logo image URLs provided by the Brand Search API must be hotlinked**. Other data, such as brand names, should not be cached ... Image URLs expire after 24 hours and must be refetched." Limits: "**500,000 requests per month** on the free plan", "200 requests every 5 minutes per IP address", "6,000 requests every 5 minutes per client ID". It advises "a debounce strategy between keystrokes". Per-IP 200 per 5 minutes "allows roughly 30 search sessions in a 5-minute period for a single visitor".
- Brand API throttle and quota, from <https://docs.brandfetch.com/brand-api/overview.md>: "throughput is limited to a sustained 100 requests/second with some flexibility to accommodate bursts (30,000 requests / rolling 5 minute hard limit)". Whether the 100 free requests are one-time or monthly: not stated (read as one-time, inferred).
- Caching terms conflict. The docs allow caching Brand API images "for as long as you like". The Terms of Service, section 5.1, give a shorter license. Source: <https://brandfetch.com/terms>. Quote: "a limited, non-exclusive, non-transferable, revocable license to download, store, and cache the Content (the \"Cached Content\") retrieved via API calls for a maximum period of thirty (30) days from the date of retrieval", and "The Customer is required to make a new API call at least once every thirty (30) days to refresh the Cached Content". Section 2.1 says caching comes "Upon specific written agreement between the Parties". Treat the stricter reading as the rule. A conflict like this is one for the maintainer to settle with Brandfetch.
- Scraping, in the terms' section on the Customer's obligations (number not captured): the terms forbid "crawl or scrape, whether manually or by automated means (including bots, scrapers, and spiders)". Do not build an index by crawling searches.

### Offline options

None that are permitted. The index cannot be built or bundled (no dump, no persisting search data). Images fetched through the Brand API can be kept at most 30 days under the terms, then must be refetched or deleted. An Asset stored on a Device outlives that (inferred), which is the open risk below.

### How current

Live data, no versions. Source: <https://docs.brandfetch.com/logo-api/overview.md>. Quote: "a simple CDN link that gives you access to any brand's latest logos". Nothing to update in the app.

### What this means for Open TouchDeck

The spec says Brandfetch is queried "only with a user key" (#63). The terms allow that shape if the user is the Customer. The constraints that follow:

- Search must run live from the UI with the user's client ID, and result thumbnails must be the returned URLs, hotlinked, not copied.
- The Companion cannot fetch the Logo API (programmatic access is refused). Turning a pick into an Asset means a Brand API call with the user's API key, which is a second credential.
  - **Settled in #127.** The Brand API answer's logo files are served from `cdn.brandfetch.io` with a signed `c` parameter, and the Brand API docs say each one "works in an `<img>` tag, from a browser, or from your server" and "does not count towards your quota". The Companion downloads only the `src` addresses a Brand API answer gives, unchanged and without the key. It never builds a Logo API address itself and never fetches search thumbnails.
- Searches and names must not be cached.
- An Asset made from a Brandfetch logo is kept past 30 days on the Device. That sits outside the cached-content license unless Brandfetch agrees. This research did not settle it.
  - **#127's answer.** The Companion makes "a new API call at least once every thirty (30) days": from day 25 it fetches each Brandfetch logo again, checking hourly. A changed logo becomes a new Asset that the Layout editor swaps into its drafts, so the next push carries it. A logo with no successful fetch for 30 days loses its cached original on the PC, is never fetched again, and Settings reports it. The Companion never deletes an Asset from a Device, so the Device copy until the next push is still the open point for the maintainer to settle with Brandfetch.

## Flaticon

The spec lists Flaticon as out of scope. This section records why, for the icon-search tickets.

Source note: `flaticon.com` and `magnific.com` returned HTTP 403 to `curl` and WebFetch from this machine, so the license text below comes from the Internet Archive's copy of the primary page, <http://web.archive.org/web/20261004095321id_/https://www.flaticon.com/legal> (snapshot 2026-10-04). The live page is <https://www.flaticon.com/legal>. Flaticon is operated by Magnific (formerly Freepik, inferred from the page text "made by Magnific from flaticon.com").

### License and trademark

- License is per user, revocable and non-transferable. Quote (section 8.1): "the Company grants the User a Flaticon License under which the User is authorized to download, use and modify the Flaticon Content on a device owned or controlled by the User on a non-transferable, limited, exclusive, revocable and worldwide basis".
- No redistribution inside an icon product. Quote (section 8.3): "the content in the Flaticon Content or any work derived therefrom, whether entirely or in part, is not used or included in a database, file or any other product or service which offers icons or images". An icon picker inside Open TouchDeck is that (inferred).
- No sublicensing. Quote: "the User does not resell, assign, transfer or sublicense the Flaticon Content or works derived from the Flaticon Content". Open TouchDeck is open source and redistributed, so every copy would need its own license (inferred).
- Trademarks. Quote: "the Flaticon Content is not used (whether entirely or in part) on a trademark or as a part thereof". Brand logos on Flaticon also fall under editorial-only use: "where within the same there are distinctive signs, recognizable products ... the User shall only be entitled to use it for such editorial use" (section 8.1).

### Attribution

Required on the free tier. Quote (section 8.1): "the authorization to use the content in the Flaticon Content shall be free of charge under the Flaticon License where any use thereof by the User is done by duly crediting said content to the Website/Company and, in any event, to the Collaborator". The page's attribution snippet reads "made by Magnific from flaticon.com". Premium removes it: "In order to benefit from the Service of using the Flaticon Content without the aforementioned crediting, the User must purchase a subscription".

### Getting the catalog and APIs

- No public index or dump. The page advertises "18.0M+ icons".
- API: the Icons API lists, fetches and downloads icons. Source: <https://docs.magnific.com/api-reference/icons/icons-api.md>. Quote: "All requests require an API key via the `x-magnific-api-key` header." Use case listed: "Build an icon picker into your design tool or CMS". On the license it says only: "Refer to the [License agreement](https://www.magnific.com/legal/terms-of-use#api-services)".
- Rate limits. Source: <https://docs.magnific.com/ratelimits.md>. Quote: "**50 hits per second** over a **5-second period**", "**10 hits per second** averaged over a **2-minute period**", "**30,000 requests per minute (RPM)**" per API key.
- API services terms (<https://www.magnific.com/legal/api-services>): not found. The page returns HTTP 403 and the Internet Archive has no copy. So caching and hotlinking terms for the API are not found.
- Offline: not permitted for a distributed catalog, per the license above.

Verdict: do not use. Even a user-keyed integration would need terms I could not read, and the free license needs attribution on every Tile and bans use in an icon-offering product.

## Recommendation

### How search ships

1. **Bundle both indexes, generated at build time.** A script reads the `simple-icons` npm package and selfh.st `index.json`, then writes one compact JSON file the Companion ships and the UI searches. Keep per icon: source (`simple-icons` or `selfhst`), display name, slug or `Reference`, aliases (Simple Icons `aka`, `loc`, `dup`, `old`), brand hex (Simple Icons only), `Tags` and `Category` (selfh.st only), whether SVG exists, whether light and dark variants exist, and for Simple Icons the license type, license URL and `guidelines` URL. Record the Simple Icons version and the selfh.st commit SHA in the file. Expected size is well under 1.4 MB raw (the two sources are 457 KB and 879 KB before trimming) and under 200 KB gzipped (121 KB plus 62 KB gzipped before trimming).
2. **Do not fetch the index at run time in v1.0.0.** A bundled index means search works offline and never depends on a CDN. Updating it is a release task: re-run the script before each release.
3. **Fetch images on pick, from jsDelivr, by the Companion.** Use version-pinned URLs recorded in the index file:
   - Simple Icons: `https://cdn.jsdelivr.net/npm/simple-icons@<version>/icons/<slug>.svg`. Immutable for a year.
   - selfh.st: `https://cdn.jsdelivr.net/gh/selfhst/icons@<commit-sha>/svg/<Reference>.svg`. Use `png/` or `webp/` when the index says SVG is missing. Add `-light` or `-dark` when the picker offers them.
   - Pinning means an icon removed upstream later still loads. A removed icon stays in the bundled index until the next release.
4. **Cache on disk with a size cap.** Store fetched files by source, version and slug. The spec asks for a size cap. Suggested: 20 MB, least recently used first. At about 1.5 KB per Simple Icons SVG and 2.4 KB per selfh.st base SVG (6.1 MB over 2,516 files, measured), 20 MB holds thousands. The picked image is rendered to the Tile's size by the Companion and stored on the Device as an Asset, so the cache only speeds up re-picks.
5. **Update cadence.** Re-run the index script at each release. Simple Icons moves weekly and selfh.st several times a month, so a quarterly release or a monthly CI job that opens an index-refresh PR keeps it fresh enough. A run-time refresh can come later without changing the shape.
6. **Trademark handling.** Show a one-line notice in the picker. For Simple Icons entries with a `guidelines` URL, show a "Brand guidelines" link on the icon's detail. Hide the Simple Icons icons whose license type contains `NC` or `ND` by default, because they restrict commercial use or modification (inferred). The 13 icons affected are CC-BY-NC-4.0, CC-BY-NC-ND-4.0, CC-BY-NC-SA-3.0, CC-BY-NC-SA-4.0 and CC-BY-ND-4.0. For icons under CC-BY, CC-BY-SA, GPL and similar, show license and `source` on the detail.
7. **Brandfetch is off by default and uses the user's own keys.** It lives in Settings, not Integrations. Settings holds two fields, a client ID (search) and an API key (image download), the key stored in the system keychain. Search runs from the UI with debounce, thumbnails are hotlinked URLs, and nothing from search is cached. On pick the Companion calls the Brand API with the user's key, renders the Asset, and does not keep the raw download beyond the cache rules above. Ask the maintainer to settle the 30-day question with Brandfetch before v1.0.0 (see the open risk above). If it is not settled, ship Brandfetch search without "add as Asset" and let users save the image themselves and upload it.
8. **Flaticon: not used.** No further work.

### Attribution text

No attribution is required for Brandfetch and none is legally required for Simple Icons' CC0 majority. CC BY 4.0 requires it for selfh.st. Show it in both places, so the app is covered wherever the icons are shown.

README, under a heading `Icon sources`:

```markdown
## Icon sources

The icon picker searches [Simple Icons](https://simpleicons.org) (CC0 1.0) and [selfh.st/icons](https://selfh.st/icons) by selfh.st (licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)). Icons are resized, padded or tinted to fit your Tile, so they are modified from the originals. Logos are trademarks of their owners. Open TouchDeck is not affiliated with or endorsed by them. Brandfetch logos appear only if you add your own Brandfetch keys.
```

Picker footer, one line with the links:

```text
Icons: Simple Icons (CC0) · selfh.st/icons (CC BY 4.0, modified) · Logos belong to their owners
```

Settings, About, under "Third-party notices", the same paragraph as the README, plus a list of the Simple Icons icons with their own licenses generated from the index (name, license type, `source` URL).

## Open points for the maintainer

- Brandfetch 30-day cache license against Assets stored on a Device. Ask Brandfetch, or ship without "add as Asset".
- Whether to hide NC and ND Simple Icons icons by default (recommended) or show them with their license.
- Index refresh: release-time only (recommended for v1.0.0) or a monthly CI job.
