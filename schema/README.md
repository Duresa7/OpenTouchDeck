# TouchDeck layout format

[layout.schema.json](layout.schema.json) defines version 1 in JSON Schema draft
2020-12. Layouts describe pages on the deck. They contain no executable code.
The app executes the actions that require the PC.

App-side actions and widgets dim on the deck while the app is offline.

## Validator commands

The validator uses Node.js with ES modules and Ajv 8. Its only installed packages
are Ajv and Ajv's dependencies. From the repository root, the commands are:

```sh
npm ci --prefix schema
node schema/check-layouts.mjs
node --test schema/
node schema/check-layouts.mjs path/to/layout.json
```

Every invocation checks all JSON files recursively under `schema/examples/`,
plus the supplied file paths. Repeated paths count once. Paths supplied on the
command line are relative to the current directory. The bundled examples resolve
relative to the script, so the command also works from another directory.

The command prints one `ok` line for each example, then a summary like this one:

```text
2 valid, 38 invalid as expected, 0 problems
```

The final line is the `wall time` in milliseconds. The time includes Node startup,
imports, schema compilation, file reads, parsing, validation, and output through
the summary. Exit code `0` means all checks passed. Exit code `1` means at least
one file failed, an invalid example passed or failed with a different rule,
or a file could not be read. The validator never launches programs or sends keys.

Each failure line starts with the rule key, as `tile.overlap: page "p" tiles "a"
and "b" overlap`. The key is the contract. The text after it is for people and may
change.

`validateSource(bytes)` checks the raw bytes of a layout file. `validateLayout(value)`
checks a parsed value and cannot see the file's text, so it does not check the
UTF-8, duplicate-member, or integer-form rules. Both return an array of
`{ key, message }` objects, empty for a valid layout, and never change their input.
`checkLayouts(paths, write)` checks the example files and extra paths, writes
status lines, and returns the exit code. `ruleKeys` is the set of rule keys.

## Reading a layout file

Every reader of a layout, which means this validator, the firmware parser in M2-04,
and the Rust validator in M2-10, rejects a file that breaks one of these rules.

- A layout file is well-formed UTF-8 JSON. A byte order mark is not valid JSON.
- Every string, member names included, is well-formed Unicode. A lone surrogate
  escape such as `"\ud800"` is invalid.
- A member name appears at most once in each object. A reader rejects a repeat. It
  never picks the first or the last value. Names that unescape to the same text
  are the same name.
- Every number is a JSON integer, written with no fraction and no exponent.
  `1.0`, `6.2e1`, and `1E0` are invalid even where JSON Schema calls them integers.
  `-0` is invalid too, because `serde_json` rejects it. An integer is `0`, or an
  optional minus followed by a nonzero digit and more digits.
- JSON nests at most 16 levels deep, and a reader reports anything deeper as
  `file.json`. M2-01 caps cJSON at 16 levels, and a valid layout nests 7 deep.
  The check runs before any parsing, so a file nested thousands of levels deep
  is rejected, never a crash.
- No string contains U+0000 to U+001F or U+007F. That covers every field, including
  command arguments and working directories. The escape `"\u0000"` is invalid.

## Layout

All fields listed here are required. Every object rejects unknown properties.
Numbers that describe cells, spans, or HID usages are integers. Strings use
Unicode code points for length limits, as JSON Schema specifies.

| Field | Value |
| --- | --- |
| `schema` | Integer `1`. A different version is invalid. |
| `name` | Display name of 1 to 24 characters with at least one that is not whitespace. |
| `orientation` | `landscape` or `portrait`. Each layout describes one orientation. |
| `pages` | Array of 1 to 8 pages, in swipe order. |
| `fullScreen` | Optional boolean. `true` hides the page title and page dots and lets the grid fill the whole screen. Absent means `false`; writers omit it unless it is `true`. |

Whitespace means the Unicode `White_Space` property, so a no-break space and an
ideographic space count. A zero-width space does not.

Page and tile IDs share one namespace across the entire layout. Every ID matches
`^[a-z0-9_]{1,15}$`. An action's page target refers to a page ID in that layout.
The first page is the initial page.

### Upload and string limits

The upload limit is 32768 bytes of compact UTF-8 JSON. The size of a layout is
the UTF-8 length of its canonical compact form, which is `JSON.stringify(layout)`
with no spacing, or the same bytes from `serde_json::to_vec`. Compact JSON has no
whitespace outside strings. The parsed layout fixes the form, so an escape like
`\u0046` counts as the one byte it stands for. A repeated member never reaches
the size check, because the validator rejects it first as `json.duplicate-member`.
Layouts hold no control characters or lone surrogates, so the only escapes the
canonical form writes are `\"` and `\\`. Exactly 32768 canonical bytes is valid.
The validator rejects 32769 or more and reports both the actual size and the limit.

Every sender sends the canonical compact form. The deck cannot parse first, so it
rejects any received layout file over 32768 raw bytes. A pretty-printed or escaped
file can pass this validator and still be rejected on the deck. Compact it with
`JSON.stringify(JSON.parse(text))` before sending it.

These limits are the shared contract for the Rust validator in M2-10, the editor's
**Push** in M2-11, and the firmware parser in M2-04. Each must reject layouts above
the compact byte limit and enforce 1 to 8 pages. The layout `name`, every page
title, and every grid tile label must hold at least one character that is not
whitespace, which includes being nonempty. Names and titles allow at most 24
Unicode code points, and labels allow at most 16. `maxLength` counts code points,
not UTF-8 bytes or UTF-16 code units. The firmware must store whatever UTF-8 the
schema allows.

## Pages

| Field | Grid page | Dashboard page |
| --- | --- | --- |
| `id` | Unique ID. | Unique ID. |
| `title` | 1 to 24 characters, one not whitespace. | 1 to 24 characters, one not whitespace. |
| `kind` | `grid` | `dashboard` |
| `rows` | 1 to 8. | Absent. |
| `cols` | 1 to 8. | Absent. |
| `tiles` | Array of 0 to 64 tiles. | Absent, including an empty array. |
| `source` | Absent. | Nonempty source identifier, such as `cliproxyapi`. |

A grid may have empty cells. A dashboard has no grid or tiles. The app supplies
its rows from the named source. Source identifiers remain open to plugins. The
deck treats a dashboard page as a grid of 1 row and 1 column, so its page model
stays valid and the pages after it still show.

## Tiles

| Field | Value |
| --- | --- |
| `id` | Unique ID. |
| `label` | 1 to 16 characters, one not whitespace. |
| `row`, `col` | Zero-based cell coordinates, each 0 to 7. |
| `rowSpan`, `colSpan` | Number of cells occupied, each 1 to 8. |
| `icon` | `null`, one of the six `lv:` names below, or `file:` followed by exactly 16 lowercase hexadecimal digits. |
| `action` | Exactly one action object from the table below. |

A tile fits only when `row + rowSpan <= rows` and `col + colSpan <= cols`.
No two tiles may occupy the same cell. Adjacent tile edges may touch.
File icon hashes use lowercase hexadecimal digits only, because deck storage
names are case-sensitive and the app derives each hash from file contents. A file
icon names a bitmap in deck storage. The validator checks the name, not file
presence.

Schema 2 tiles may add `asset`, `render`, `streams` and `input`, as
[USB link v2](protocol/link-v2.md#layout-references-and-live-values) defines.
`render: "gif"` needs a `sha256:` Asset and `icon: null`. The Live tile renderers
and their streams are in [live/renderers.json](live/renderers.json), which every
validator reads or matches:

| `render` | Required streams | Optional streams |
| --- | --- | --- |
| `graph` | `value` as `number` or `series` | none |
| `bar` | `value` as `number`, from 0 to 1 | `muted` as `number`; nonzero draws the muted look |
| `now-playing` | `title` and `artist` as `text`, `cover` as `image`, `state` as `number` | none |

A Tile declares every required stream of its renderer, may declare the optional
ones, and declares no others. A Tile without `render` declares no streams. `bar`
and `now-playing` take no Asset in this release: their `asset` is absent or
`null`. `input: true` makes a `bar` Tile draggable, and needs `render: "bar"` and
a `widget` action. Any other value, or `input` on another Tile, is invalid.
Absent `input` means the Tile only shows its value. Schema 1 rejects all four
fields.

The six built-in icon names reproduce milestone 1's media icons.

| Name | LVGL glyph |
| --- | --- |
| `lv:prev` | `LV_SYMBOL_PREV` |
| `lv:play_pause` | `LV_SYMBOL_PLAY` followed by `LV_SYMBOL_PAUSE` |
| `lv:next` | `LV_SYMBOL_NEXT` |
| `lv:mute` | `LV_SYMBOL_MUTE` |
| `lv:volume_mid` | `LV_SYMBOL_VOLUME_MID` |
| `lv:volume_max` | `LV_SYMBOL_VOLUME_MAX` |

## Actions

Each action has a `type` and exactly the fields for that type. Fields from another
type are invalid. The schema uses `oneOf` with a constant `type` in each branch.

| `type` | Other required fields | Runs on | Meaning |
| --- | --- | --- | --- |
| `key` | `usage`, `mods` | Deck | Tap a keyboard HID usage from 1 to 255 with the listed modifiers. `mods` is a duplicate-free subset of `ctrl`, `shift`, `alt`, and `win`. An empty array is allowed. |
| `mod` | `mod` | Deck | Apply the modifier `ctrl`, `shift`, `alt`, or `win`. A tap arms the next key, a double tap locks it, and a tap on a locked modifier clears it. |
| `consumer` | `usage` | Deck | Tap a consumer HID usage from 1 to 65535. |
| `page` | `page` | Deck | Jump to the existing page with that ID, including a dashboard. |
| `none` | None. | Deck | Do nothing. |
| `launch` | `path`, `args`, `cwd` | App | Start the nonempty executable path with an array of string arguments. `cwd` is a working-directory string. An empty string inherits the app's working directory. |
| `open` | `target` | App | Open a nonempty file path or URL with the operating system's default handler. |
| `command` | `program`, `args`, `cwd`, `shell` | App | Run the nonempty program with an array of string arguments and a working-directory string. An empty `cwd` inherits the app's working directory. Boolean `shell` chooses whether the app invokes a shell. The app asks before a new or changed command runs. |
| `widget` | `source`, `field`, `account` | App | Display a reading from a source for an account. `source` and `field` are nonempty strings. `account` is a required string that may be empty. |

The deck owns the key engine and its console key log. The milestone 2 app mirrors
key reports to the PC. Deck-side describes action ownership, not a USB HID sink.
The schema does not check installed programs, URLs, accounts, or source availability.

### Widget accounts

A widget's `account` picks one reading from its source. An empty account means the
source's single or aggregate value. Source plugins interpret `field`.

A `cliproxyapi` account uses a `provider:email` key, such as
`codex:alice@example.com`. Usage fields are `five_hour_pct`, `seven_day_pct`,
`five_hour_reset`, `seven_day_reset`, and `plan`. An empty account selects the
highest available percentage for each usage window across accounts. A missing
window is unavailable, not zero; plans and reset times belong to individual accounts.

The `status`, `success`, `failed`, and `recent_10m` fields remain available. With
an empty account, `status` is the worst entry's status and counts are summed.

The schema does not check accounts. The app resolves them.

## Examples

[builtin.json](examples/builtin.json) is the firmware's built-in fallback.
Its `fkeys` page is 4 by 4, with F1 to F12 and Shift, Ctrl, Alt, and Win.
Its `media` page is 2 by 3, with the six original consumer actions and icons.
The 22 tile IDs, labels, cell positions, and usages match the milestone 1 tables.

[dev.json](examples/dev.json) has three grid pages and a CLIProxyAPI dashboard.
It covers all nine action types, a horizontal span, a vertical span, a key chord,
a dashboard jump, and four CLIProxyAPI widgets.
Its command tile describes `cmd /c echo TouchDeck` on Windows. It names no `file:`
icon, because no app cache holds a bitmap whose hash is not a real file's.

Later milestone lanes depend on these fixture IDs and roles. `f5` sends F5 with
usage 62, and `shift` applies Shift on the same page. `chord` is a key with `ctrl`
and `shift`. `launch`, `command`, and `open` each use their matching app action.
The `five_hour` and `seven_day` widgets use CLIProxyAPI fields `five_hour_pct`
and `seven_day_pct` with an empty account for aggregate usage. `claude_5h`
uses `five_hour_pct` for the fixture account `claude:alice@example.com`. The
`proxy` widget uses `cliproxyapi` field `status` with an empty account. The
`accounts` dashboard uses source `cliproxyapi`, and `accounts_jump` opens it.

[media.json](examples/media.json) is a schema 2 Page with a draggable Volume
`bar` that also shows `muted`, a read-only `bar` with `value` alone, and a
`now-playing` Tile.

The [invalid examples](examples/invalid/) each break one rule and have one file per
rule key. The first line of each file is plain text, not JSON:

```text
expected: label.blank
```

The layout text starts on the second line. A reader of the invalid examples drops
everything through the first line feed, then validates the rest. Each file must
fail with exactly one error, and its key must be the key on the first line.
Only files in the bundled `examples/invalid/` directory receive this treatment.
An invalid file supplied from elsewhere keeps its first line, so it fails with
`file.json`. An unreadable file always counts as a problem.

## Rule keys

Every failure carries one of these keys. The firmware parser and the Rust validator
emit the same keys, so all three run the same invalid examples. Keys name the rule,
and the message names the place. A layout that breaks several rules reports several
keys. Rules whose key starts with `file.` or `json.` apply to the file's text, and
`schema/check-layouts.test.mjs` checks this table against the validator.

| Key | Meaning |
| --- | --- |
| `file.utf8` | The file is not well-formed UTF-8. |
| `file.json` | The file is not valid JSON, which includes a leading byte order mark and nesting deeper than 16 levels. |
| `json.duplicate-member` | An object repeats a member name. |
| `json.integer-form` | A number has a fraction or an exponent, or is `-0`. |
| `string.malformed-unicode` | A string or member name contains a lone surrogate. |
| `string.control-character` | A string contains U+0000 to U+001F or U+007F. |
| `shape.type` | A value has the wrong JSON type, such as a string where an integer belongs. |
| `shape.missing-field` | A required field is absent. |
| `shape.unknown-field` | An object has a field its type does not define. |
| `layout.schema-version` | `schema` is not the integer 1. |
| `layout.orientation` | `orientation` is not `landscape` or `portrait`. |
| `layout.page-count` | `pages` has fewer than 1 or more than 8 pages. |
| `layout.too-large` | The canonical compact form exceeds 32768 bytes. |
| `name.blank` | `name` has no character that is not whitespace, which includes being empty. |
| `name.too-long` | `name` has more than 24 code points. |
| `id.format` | A page ID, tile ID, or page target does not match `^[a-z0-9_]{1,15}$`. |
| `id.duplicate` | Two pages or tiles share an ID. |
| `page.kind` | A page `kind` is missing, or is not `grid` or `dashboard`. |
| `title.blank` | A page `title` has no character that is not whitespace, which includes being empty. |
| `title.too-long` | A page `title` has more than 24 code points. |
| `grid.size` | `rows` or `cols` is outside 1 to 8. |
| `page.tile-count` | A grid page has more than 64 tiles. |
| `label.blank` | A tile `label` has no character that is not whitespace, which includes being empty. |
| `label.too-long` | A tile `label` has more than 16 code points. |
| `tile.position` | `row` or `col` is outside 0 to 7. |
| `tile.span` | `rowSpan` or `colSpan` is outside 1 to 8. |
| `tile.bounds` | A tile extends past its page's rows or columns. |
| `tile.overlap` | Two tiles on one page occupy the same cell. |
| `tile.streams` | A schema 2 tile's `streams` do not match its `render`: a stream the renderer doesn't bind, a type it doesn't accept, or a required stream that is missing. Tiles without a Live tile renderer declare none. |
| `tile.input` | A schema 2 tile's `input` is not `true`, or the tile is not a `bar` with a `widget` action. |
| `icon.format` | `icon` is not `null`, a known `lv:` name, or `file:` and 16 lowercase hexadecimal digits. In schema 2 also an invalid `asset` or `render`, a `gif` without an Asset, or a `bar` or `now-playing` with one. |
| `action.sink` | A key or consumer action has a sink other than `link` or `hid`, or target validation finds HID unsupported. |
| `action.type` | An action `type` is missing, or is not one of the nine types. |
| `action.page-missing` | A `page` action names a page that does not exist. |
| `key.usage-range` | A `key` action `usage` is outside 1 to 255. |
| `key.mods-duplicate` | A `key` action lists a modifier twice. |
| `modifier.unknown` | A modifier is not `ctrl`, `shift`, `alt`, or `win`. |
| `consumer.usage-range` | A `consumer` action `usage` is outside 1 to 65535. |
| `text.empty` | A required text field is empty. Those are `source`, `path`, `target`, `program`, and `field`. |

Structural keys come from the schema. Each constraint in `layout.schema.json`
carries its key in an `x-rules` annotation, which JSON Schema validators ignore.
A schema validator that lacks the annotation still rejects the same layouts.

## Errors

The message after a key names the place with a JSON Pointer and says what is wrong.
Structural messages contain the schema keyword in parentheses. Missing or extra
fields also include the quoted field name. Known page kinds and
action types report errors from the selected branch only. A field the selected
branch does not define is reported as `shape.unknown-field` and no other schema
error, so a dashboard page with a `tiles` array never reports schema errors about
those tiles. The file-wide text rules still apply inside such a field:
`json.integer-form`, `json.duplicate-member`, and `string.malformed-unicode`.
A missing or unknown kind or type reports only `page.kind` or `action.type`, so
a missing `kind` never reports `shape.missing-field`. Ajv is pinned by the lockfile.

The text checks run first, then the structural checks. The semantic checks run
only after structural validation succeeds. They report the compact byte limit
first, then page and tile errors in order with these message forms:

```text
layout compact UTF-8 JSON is <actual> bytes; maximum is 32768 bytes
duplicate id "<id>" at <location>; first used at <location>
page "<page>" tile "<tile>" references missing page "<target>"
page "<page>" tile "<tile>" exceeds grid bounds <rows>x<cols>
page "<page>" tiles "<first>" and "<second>" overlap
```

A location is `page <index> "<id>"` or `page <index> "<id>" tile <index> "<id>"`,
with zero-based indexes, so two uses of one ID always differ.
The duplicate check includes page IDs and tile IDs. Overlap reports both tile
IDs once per pair, even when the pair shares several cells.

## USB link v2

[USB link v2](protocol/link-v2.md) defines negotiated Device capabilities, names,
Assets, live values, and photo idle mode. Its [golden frames](protocol/golden-frames-v2.json)
share the v1 codec format. Runtime support and the Layout schema 2 validator
are separate implementation work; the schema 1 rules above remain unchanged.

## Bluetooth Tile destination

`key` and `consumer` actions accept an optional `sink`, either `link` (default,
the Companion) or `hid` (the Bluetooth keyboard host). Other actions reject it.
This is an additive optional field in schemas 1 and 2; their version numbers and
all existing omitted-sink Layouts stay unchanged. Older readers reject the new
field under their unknown-field rule. Target validation requires `hello.sinks`
to contain `hid` for Bluetooth actions; normal Firmware rejects them. A Bluetooth
consumer usage must fit the existing HID report map (1–1023). Invalid sink values
report `action.sink`. See `examples/bluetooth.json` for keys and media keys.
