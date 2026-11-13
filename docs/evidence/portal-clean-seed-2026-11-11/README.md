# Clean seed and leaderboard acceptance — 2026-11-11

## Result

PASS for the second 2026-11-11 review finding. A fresh disposable PostgreSQL
18-alpine service listened on `127.0.0.1:55581`. The current executable
`seed-local-portal` created its three synthetic profiles and two genuine
settlements. The real RU Account API projected them, the production RU Next
standalone server served the portal, and the strengthened Playwright test ran
without any manual SQL fixture or post-seed SQL.

- game head: `1b61b9bdadb1705f57eef00a4c54ad14dfc9b836`;
- tested PR merge: `c1c47063e476206dc8cf4fe28375ca821e02fb1c`;
- tested merge tree: `e042757216e42b7062680aa2cad642e9ccd51fef`, byte-identical
  to the game head tree;
- portal: `84525384bc6e8d37991e2ad3fe4f2dd4a5d30d41`;
- accepted run: [43012620207](https://github.com/o-moba/omoba-bevy/actions/runs/43012620207).

The seed source SHA-256 is
`f489d8710348b76d50b70d38ca19896590d1f3735f9de6dbacacedb76d043532`;
the browser test source SHA-256 is
`bfa32e4f76a3c63a0399a9e9719119417a0744b63a4b6d3b8194e163d82c35de`.
The compiled Account API and seed executable hashes are retained separately in
`source-and-binary-sha256.txt`.

## Freshly observed checks

| Gate | Result |
| --- | --- |
| Portal unit tests | 257 PASS, 0 FAIL, 3 expected skips; 260 total |
| TypeScript | PASS, `tsc --noEmit` |
| Production RU portal build | PASS |
| Current seed executable | PASS, `synthetic=true`, `saved=true`, two result IDs |
| Leaderboard API and DOM | PASS, two entries with distinct ratings 1031 and 985 |
| Own row, opt-out and hidden explanation | PASS |
| Seasonal placement | PASS, lifetime 21, current season 1, visible `Калибровка 1/20` |
| Mobile browser layout | PASS, 390 px, no horizontal overflow |
| Browser images | `mobile.png` and `hidden.png` visually inspected and accepted |

The two accepted images show the ordered synthetic entries and the hidden-state
explanation. They are rendering evidence only, not physical-device input or
native-key-storage proof.

## Why earlier runs are not accepted

Run 43010103535 had an invalid job-level `runner` context and started no jobs.
Run 43010406980 did not build the Account API binary and stopped before touching
the database or seed. Run 43011390908 produced a real browser PASS, but its job-
wide RU environment contaminated two unit tests and a `tee` pipeline without
`pipefail` hid their failure. That run is rejected as a complete gate.

The accepted run builds both binaries, isolates unit tests from service `OMOBA_*`
variables, and uses Bash `pipefail` for every step. Its unit log was independently
re-read after completion rather than inferred from the green workflow status.
The temporary review workflow is removed in the final documentation commit; this
pack retains the accepted artifacts and provenance.

## Reproduce the archive checks

```sh
python3 docs/evidence/portal-clean-seed-2026-11-11/verify.py
python3 -S docs/evidence/portal-clean-seed-2026-11-11/verify.py
```

The verifier uses only the Python standard library. It recomputes retained file
hashes, the game seed source hash and, when the sibling `mm-web` checkout exists,
the portal test source hash. It checks unit counts, seed/result identity,
ordering, placement text, service markers and PNG dimensions. Automated checks
do not replace the recorded visual inspection.

## Limits

This is a synthetic loopback integration result. It proves neither production
accounts, organic matchmaking, WAN behavior, isolated RU-host capacity, live
signing-key provisioning, deployment, recovery nor physical devices. It does not
change the original 62 done / 5 partial / 1 open finding count or RU **NO SHIP**.
