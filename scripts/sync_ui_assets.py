#!/usr/bin/env python3
"""Install the Verdant Crown UI assets from the omoba-ui handoff into the client.

`omoba-ui/handoff/assets/manifest.json` lists every asset with its install
path, 1x/2x sizes, 9-slice insets, atlas grid, tint rule and licence. This
script copies every `ship: true` entry (both densities) to
`client/assets/ui/verdant/<subpath>`, ships the licence texts next to them
(`fonts/OFL-*.txt`, `icons/LICENSES.md`) and writes a trimmed manifest,
`client/assets/ui/verdant/manifest.json`, that the client tests read.

    python3 scripts/sync_ui_assets.py            # install from the handoff (or --source)
    python3 scripts/sync_ui_assets.py --check    # verify the installed set and the package budget
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from sync_ui_tokens import ROOT, find_handoff  # noqa: E402

INSTALL = ROOT / "client/assets/ui/verdant"
MANIFEST = INSTALL / "manifest.json"
# BUILD-ORDER step 0: "the new assets add <= 2.3 MB" (manifest `package_bytes`).
PACKAGE_BUDGET = int(2.3 * 1024 * 1024)
LICENCE_TEXTS = ("fonts/OFL-Cinzel.txt", "fonts/OFL-Inter.txt", "fonts/OFL-BarlowCondensed.txt",
                 "fonts/OFL-NotoSerifSC.txt")
KEPT_FIELDS = ("size_1x", "size_2x", "nine_slice", "atlas", "tintable", "licence", "attribution")


def relative(path: str) -> str:
    """`assets/frames/x@1x.png` -> `frames/x@1x.png` (path under the install root)."""
    if not path.startswith("assets/"):
        raise ValueError(f"asset path outside assets/: {path}")
    return path[len("assets/"):]


def shipped(manifest: dict) -> list[dict]:
    """Trimmed entries for every shipped asset, in manifest order."""
    entries = []
    for asset in manifest["assets"]:
        if not asset.get("ship"):
            continue
        entry = {"path": relative(asset["path"])}
        if asset.get("path_2x"):
            entry["path_2x"] = relative(asset["path_2x"])
        entry["bytes"] = asset["bytes"]
        entry.update({field: asset[field] for field in KEPT_FIELDS if asset.get(field) is not None})
        entries.append(entry)
    return entries


def install(handoff: Path) -> None:
    manifest = json.loads((handoff / "assets/manifest.json").read_text())
    entries = shipped(manifest)
    if INSTALL.exists():
        # The folder holds only what this script installs; stale files would ship.
        shutil.rmtree(INSTALL)
    for entry in entries:
        for key in ("path", "path_2x"):
            if key in entry:
                source = handoff / "assets" / entry[key]
                target = INSTALL / entry[key]
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
    for text in LICENCE_TEXTS:
        shutil.copyfile(handoff / "assets" / text, INSTALL / text)
    MANIFEST.write_text(json.dumps({
        "about": "Installed by scripts/sync_ui_assets.py from omoba-ui/handoff/assets/manifest.json. "
                 "Paths are relative to client/assets/ui/verdant; see that manifest for purpose/source.",
        "credits_required": manifest["credits_required"],
        "package_bytes": manifest["package_bytes"],
        "assets": entries,
    }, indent=1, ensure_ascii=False) + "\n")
    print(f"installed {len(entries)} assets into {INSTALL.relative_to(ROOT)}")


def installed_bytes(entry: dict) -> int:
    return sum((INSTALL / entry[key]).stat().st_size for key in ("path", "path_2x") if key in entry)


def check(handoff: Path | None) -> list[str]:
    if not MANIFEST.is_file():
        return ["client/assets/ui/verdant/manifest.json is missing; run scripts/sync_ui_assets.py"]
    manifest = json.loads(MANIFEST.read_text())
    problems = []
    listed = {MANIFEST.relative_to(INSTALL).as_posix(), *LICENCE_TEXTS}
    total = 0
    for entry in manifest["assets"]:
        for key in ("path", "path_2x"):
            if key in entry:
                listed.add(entry[key])
                if not (INSTALL / entry[key]).is_file():
                    problems.append(f"missing {entry[key]}")
        if not problems:
            size = installed_bytes(entry)
            total += size
            if size != entry["bytes"]:
                problems.append(f"{entry['path']}: {size} bytes installed, manifest says {entry['bytes']}")
    for text in LICENCE_TEXTS:
        if not (INSTALL / text).is_file():
            problems.append(f"missing licence text {text}")
    extra = sorted(path.relative_to(INSTALL).as_posix() for path in INSTALL.rglob("*")
                   if path.is_file() and path.relative_to(INSTALL).as_posix() not in listed)
    problems += [f"unlisted file {path}" for path in extra]
    if total > PACKAGE_BUDGET:
        problems.append(f"UI assets are {total} bytes, over the {PACKAGE_BUDGET} byte budget")
    if handoff is not None and (handoff / "assets/manifest.json").is_file():
        upstream = shipped(json.loads((handoff / "assets/manifest.json").read_text()))
        if upstream != manifest["assets"]:
            problems.append("installed manifest differs from the handoff; run scripts/sync_ui_assets.py")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--source", help="handoff folder (default: search omoba-ui/handoff)")
    parser.add_argument("--check", action="store_true", help="verify the installed assets")
    args = parser.parse_args(argv)
    handoff = find_handoff(args.source)
    if args.check:
        problems = check(handoff)
        for problem in problems:
            print(f"ui assets: {problem}", file=sys.stderr)
        if not problems:
            print("ui assets OK")
        return 1 if problems else 0
    if handoff is None:
        raise SystemExit("handoff not found; pass --source or set OMOBA_UI_HANDOFF")
    install(handoff)
    return 0


if __name__ == "__main__":
    sys.exit(main())
