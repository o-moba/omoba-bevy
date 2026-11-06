# Ranked presentation evidence — 2026-11-06

Unmodified real native/Chrome PNGs, inspected PASS. Each file's SHA256, source
revision and method are in [manifest.json](manifest.json). Profile/result/API
values and returning-user settings are synthetic. These images do not prove
physical phones, live accounts, legal consent, keep-awake or haptics. This is a
focused LR-53 RU/EN criterion check, not a full release UI matrix.

Native RU baseline: build51ba584e… on56b86dec with opt-in QA WIP (diff
748c0b5ea21442702f16d78101506e84ff6d095e4a40a6443bd44ab1cfe6fda3).
The underlying seasonal presentation is unchanged through c8260b31. The final
RU expired/demotion and English profile images use exact c8260b31 binary
366d088f72db1455fcab8d6a3071784f3be582d7ae8c4eb36f7ec966394a7d01;
client sources/runtime assets were checked unchanged while disjoint directory
work continued. Current c826 native CI executes1578 tests PASS (one ignored).

Portal ccee8fa: production Next + Chrome against a synthetic loopback API;
250 tests/3 expected skips,typecheck/build, seven RU change-notice checks and
six EN season/privacy/legacy checks PASS. Portal locale evidence does not
certify deployment, provider login or every market configuration.

Black, missing-notice and incoherent fixture attempts are retained privately in
the task directory as rejected evidence. The first post-unlock Home attempt was
black despite an unlocked, awake display; its renderer cause is unconfirmed.
The successful captures required no new native build or OS preference changes.

| Image | Surface / locale | Checks |
| --- | --- | --- |
| [native-ru-baseline-1.png](native-ru-baseline-1.png) | native / ru | calibrated Gold Home |
| [native-ru-baseline-2.png](native-ru-baseline-2.png) | native / ru | Home0/20 despite100 lifetime |
| [native-ru-baseline-3.png](native-ru-baseline-3.png) | native / ru | own profile, season2, retained season1 Platinum2300 |
| [native-ru-baseline-4.png](native-ru-baseline-4.png) | native / ru | friend placement badge |
| [native-ru-baseline-5.png](native-ru-baseline-5.png) | native / ru | visited profile season/badge |
| [native-ru-baseline-6.png](native-ru-baseline-6.png) | native / ru | scoped Gold promotion notice |
| [native-ru-final-1.png](native-ru-final-1.png) | native / ru | Ranked interseason label fits and is visibly disabled; Casual remains visibly available while online; current calibrated rating retained |
| [native-ru-final-2.png](native-ru-final-2.png) | native / ru | personal defeat despite winning team; 1478(-32),zero XP,saved progress; visible settled Silver demotion warning; all result controls readable |
| [native-en-profile-1.png](native-en-profile-1.png) | native / en | current Gold badge; profile windows and historical Platinum2300 badge where present |
| [native-en-profile-2.png](native-en-profile-2.png) | native / en | current Gold badge; profile windows and historical Platinum2300 badge where present |
| [native-en-profile-3.png](native-en-profile-3.png) | native / en | current Gold badge; profile windows and historical Platinum2300 badge where present |
| [portal-ru-1.png](portal-ru-1.png) | portal / ru | calibration.png |
| [portal-ru-2.png](portal-ru-2.png) | portal / ru | personal-penalty-demotion.png |
| [portal-en-1.png](portal-en-1.png) | portal / en | reset-and-old-badge.png |
| [portal-en-2.png](portal-en-2.png) | portal / en | calibrated-ladder.png |
