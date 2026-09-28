# iPad UI feedback — 0.28.2

## Scope

Owner-reported physical iPad issues: phone-sized top-left Home, missing parallax/blinks, competing hero platforms, small collection preview, long settings scroll, unfinished practice entry and Match Score. Full offline match parity is explicitly deferred to `docs/plans/offline-practice-parity.md`.

## Implementation

- Use viewport height to distinguish compact and spacious layouts rather than equating touch with phone. Home fits/centers a reference canvas; Collection and picker own their responsive layout.
- Keep Home background on a separate screen-lifetime root. Session/locale changes rebuild foreground content without restarting the living fade. Touch parallax, capped frame delta and interpolated low-detail visibility avoid abrupt visual transitions.
- Align Home preview feet with the covered painting's stage, hide its extra pedestal, preserve drag yaw, and run actual expressive clips as one-shots. VRM is not an animation library: Agnes ships idle/walk/attack/cast/death, so a dedicated wave remains an asset task.
- Add Collection panels/filter state and prominent artwork; split Settings into four scoped groups with stable rail/footer; retain phone scrolling where necessary.
- Use the existing kit for spacious practice selection and live scoreboard identity/stat rows. Preserve production session, modal/input and authoritative scoreboard paths.

## Verification and limits

Native renderer captures use desktop-hosted touch emulation, including a 1180×820 tablet viewport. Tests cover centered Home geometry, background identity across refresh, settings content bounds at tablet HiDPI, filters' existing selection paths and preview gesture/clip rules. Task proof is in `.agent/tasks/UI-IPAD-FEEDBACK/`.

These are not physical-device captures. Metal performance, actual finger gestures and absence of the owner's intermittent blink must be rechecked on the iPad. No iOS signing/build-phase, CI/deployment or dependency changes are included.

Current handoff status: native desktop (24 frames) and tablet shell/offline (22 frames) pass; 768 client tests passed before final cosmetic adjustments. Phone shell reaches 12 frames and exposes a stale expected Home scale after closing Help; that QA assertion is fixed in source. Disk reached 18 GiB free, so the rebuilt phone rerun and fresh full workspace test gate are paused per disk policy. No cache was deleted. This work is not yet committed/merged or declared release-ready.

## Owner-requested integration

On 2026-09-28 the owner explicitly requested committing/merging the pending changes into main without another local build and will rebuild in Xcode. Integrated alongside the visible build label; version advanced to 0.28.2 to distinguish this UI build from 0.28.1. Only lightweight merge checks run during integration. The unrun final phone/native/regression/device checks above remain unrun; merging is not a claim of full verification. No cache deletion, signing, deployment or device installation performed.
