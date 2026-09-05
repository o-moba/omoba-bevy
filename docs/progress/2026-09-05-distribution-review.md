# Distribution review — 2026-09-05

Raw evidence directory: `.agent/tasks/RELEASE-3D-2026-09-05/`. All raw filenames below refer to that directory. This report is the durable repository copy of the current review.

Status: **RG5 remains BLOCKED** pending resolution/exclusion of conflicting asset licenses and completion of the final candidate build review. This is a current read-only review plus one explicitly authorized dependency patch; it does not grant distribution approval. The coordinator retains the source assets and labels the produced local package **INTERNAL REVIEW ONLY** behind an explicit packaging guard. No public asset/package publishing is authorized by this review.

## Current alerts and the narrow repair

`gh auth status` succeeded for the existing wotori account. A fresh paginated GitHub API request to `repos/o-moba/omoba-bevy/dependabot/alerts?state=open&per_page=100` returned **16 open alerts: 6 high, 6 medium, 4 low**. This independently confirms today's count; it is not copied from the prior audit. All 16 affected package versions were in this worktree's lockfile before repair. Raw response, retrieval time and command status are retained in `rg5-dependabot-open.json` and `rg5-retrieval-status.json`.

The directly used `crossbeam-channel` 0.5.14 matched [GHSA-pg9f-39pc-qf8g](https://github.com/advisories/GHSA-pg9f-39pc-qf8g). Client networking uses channels, including the unbounded implementation associated with the drop race; Bevy also depends on this crate. The fixed version is **0.5.15**. After coordinator authorization, the workspace requirement and Cargo.lock were updated to 0.5.15. The command completed with exit 0 despite initial registry-index transfer timeouts. The exact lock diff changes only that package's version and checksum; no package was added and the SDK source/revision was unchanged. See `rg5-crossbeam-update.log` and `rg5-crossbeam-lock.diff`. Full rebuilt workspace/package verification belongs to the coordinator. GitHub alert #3 was not dismissed or marked closed; branch merge and GitHub rescanning have not occurred.

## Per-alert disposition

The following is source/feature triage for the current native macOS candidate, not a claim that a vulnerable package is generally safe. Cached native build fingerprints corroborate feature use but are not a replacement for the final package dependency graph. No other dependency was modified.

| Alert | Package / locked version | Fixed version | Current disposition |
| --- | --- | --- | --- |
| [#20 / phqj](https://github.com/advisories/GHSA-phqj-4mhp-q6mq) | openssl 0.10.72 | 0.10.80 | Not selected by macOS native-tls; AES key-wrap-padding API precondition. Reassess/update for Linux. |
| [#19 / xv59](https://github.com/advisories/GHSA-xv59-967r-8726) | openssl 0.10.72 | 0.10.79 | Same target exclusion; AES key-wrap-padding API precondition. |
| [#18 / xp3w](https://github.com/advisories/GHSA-xp3w-r5p5-63rr) | openssl 0.10.72 | 0.10.79 | Same target exclusion; OCSP responder decoding API precondition. |
| [#17 / 82j2](https://github.com/advisories/GHSA-82j2-j2ch-gfr8) | rustls-webpki 0.103.1 | 0.103.13 | CRL configuration is opt-in; no application/SDK CRL loading or RevocationOptions calls found. Keep a maintenance patch obligation. |
| [#16 / 38c5](https://github.com/advisories/GHSA-38c5-483c-4qqp) | grid 1.0.0 | 1.0.1 | Compiled through Taffy UI, but its only lockfile reverse dependency does not call affected expand_rows API. No direct app call found; bounded UI source lowers current exposure. Patch remains recommended maintenance. |
| [#15 / pqf5](https://github.com/advisories/GHSA-pqf5-4pqq-29f5) | openssl 0.10.72 | 0.10.78 | Not selected on macOS; short derive buffer on OpenSSL 1.1.x precondition. |
| [#14 / xmgf](https://github.com/advisories/GHSA-xmgf-hq76-4vx2) | openssl 0.10.72 | 0.10.78 | Not selected on macOS; oversized PEM callback return precondition. |
| [#13 / 8c75](https://github.com/advisories/GHSA-8c75-8mhr-p7r9) | openssl 0.10.72 | 0.10.78 | Not selected on macOS; AES key-unwrap API precondition. |
| [#12 / hppc](https://github.com/advisories/GHSA-hppc-g8h3-xhp3) | openssl 0.10.72 | 0.10.78 | Not selected on macOS; PSK/cookie callback precondition. |
| [#11 / ghm9](https://github.com/advisories/GHSA-ghm9-cr32-g9qj) | openssl 0.10.72 | 0.10.78 | Not selected on macOS; short digest-final buffer precondition. |
| [#10 / cq8v](https://github.com/advisories/GHSA-cq8v-f236-94qc) | rand 0.9.2 | 0.9.3 | Native fingerprints enable alloc/std, without log or thread_rng required by the advisory; no custom logger reentering rand was found. Reassess if features change. |
| [#9 / xgp8](https://github.com/advisories/GHSA-xgp8-3hg3-c2mh) | rustls-webpki 0.103.1 | 0.103.12 | TLS name-constraint bypass requires certificate issuance preconditions; rustls is compiled in workspace HTTP tooling. Not dismissed; upgrade/audit before exposing remote-fetch tooling broadly. |
| [#8 / 965h](https://github.com/advisories/GHSA-965h-392x-2mh5) | rustls-webpki 0.103.1 | 0.103.12 | Same HTTP/TLS maintenance disposition for URI name constraints; no exploit demonstrated. |
| [#6 / pwjx](https://github.com/advisories/GHSA-pwjx-qhcg-rvj4) | rustls-webpki 0.103.1 | 0.103.10 | CRL-checking preconditions; no application/SDK CRL configuration found. |
| [#4 / 434x](https://github.com/advisories/GHSA-434x-w66g-qw3r) | bytes 1.10.1 | 1.11.1 | Compiled in HTTP/Tokio dependencies. Overflow requires an extreme reserve request with wrapping arithmetic; no direct application BytesMut reserve caller. Transitive reachability was not conclusively excluded. Patch or deeper audit remains open before broad remote-fetch distribution. |
| [#3 / pg9f](https://github.com/advisories/GHSA-pg9f-39pc-qf8g) | crossbeam-channel 0.5.15 after repair | 0.5.15 | Resolved in local manifest/lock; full rebuilt validation and repository merge still required. |

Native-tls 0.2.14's own manifest selects OpenSSL only outside Windows/Apple, and Security Framework on Apple. `rg5-lock-reverse-dependencies.json`, `rg5-native-fingerprints.json`, and `rg5-reachability-checks.json` preserve the actual local evidence. The new UDP gameplay transport uses bounded byte vectors and serde, not HTTP/TLS. Optional SDK/arena-sync HTTP use is a distinct surface. Their inspected calls construct default reqwest clients; reqwest 0.12.15 chooses its default native TLS backend when default-tls is enabled and HTTP3 is absent, matching these fingerprints. Thus compiled rustls presence alone does not establish an active TLS verification path here. No claim is made about untested Linux builds or every possible transitive call.

No second issue was demonstrated as directly applicable as the repaired channel race in this bounded review. Candidate follow-ups are grid 1.0.1 and rand 0.9.3 (patch releases with unmet observed trigger preconditions), rustls-webpki 0.103.13 (patch covering four advisories, optional TLS surface), and bytes 1.11.1 (minor-version update with transitive HTTP reachability still requiring review). These were reported to the coordinator rather than expanded into the frozen implementation.

## Required 3D provenance

All three committed required manifests are tracked and cover **20 tracked models** (16 heroes, two minions, two bosses). Every entry has the expected slug/name/collection/license/source URL/author fields. Full model hashes and embedded VRM metadata are in `rg5-3d-provenance.json`. Field completeness does not resolve conflicting source permissions.

Sixteen models' embedded metadata also says CC0. Four conflict with the committed CC0 labels:

| Asset | Embedded evidence | Current source evidence | Disposition |
| --- | --- | --- | --- |
| `avatars/el-bueno.glb` (and its preview) | Redistribution_Prohibited; commercial usage disallowed | Original [NeonGlitch86 repository](https://github.com/neonglitch86/vrm) has no repository license from GitHub's license endpoint (404). OSA registry's collection entry separately labels it CC0. | Unresolved: exclude from distributed content until an explicit later rights-holder grant resolves the conflict. |
| `minions/slime-green.glb` | Other license, explicitly licensed user; linked permission CID | Linked immutable Halloween Rising permission document retains restricted direct usage, credit and license-preservation conditions. | Unresolved: exclude from distribution or replace with approved content; retain local source file. |
| `minions/slime-blue.glb` | Same Halloween permission metadata | Same exact linked permission document. | Same unresolved disposition. |
| `bosses/wendigo-hollow.glb` (and its preview) | Same Halloween permission metadata | Same exact linked permission document. | Same unresolved disposition. |

The Halloween permission document was successfully retrieved from [the exact embedded CID via Pinata](https://gateway.pinata.cloud/ipfs/QmYGUNRqJdkoHyYQtUJXPRqtx1p8javsdv3fADR6mh54en); the body and HTTP headers are retained as `rg5-halloween-license-pinata.*`. The ipfs.io and dweb.link attempts returned a gateway migration notice, not the license; those bodies/headers are also retained.

Current OSA registry [projects.json](https://github.com/ToxSam/open-source-avatars/blob/0f9a1b2fd99894736563d55b2c9dc9125700d081/data/projects.json) labels both Halloween Rising and NeonGlitch86 as CC0. However its [LICENSE](https://github.com/ToxSam/open-source-avatars/blob/0f9a1b2fd99894736563d55b2c9dc9125700d081/LICENSE) applies the repository dedication to registry metadata/documentation/code and explicitly sends readers to individual collection licenses. These statements conflict with the embedded/rightsholder-hosted evidence; a registry label is insufficient to silently rewrite existing restrictive metadata. No contacting of creators or change to source assets was performed. The raw collection entries, license, README and Git blob-hash comparisons to source commit `0f9a1b2fd99894736563d55b2c9dc9125700d081` are preserved in `rg5-osa-*` files.

The retargeted animation source has tracked attribution, file names, clip mapping and source links in `assets-src/animations/README.md`; both source glTF/buffer files are tracked. The [official Quaternius Universal Animation Library page](https://quaternius.com/packs/universalanimationlibrary.html) currently identifies CC0 and permits personal/educational/commercial use. No animation-source licensing conflict was found in this bounded check.

## Clearing this gate

1. Validate the complete rebuilt candidate using the patched crossbeam lock and record the final package identity.
2. Exclude/replace the four disputed models and corresponding previews/manifests, or retain explicit rights-holder evidence of a later grant covering the redistributed derivatives. Required minion/boss visual references must still resolve after any exclusion.
3. Record a final disposition for remaining HTTP/TLS/bytes obligations for the actual distributed binaries and their enabled remote-fetch behavior; retest any patch selected. Do not mark GitHub alerts closed based on this unmerged branch.
