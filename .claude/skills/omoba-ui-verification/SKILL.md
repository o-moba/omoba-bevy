---
name: omoba-ui-verification
description: Plan and run economical OMOBA UI smoke checks and screenshot evidence in omoba-bevy or omoba-ui. Use when changing UI, checking layouts or localization, or preparing visual verification. Defaults to one language and one device; a full release matrix requires an explicit user request.
---

# OMOBA UI verification

## Routine checks — default

- Use **one language and one device/viewport** for the task. Check only changed screens and states needed to reproduce and verify the requested behavior.
- Select the device implicated by the bug or explicitly requested by the user. Otherwise reuse the cheapest available representative profile; if none is selected, use the iPhone 16 landscape viewport at 852 × 393 logical points. A viewport check does not prove behavior on physical iOS hardware.
- Use English unless the task concerns another language; for a Russian localization bug, check Russian instead. Do not add other languages just for completeness.
- Reuse the existing build and screenshot tooling. Capture each required state once per relevant revision; after a fix, repeat the failed check and directly affected states. Reuse captures in reports instead of rendering the same screens again.
- Do not fan out agents, builds, simulators, or capture batches across devices/locales. Avoid full-game screenshot sweeps for a focused UI change. Documentation-only changes need no game launch.
- Keep the smallest relevant functional/unit checks and required build checks. This budget limits visual sweeps; it does not excuse leaving the requested behavior unverified. Run the targeted suite once and repeat only after a relevant change, failure, or unresolved concern.

## Full release verification — explicit opt-in

Enable a full UI matrix only when the user explicitly requests it for the current run, for example: **“Полная UI-проверка OMOBA перед релизом”** or “Run the full OMOBA UI matrix across devices and languages.” This is a natural-language instruction, not a shell command.

“Make it release-ready,” “check everything,” a version bump, or an upcoming large release alone does not authorize the device/language matrix. Use the routine scope unless the user clearly asks for the full matrix. A user request for specific additional devices or languages authorizes only that stated scope.

For an authorized full run, state the planned screens, languages, and device profiles, reuse existing builds, and execute that matrix once. Rerun only failed or newly affected cells. Record viewport/simulator/physical-device evidence accurately. Return to routine mode for subsequent tasks. This skill does not authorize CI, deployment, or production changes.

## Proof and reporting

- Apply the same budget when freezing a task spec and when a fresh verifier reruns checks. Fresh verification does not imply a larger matrix.
- If an older task spec assumes a broad matrix without the owner's explicit request, document a scope amendment reflecting this standing instruction before proceeding. Never mark untested criteria PASS.
- Record the selected language, device/profile, viewport, build identity, checked screens, and actual results. State that other devices/languages were not checked; do not describe routine evidence as full release certification.
- If the selected environment cannot prove a required behavior, report that specific limitation and use the smallest suitable replacement check rather than automatically adding every platform.
