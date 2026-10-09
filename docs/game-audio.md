# Game audio

The client includes an instrumental background loop and twenty sound cues, and
builds a voice for every skill from six of them (see "Skill voices").
Start normally with `make practice` or join a running server. Audio requires no
separate service, network requests or installation. The existing Bevy audio stack
decodes the bundled Ogg Vorbis assets on desktop and mobile builds.

## Player controls

Open **Menu → Settings** (Escape on desktop). The Audio section contains master,
music, effects and interface volume in 5% steps, plus **Mute sound**. Scroll the
settings panel if a control is below the screen. Touch buttons activate on release;
dragging the panel cancels a button tap. Desktop settings also accept wheel scrolling.

Defaults are 80% master, 25% music, 70% effects and 60% interface. Volume changes
apply to already-playing sounds. Preferences are saved in the existing
`client_preferences.json`; schema5 adds the audio object and preserves older
graphics, character and connection settings. Reset graphics leaves audio alone.

The music is quieter in the menu and after the match, and fades between target
levels. Muting or losing window focus pauses it immediately; resuming continues
the same loop. One-shot sounds are discarded while muted/unfocused. Touch/mobile
and WebAssembly builds wait for first keyboard, mouse or touch input before
starting playback. Browser autoplay and physical-device audio routes still need
their own acceptance checks.

## What makes a sound

| Cue IDs | Source |
| --- | --- |
| `melee`, `arrow`, `arcane`, `holy`, `caster`, `tower` | Accepted server damage receipts, selected by attack style |
| `hit`, `kill`, `death` | Local hero damage and hero kills/death |
| `butterfly` | Confirmed local healing pickup collection, once per receipt |
| `trap_trigger` | Authoritative trap activation damage receipt; owner/victim or nearby visible combat |
| `bluff` | The cast voice of Bluff (`dagger_bluff`), by distance like every other cast voice |
| `vital_break` | A confirmed `near_lethal` receipt of the Adventurer's Backstab |
| `respawn`, `level_up` | Local authoritative alive/level changes |
| `match_start`, `victory`, `defeat` | Observed match transitions and local team outcome |
| `ui_click`, `ui_confirm` | Interface interaction and explicit presentation confirmation |

These are impact sounds, not client-side claims that a predicted attack landed.
The audio cursor baselines a new round/player/connection instead of replaying the
server's retained history. Repeated snapshots do not restart effects. Local hero
notifications have priority over ordinary nearby impacts; minion kills do not
trigger the hero-kill notification.

Distance gain uses simulation X/Z coordinates in both 2D and 3D: full gain within
6 world units, squared falloff to silence at 36. This is distance attenuation,
not stereo positional/HRTF audio or a separate spectator camera listener.
The mixer caps effects at twelve voices, four admissions per frame and eight per
second, with additional per-cue cooldowns. Missing or late assets are dropped,
not queued. An effect without a sink expires after 300 ms; four seconds is the
maximum effect lifetime. Machines without an audio device can still play the game.

## Skill voices

No sample was added for the class visual identity pass: `client/assets/audio`
is unchanged. A skill row of `config/skills.skillfx` gives a voice to up to
four moments of its skill, and each voice is one of the existing samples
played differently:

| Field | Meaning |
| --- | --- |
| `base` | the sample: `melee`, `arrow`, `arcane`, `holy`, `caster`, `tower`, or `bluff` (Bluff only) |
| `speed` | playback speed, 0.70 to 1.40 in steps of 0.05 (15 steps) |
| `slice` | the part that sounds: `full`, `tick` (the first 0.12 s), `body` (0.06 to 0.40 s), `tail` (from 0.20 s on) |
| `gain` | 0.2 to 1, multiplied with the gain the moment is heard at (the distance gain for other heroes) |
| `notes` | up to two later notes of the same sample and slice, each with a delay of at most 240 ms, a speed and a gain |

The moments are `sound.cast` (required for every row: the accepted cast),
`sound.recast` (a second press of a row that has one), `sound.release` (a
telegraph that fired) and `sound.impact` (a confirmed hit by a hero the client
sees). A basic-attack row has one voice, heard on its confirmed hit and, for an
enemy, on its accepted attack. The Wildspark has a voice for each of its two
rounds. The 68 cast voices are pairwise different (`base`, `speed`, `slice` and
notes), which the identity test checks.

Rules of the mixer for these voices:

- Later notes belong to the local hero's own action. Everyone else is heard
  with the first note alone.
- A voice is admitted with every note it has or refused whole: the voice
  limit, the frame limit and the rate tokens must cover all of them in the
  frame that asks.
- The cooldown is kept per variant (sample, speed step and slice), so two
  variants of one sample do not silence each other, while several receipts of
  one skill in one frame are heard once.
- A voice of a row is played up to 3 % faster or slower, by a fixed amount
  that follows from the action, receipt or effect behind it, so a skill that
  hits many times does not repeat one identical sound.
- Row voices come after the existing cues in the admission order: they never
  displace a hit, kill or trap cue.
- A receipt whose source is a hero the client does not see keeps the cue of
  its wire style; the trap cue, the hit cue and a death are still heard, a
  kill cue is not claimed for an unnamed attacker.
- A block of Northwall has no sound.

Nobody has listened to these voices in this pass. The evidence is the voice
that was resolved, the values handed to the playback settings in a headless
app and the decoded slice lengths; whether the sink honours speed, start and
length on every platform, and how the 68 voices sound together, needs a
listening pass.

## Art, rights and size

The complete twenty-one-file palette is approximately **1.89 MB**. Music is stereo;
effects are mono. All clips use Vorbis at 44.1 kHz.

- **Exploration Theme**, Cleyton Kauffman — [original author page](https://opengameart.org/content/exploration-theme), CC0. The author marks the 134.4-second composition as seamless. The packaged version is encoded from the lossless source with its duration and loop boundaries retained.
- Two adapted foley clips from **Kenney RPG Audio** — [original source](https://kenney.nl/assets/rpg-audio), CC0.
- Eighteen original effects synthesized from mathematical tones and filtered noise, with short authored musical motifs and echoes. No imported instrument samples, voices or commercial-game melodies are used.

`client/assets/audio/LICENSE.md`, both provenance JSON files and the `licenses/`
directory record terms, credits, exact source/member/output hashes and adaptations.
Keep these files with packaged audio. Code stays MPL-2.0; asset rights are recorded
separately. Download archives and temporary PCM output are development materials.

## Changing the palette

`client/assets/audio/manifest.json` is a version1 catalog embedded at build time:

```json
{
  "version": 1,
  "music": {"path": "audio/music/arena.ogg", "gain": 0.8},
  "cues": {
    "melee": {"path": "audio/sfx/melee.ogg", "gain": 0.6}
  }
}
```

This excerpt shows the shape; the real file must contain all twenty known cue
IDs. Gains must be finite in 0–1. Music paths must be `audio/music/<name>.ogg`;
effect paths must be `audio/sfx/<name>.ogg`, using letters, digits, hyphens or
underscores. URLs, traversal, nested paths and unknown/missing cues are rejected.
An invalid catalog falls back to the built-in bundled names. Missing files fail
quietly at the gameplay boundary while the asset loader reports their error.

Replace a cue's local file/path and rebuild to change its sound across matching
events. Attack style already separates class sounds. This release does not choose
different sounds by individual avatar/NFT ownership; such a future adapter can
resolve trusted cue assets without granting gameplay authority or accepting
arbitrary remote URLs.

To regenerate the eighteen original sounds and two normalized foley adaptations:

```sh
python3 scripts/build_audio_palette.py --kenney-archive /path/to/kenney-rpg-audio.zip
```

Use `--ffmpeg /path/to/ffmpeg` if the default executable lacks `libvorbis` (for
example an existing `ffmpeg-full` installation). Python uses only its standard
library; FFmpeg is an offline authoring tool, not a new game dependency. Exact
encoded bytes may depend on encoder version; packaged hashes identify the release.

## Verification and remaining scope

Task evidence lives in `.agent/tasks/GAME-AUDIO-2026-09-14/`. It distinguishes
event/persistence tests, complete-file decode/level checks, actual native sink
observations and desktop/phone-sized settings captures. Its 32-second audio
preview is an authored palette demonstration, not a recording of game output.

Native automation scripts normal controls and a single explicit confirmation
cue while connected to a real Practice server. It checks decoding, advancing
music position, mute/resume, volume changes and saved preferences. This does not
establish subjective listening quality, physical Android/iOS output, Bluetooth
latency, browser autoplay or production load. Those require device playtests.
