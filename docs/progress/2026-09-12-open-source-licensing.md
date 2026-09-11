# Open-source licensing and mission — 2026-09-12

Version: 0.19.0-rc.3. The owner explicitly chose to keep developing an open engine
and a MOBA where ordinary people can bring their avatars, with a long-term
objective of a widely accessible competitive game.

## Decision and scope

Use unmodified standard AGPL-3.0-only for server source and MPL-2.0 for the client,
shared crates and reusable source. The server is not available under an optional
weaker license. Shared MPL code remains eligible for the secondary-license route;
no Exhibit B incompatibility designation is added. Existing program boundaries
support this without a gameplay or protocol change.

MPL preserves source-file reciprocity with flexibility for mobile executable
terms; it permits separately authored proprietary additions. AGPL extends source
offer requirements to modified network servers. Both permit commercial forks.
Neither compels upstream pull requests or agreement with the official mission.

Original standalone documentation and identified original Verdant visual content
are CC-BY-4.0. Existing CC0/OFL/imported-asset terms remain unchanged. Creator and
user-avatar rights stay separate; token ownership does not establish copyright.
Root mission, contribution, brand and source-delivery guides explain these
boundaries without assigning contributor copyright or asserting registered marks.

## Implementation and checks

- Root scope guide plus full license texts and seven crate-level LICENSE files.
  Cargo metadata reports AGPL-3.0-only for server and MPL-2.0 for six other crates.
- Native packager includes `legal/`; Android/iOS include `assets/legal/` before
  signing. Existing asset/font notices remain present. Missing/empty required
  notices fail before a build. Revision metadata explicitly does not certify
  source publication or identify uncommitted changes by a revision alone.
- Nine Python packaging regressions pass, including all three packager main paths
  with fixture binaries/tooling, actual legal directory/ZIP bytes, signing order,
  missing notices and preservation of font/asset notices. No native/mobile build,
  store submission or runtime change is claimed for this documentation task.
- Locked/offline workspace metadata, Rust formatting, Python syntax and whitespace
  checks pass. Downloaded license bytes match recorded SHA-256 values from the
  SPDX license-list-data project; the standard license texts are unchanged.
- Six pre-existing asset license/roster files are byte-identical to the base.
  Two independent policy reviews found no substantive defect; wording was refined
  to distinguish standalone documentation from source comments/docstrings.

## Separate release requirements

The exact pinned Ekza SDK revision `8254ed5` has no license grant. Its local history
indicates a common maintainer and extraction from Omoba, but does not replace a
rights-holder's authorization. The owner has been asked whether SDK licensing
may be included; the SDK was not relicensed or changed in this task. Until that
is resolved, do not describe the entire dependency stack as cleared for public
redistribution. A cached inventory covered 633 locked package manifests; the
other previously unspecified licenses were these seven workspace crates.

Packaged legal notices do not themselves publish the matching source, satisfy
all dependency/AGPL Corresponding Source obligations, grant avatar permissions
or obtain store approval. Follow SOURCE.md for a real distribution. No GitHub
push or production deployment is part of this change.

Raw evidence: `.agent/tasks/OPEN-LICENSE-2026-09-12/` in the licensing worktree.
Primary policy sources are linked in the root LICENSING.md.
