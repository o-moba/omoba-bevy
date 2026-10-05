# Community beta 0.43.0 preparation

Freeze the current Bevy 0.19.1 game as the first numbered community beta. Preserve the 0.43.0 application version and mark the GitHub release as a prerelease. The canonical release policy now defines branches, immutable tags, version increments, server-first compatibility, signing, evidence and rollback. Tester instructions and GitHub issue templates make feedback reproducible.

Release packaging corrects its default-server instructions, carries the compatibility manifest/source commit into release assets and rejects tagged/manual Android builds without the persistent signing key. No gameplay, protocol or schema change is part of this release preparation. Final delivery evidence is retained in `.agent/tasks/BETA-043-20261005/`; a build, upload, publication and physical-device test must be reported separately.
