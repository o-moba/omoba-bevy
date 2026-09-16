# Mobile playtest polish — 0.20.0-rc.4

Implemented five reported playtest improvements in the primary checkout:

- Stable bot target/range decisions, immediate route continuation and bounded
  swept hero separation; 100 ms remote interpolation and current-pose head labels.
- One continuous opaque river surface below lane crossings, with joined banks.
- Phone chat composer/send/close in the top safe area, draft-preserving keyboard
  toggle and UIKit Return submission; desktop chat layout preserved.
- Bounded mobile target handle with an independent forward selection ray. Release
  confirms only the same final eligible preview; actual attack range is unchanged.
- Sixteen original skill icons for phone/desktop. Stationary 450 ms skill holds show
  shared ability metadata; inspection release never casts. Deliberate drags still aim.

Validation: 364 client tests; 14 server movement/practice regressions; 36 iOS Python
tests; native phone-layout chat/long-press capture and five real map views; source
formatting and whitespace checks. UI screenshots exposed and resolved tooltip
stacking and square image corners. Test-only native focus/readiness diagnostics
were improved without changing production focus policy.

The physical arm64 iPhone build and Xcode archive are retained at
`builds/iphone-polish-2026-09-16/` (TestFlight build 3). The actual executable and
retained dSYM UUIDs match; source/art hashes and development signature verified.
The iOS tools preserve symbols, resolve Cargo root dSYM aliases and reject missing,
empty or mismatched symbols before archiving. The practice helper also resolves
assets inside the packaged iPhone app.

Local updated practice server: 192.168.1.71:4000. Final task evidence is under
`.agent/tasks/MOBILE-POLISH-2026-09-16/`. A background supervisor renews its own
separate runtime lease and stops only its owned child. Existing packages and
unrelated cinematic/TestFlight work were preserved. No new production dependency,
commit, push, Apple upload or physical iPhone acceptance is claimed.
