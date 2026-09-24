# iPad live statistics, base recovery and scrolling — 2026-09-23

Version: 0.22.0-rc.4.

The local server still running on UDP 4010 omitted the scoreboard field entirely. A Hello-only probe reproduced that difference from the newer server on 4020. The current ledger path already reports accepted damage correctly; new regressions exercise kills, deaths and assists through a real UDP snapshot and the client ingest/HUD resource path. Restart the local server using the updated executable to deliver scores.

Living, joined heroes now recover 12% of maximum HP per second inside their own base shop zone during a running match. Both teams and bots use the same authoritative rule; dead heroes, the enemy base and finished matches do not heal.

Settings now have a constrained scrolling body and a fixed Back footer. Career cards no longer shrink to fit the screen. Mobile scrolling captures one finger, handles batched touch events, converts retina/UI-scale coordinates correctly and cancels button activation during drags. Career buttons activate on short release. Desktop wheel scrolling remains covered.

Validation: 221 server tests passed (3 intentionally ignored); 484 client tests passed. Native Bevy fixture renders cover tablet results and phone settings, including the bottom of both lists and reachable exit controls. Gesture tests cover DPI 1/2 and UI scale 1/0.75. Physical iPad swipes are not verified by these desktop renders; rebuild and run on the iPad for the final hardware check.

User Makefile and Xcode signing/scheme edits must remain untouched. This change does not upload to TestFlight or modify production infrastructure.

Local delivery: both idle practice servers on UDP 4010 and 4020 were restarted with the rc.4 build. Hello-only probes now receive the scoreboard field from both; it is null in the empty lobby and populated once a match starts. The accepted-damage regression verifies nonzero match values.

To test on iPad: open `mobile/ios/Omoba.xcodeproj` from the canonical repository, choose scheme Omoba and the connected iPad, then Run to rebuild/install. Keep the Mac and iPad on the same network; in the game choose Server and connect to `192.168.1.71:4010` (4020 also has the updated local build). Local practice matches provide per-match results, not persistent career/rating credit.
