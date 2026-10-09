# First-match guidance: integration review, 2026-10-09

PR #74 adds objective-first help, five loading advice cards, a persisted beginner-tips switch, smaller creature bars, quieter combat effects and a closer initial mobile camera. Manual Help and saved camera choices remain available.

Merged the current main (including #75 and #76) into the PR branch. Resolved the changelog and three pause dictionaries by retaining both features. The new beginner-tip keys coexist with the Debug tools label. No workflow or dependency change is introduced relative to main.

Review found that loading advice tracked only a finger beginning inside its text. A finger already held outside the text, or a third finger after cancellation, could therefore advance a card during multitouch. Track every contact until release, cancel the candidate on multitouch, and reset the gesture on entry to a new loading screen. A regression covers two and three fingers, inside/outside starts, recovery and re-entry.

The final layout review also found that the fixed 904 px desktop footer put its new previous-tip button outside an 800 px window. The footer now fits between the shell insets, remains capped at 904 px and centres using its resolved width. A headless Bevy layout regression checks both buttons against the viewport insets. The phone layout is unchanged by this follow-up.

Validation on the combined tree:

- `make check` passed: format, workspace Clippy, client Clippy without QA, Rust workspace tests (client: 1,285 passed, one ignored), 208 script tests and 45 iOS tooling tests. Database-only tests remain CI's responsibility.
- The multitouch regression failed before the fix and passed afterward.
- A native client build and captures at `15fc655` passed; the desktop-only follow-up is covered by the headless layout test and leaves phone rendering unchanged. English, 844×390 desktop phone emulation: loading advice in the offline/retry state, help at the top and bottom, controller dismissal and return to Settings, and the synthetic team countdown. All three capture runs exited successfully; all five Help QA assertions passed. Advice and controls fit the viewport; the help content scrolls to its footer.
- Evidence is local at `.agent/tasks/PR74-MERGE-REVIEW-20261009/`. CI on the pushed head remains the final merge gate.

Routine visual scope is one language and one phone viewport. This does not certify physical iOS/Android behavior, frame time or the subjective audio balance. No deployment or mobile upload is part of this merge.
