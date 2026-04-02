# Progress Report: English Agent Guidance

- Date: 2026-04-01
- Scope: standardize agent prompts and coordination docs on English

## Changes

- Converted the copy-paste prompt templates in `docs/agents/README.md` from mixed Russian/English to full English.
- Added a persistent English-language rule for Claude in `.claude/rules/project-language.md` and imported it from `CLAUDE.md`.
- Added the same persistent language rule for Cursor in `.cursor/rules/005-project-language-english.mdc`.
- Mirrored the English-default guidance in `AGENTS.md`.

## Checks

- Verified the shared task-start prompts are now fully English.
- Verified both Cursor and Claude have explicit repo-local guidance to default to English.

## Remaining Risks

- Older pre-existing rule files still contain legacy non-English guidance and may need a later cleanup pass if full repository consistency is required.
- Future prompt templates should be updated in one place first and then copied into the rule files to avoid drift.
