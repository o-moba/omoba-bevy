#!/usr/bin/env python3
"""Write ../trailer-manifest.json and ../demo-summary.json for the v2 trailer.

    python3 manifest.py [<record_demo output dir> ...]

The output dirs are `scripts/record_demo.py --profile v2` runs (one or several
lanes); their `demo-summary.json` files are merged into ../demo-summary.json.
Without arguments the existing ../demo-summary.json is kept and only the
trailer manifest is rewritten. Hashes and stream facts are read from the files
next to this folder.
"""
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
ORDER = ["phone-wildspark", "phone-stormfist", "phone-emberveil", "phone-warden",
         "desktop-dawnweaver", "desktop-frostguard"]
# The cut, mirroring T in src/Trailer.tsx (seconds).
TIMELINE = [
    (0.0, 4.0, "Cold open: 'A whole MOBA in your pocket'", []),
    (4.0, 12.0, "Reveal: the phone, hero select, 17 classes", ["roster"]),
    (12.0, 20.0, "Wildspark: abilities, close-up, callouts", ["wildspark", "wildsparkPush"]),
    (20.0, 24.0, "Stormfist", ["stormfist"]),
    (24.0, 28.0, "Emberveil", ["emberveil"]),
    (28.0, 32.0, "Warden in the jungle", ["warden"]),
    (32.0, 36.0, "Built for thumbs: joystick and attack cluster", ["thumbs"]),
    (36.0, 40.0, "Desktop: Dawnweaver", ["dawnweaver"]),
    (40.0, 44.0, "Desktop: Frostguard", ["frostguard"]),
    (44.0, 52.0, "Montage, one cut a second: push lanes, clear camps, land combos, take towers", ["montage"]),
    (52.0, 56.6, "End card: OMOBA, links", ["end"]),
]


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def probe(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries",
                          "stream=codec_type,codec_name,width,height,r_frame_rate,pix_fmt,sample_rate,channels:"
                          "format=duration", "-of", "json", str(path)], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


def facts(path):
    data = probe(path)
    info = dict(file=str(path.relative_to(ROOT)), bytes=path.stat().st_size, sha256=sha256(path),
                seconds=round(float(data["format"]["duration"]), 3))
    for stream in data["streams"]:
        if stream["codec_type"] == "video":
            num, den = map(int, stream["r_frame_rate"].split("/"))
            info.update(video_codec=stream["codec_name"], width=stream["width"], height=stream["height"],
                        fps=round(num / den, 3), pixel_format=stream["pix_fmt"])
        elif stream["codec_type"] == "audio":
            info.update(audio_codec=stream["codec_name"], audio_hz=int(stream["sample_rate"]),
                        audio_channels=stream["channels"])
    return info


def shots():
    """Shot names -> [{clip, from}] parsed from src/shots.ts."""
    text = (HERE / "src/shots.ts").read_text()
    table = {}
    for name, body in re.findall(r"^  (\w+): (\{[^\n]*\}|\[\n(?:.*\n)*?  \])", text, flags=re.M):
        table[name] = [dict(clip=clip, at=float(at), seconds=float(length), **(dict(rate=float(rate)) if rate else {}))
                       for clip, at, length, rate in re.findall(
                           r'clip: "([\w-]+)", from: ([\d.]+), len: ([\d.]+)(?:, rate: ([\d.]+))?', body)]
    return table


def main():
    lanes = [Path(arg) for arg in sys.argv[1:]]
    if lanes:
        summaries = [json.loads((lane / "demo-summary.json").read_text()) for lane in lanes]
        clips = {}
        for summary in summaries:
            for clip in summary["clips"]:
                if clip.get("pass"):
                    clip["binary_sha256"] = summary["binaries"]["client"]["sha256"]
                    clip["recorded_on"] = summary["recorded_on"]
                    clips[clip["clip"]] = clip
        merged = dict(summaries[0], clips=[clips[name] for name in ORDER if name in clips])
        merged.pop("binaries", None)
    else:
        merged = json.loads((ROOT / "demo-summary.json").read_text())
        summaries = [merged]
    for clip in merged["clips"]:
        video = ROOT / "sources/clips" / f"{clip['clip']}.mp4"
        clip["video"] = facts(video)
    (ROOT / "demo-summary.json").write_text(json.dumps(merged, indent=2, ensure_ascii=False) + "\n")

    table = shots()
    music = ROOT / "music.mp3"
    prompt = (ROOT / "music.prompt.txt").read_text().strip()
    manifest = dict(
        title="OMOBA gameplay trailer v2",
        made_on=datetime.date.today().isoformat(),
        build=dict(version="0.41.0", source=summaries[0].get("source"),
                   note="dev build with the qa feature; the clips were recorded while the demo director was "
                        "being tuned, so client binaries differ between clips (see demo-summary.json)"),
        video=facts(ROOT / "omoba-trailer-v2.mp4"),
        poster=dict(file="poster.jpg", bytes=(ROOT / "poster.jpg").stat().st_size, sha256=sha256(ROOT / "poster.jpg")),
        capture=dict(
            generator="scripts/record_demo.py --profile v2",
            method="scripted ordinary inputs against the in-process offline practice (server bots, hero at "
                   "level 6); frame-stepped window readbacks, one per 1/60 s of game time, encoded at constant "
                   "60 fps",
            edit_frames="the edit reads still frames extracted from the clips by frame number "
                        "(motion/prepare.py), so every trailer frame shows exactly one captured frame",
            phone_scenes="the 844x390 phone interface rendered by a desktop development build at scale 3 "
                         "(2532x1170), not a device recording",
            desktop_scenes="the 1280x720 desktop layout rendered at scale 2 (2560x1440)",
            clips=[dict(clip=clip["clip"], hero=clip["hero"], script=clip["script"], lane=clip.get("lane"),
                        layout="phone" if clip["touch"] else "desktop", size=clip["size"], scale=clip["scale"],
                        frames=clip["frame_count"], events=[dict(seconds=round(e["seconds"], 2), title=e["title"])
                                                            for e in clip["events"]],
                        binary_sha256=clip["binary_sha256"], video=clip["video"]) for clip in merged["clips"]]),
        edit=dict(
            tool="Remotion 4 (motion/src/Trailer.tsx, shots.ts, lib.tsx)",
            added_in_the_edit="device frame, desktop window frame, titles, ability lists, close-up lens (a crop "
                              "of the same footage), callouts, wipes, flashes, sound effects",
            speed="gameplay plays at recorded speed; the hero-select tour of the reveal scene plays at 1.5x "
                  "(`rate` in the timeline); walks between fights are cut out",
            fonts=["Unbounded Black/SemiBold (OFL)", "Barlow Condensed Bold/SemiBold (OFL, the game's HUD face)",
                   "Manrope ExtraBold (OFL)"],
            timeline=[dict(at=at, to=to, scene=scene,
                           footage=[dict(shot=name, **piece) for name in names for piece in table[name]])
                      for at, to, scene, names in TIMELINE]),
        music=dict(source="ElevenLabs Music API (/v1/music, force_instrumental)", prompt=prompt, file="music.mp3",
                   bytes=music.stat().st_size, sha256=sha256(music), bpm=120, trailer_starts_at_seconds=12.03),
        claims=["Beta 0.41", "17 classes", "5v5 matches, 3 lanes, jungle camps, towers and a Nexus",
                "phone and desktop layouts of the same game", "gameplay captured in offline practice with bots"],
    )
    (ROOT / "trailer-manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    print(ROOT / "trailer-manifest.json")


if __name__ == "__main__":
    main()
