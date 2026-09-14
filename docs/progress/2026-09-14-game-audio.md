# Game audio — 2026-09-14

Base: `747b991`. Release version: `0.19.0-rc.8`.
Current-source verification passes; publication is recorded in the task evidence.

The existing bot/social work was already committed and pushed to main. This
isolated follow-up adds a small CC0 audio palette, local event-based playback and
persisted desktop/mobile audio controls. Unrelated cinematic files are preserved.

Music: Exploration Theme by Cleyton Kauffman, a 134.4-second author-designated loop.
Sound: two adapted Kenney RPG Audio clips and fourteen original deterministic
synthesized cues. All seventeen Vorbis files total approximately 1.86 MB. Source
archives stay outside the committed assets; source notices and hashes are retained.

The client uses authoritative combat receipts for style-specific impacts and local
match/player state for notifications. Voice/rate limits, stale-event baselines,
distance attenuation and focus/mute rules keep the mix bounded. Audio does not
modify networking, game rules, ranked progression or cosmetic ownership.

The settings menu adds master/music/effects/UI controls and mute, persists them
through schema5 preferences, and preserves old graphics migrations. The cue
manifest is intentionally local and versioned; native Bevy audio and Vorbis
support were already present, so no production dependency was introduced.

See [the authoring/run guide](../game-audio.md) and
`.agent/tasks/GAME-AUDIO-2026-09-14/evidence.md` for exact acceptance results,
decoded playback diagnostics, preview files and limits. No physical mobile,
subjective listening, public deployment or browser autoplay claim is implied.

Verification: 628 workspace tests passed (14 PostgreSQL tests ignored), followed
by 341 passing final client tests after client-only refinements. Strict workspace
Clippy, formatting, the final native build and asset decoding pass. Three actual
native Practice connections verified audio sinks, music position, mute/resume and
saved settings; six desktop/phone 2D/3D captures were visually reviewed. All nine
audio buttons fit the compact phone settings panel. These are overlapping test
runs and native phone previews, not physical-device acceptance.
