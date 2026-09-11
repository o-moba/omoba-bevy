# Mission formatting and main publication — 2026-09-12

Normalized MISSION.md, LICENSING.md, CONTRIBUTING.md, TRADEMARKS.md and SOURCE.md to one source line per paragraph/list item with explicit spacing between items. Removed an accidental Cyrillic editing suffix from MISSION.md. Word-level comparison confirms the substantive text is otherwise unchanged, including the user's existing paragraph edits. Canonical license files remain byte-identical.

The actual GitHub GFM renderer verified all five documents: headings, six mission items, five source-delivery steps, the license table and links. Nine existing packaging regressions passed. A separate source review found no serious contradiction in commercial-fork permissions, AGPL/MPL scopes, creator rights or the separation between mission goals and license terms. This formatting change does not claim to resolve the separate SDK license authorization or public-binary distribution requirements.

The user explicitly requested main publication. GitHub main was an ancestor of the local cumulative main, which contained the previously verified gameplay, jungle and licensing commits. Publication is scoped to refs/heads/main; the existing remote branches and tags are outside this operation. Raw checks and the final remote revision result are in `.agent/tasks/MISSION-FORMAT-2026-09-12/` in the cumulative local checkout.
