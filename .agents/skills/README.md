# Agent skills

Skills for coding agents working on this repo. This folder is the canonical copy. Claude Code reads only `.claude/skills/`, so that folder holds a byte-for-byte copy of each skill here. After you change a skill here, run:

```sh
python .github/scripts/skills_mirror.py sync
```

CI runs `python .github/scripts/skills_mirror.py check`, which fails when the copy differs.

## Sets

Each third-party skill keeps its upstream license in its own folder (`LICENSE` or `LICENSE.txt`). All of them allow redistribution with the notice kept.

| Set | Source | License | Version | Skills |
| --- | --- | --- | --- | --- |
| Matt Pocock's skills | [mattpocock/skills](https://github.com/mattpocock/skills) | MIT | Not recorded. `skills-lock.json` lists each skill's upstream path and content hash. | ask-matt, chief-of-staff, claude-handoff, code-review, codebase-design, diagnosing-bugs, domain-modeling, git-guardrails-claude-code, grill-me, grill-with-docs, grilling, handoff, implement, implement-spec, improve-codebase-architecture, loop-me, migrate-to-shoehorn, pr, prototype, research, retro, scaffold-exercises, setup-matt-pocock-skills, setup-pre-commit, setup-ts-deep-modules, tdd, teach, to-questionnaire, to-spec, to-tickets, triage, wait-what, wayfinder, wizard, writing-beats, writing-for-agents, writing-fragments, writing-shape |
| Emil Kowalski's skills | [emilkowalski/skills](https://github.com/emilkowalski/skills) | MIT | Not recorded. `skills-lock.json` lists each skill's upstream path and content hash. | animate, animate-expo, animation-performance, animation-vocabulary, apple-design, ask-sonner, break-ui, emil-design-eng, find-animation-opportunities, improve-animations, mobile-native, pick-ui-library, review-animations, write-swift |
| tstack | [t3dotgg/tstack](https://github.com/t3dotgg/tstack) | MIT | `11e37a7` | babysit-pr, council, file-pr, full-send, full-stop, leaving-pr-comments, modernize, sol-loop, takeover |
| pstack | [cursor/plugins](https://github.com/cursor/plugins/tree/main/pstack) | MIT | Not recorded. Changed here: `disable-model-invocation` is removed so agents can load the skills on their own, and the unslop description says when to use it. | typescript-best-practices, unslop |
| Anthropic skills | [anthropics/skills](https://github.com/anthropics/skills) | Apache-2.0 | Matches `2c5958e` | frontend-design |

`pr` credits content from HumanLayer's `show-me` skill in `pr/CREDITS.md`.

## Project skills

`verify-touchdeck` (firmware on the real board) and `verify-touchdeck-app` (the app against a mock or real Device) belong to this repo and fall under its [license](../../LICENSE). They read machine settings, such as the board's serial port, from environment variables; each `SKILL.md` lists them.

## Updating a set

1. Clone the upstream repo into an empty scratch folder.
2. Compare each skill with its folder here, ignoring line endings (`diff -r --strip-trailing-cr`). Read the changes before you take them: they are instructions agents will follow, and some skills ship scripts.
3. Copy the changed files here with LF line endings, keep the skill's license file, and update the Version column.
4. Run `python .github/scripts/skills_mirror.py sync`.
