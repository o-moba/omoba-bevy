#!/usr/bin/env python3
"""Mirror the Verdant Crown design tokens from the omoba-ui handoff into the game.

`omoba-ui/handoff/tokens.json` is the single source of truth for the UI's
colours, type, spacing, sizes and motion (DECISIONS R4: tokens stay data). The
client keeps a byte-identical copy in `client/ui/tokens/verdant-crown.json`;
`client/build.rs` turns it into typed Rust constants, so a token the code names
but the data lacks is a compile error.

    # copy from the handoff (found next to this repository, or --source), validate, record the hash
    python3 scripts/sync_ui_tokens.py
    # CI/tests: validate the copy and its recorded hash; compare with the handoff when it is present
    python3 scripts/sync_ui_tokens.py --check

The lock file `client/ui/tokens/verdant-crown.lock.json` records where the copy
came from and its SHA-256, so a hand edit of the copy fails `--check`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys

ROOT = Path(__file__).resolve().parents[1]
COPY = ROOT / "client/ui/tokens/verdant-crown.json"
LOCK = ROOT / "client/ui/tokens/verdant-crown.lock.json"
HANDOFF_ENV = "OMOBA_UI_HANDOFF"
HANDOFF_RELATIVE = Path("omoba-ui/handoff")

COLOUR = re.compile(r"^#[0-9A-F]{8}$")
BEZIER = re.compile(r"^cubic-bezier\(\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*\)$")
TYPE_FIELDS = {
    "family", "size.desktop", "size.phone", "line_height", "letter_spacing_em", "case",
    "cjk.family", "cjk.letter_spacing_em",
}
GROUPS = {"color", "font", "type", "space", "radius", "border", "size", "motion"}
# Keys whose value is prose for the engineer, not a number (kept as a documented string).
NOTE_KEYS = {"space.safe_area.phone"}


def find_handoff(explicit: str | None = None) -> Path | None:
    """The handoff folder: --source, $OMOBA_UI_HANDOFF, or omoba-ui/handoff in an ancestor."""
    if explicit:
        path = Path(explicit)
        return path.parent if path.is_file() else path
    if os.environ.get(HANDOFF_ENV):
        return Path(os.environ[HANDOFF_ENV])
    for parent in ROOT.parents:
        candidate = parent / HANDOFF_RELATIVE
        if (candidate / "tokens.json").is_file():
            return candidate
    return None


def font_asset(path: str) -> Path:
    """Where a `font.family.*` file lives in the client (install root of the handoff manifest)."""
    if path.startswith("assets/"):
        return ROOT / "client/assets/ui/verdant" / path[len("assets/"):]
    if path.startswith("client/assets/"):
        return ROOT / path
    raise ValueError(f"font path outside assets/ or client/assets/: {path}")


def validate(tokens: dict, check_fonts: bool = True) -> list[str]:
    """Every problem with a token file (empty = valid). Mirrors what client/build.rs accepts."""
    problems = []
    if tokens.get("$schema_version") != 1:
        problems.append("$schema_version must be 1")
    styles: dict[str, set[str]] = {}
    for key, value in tokens.items():
        if key.startswith("$"):
            continue
        group = key.split(".", 1)[0]
        if group not in GROUPS:
            problems.append(f"{key}: unknown group {group!r}")
            continue
        if not re.fullmatch(r"[a-z0-9_]+(\.[a-z0-9_]+)+", key):
            problems.append(f"{key}: keys are dot-separated lowercase words")
        if group == "color":
            if not (isinstance(value, str) and COLOUR.match(value)):
                problems.append(f"{key}: colour must be #RRGGBBAA, got {value!r}")
        elif group == "font":
            if not key.startswith("font.family.") or not isinstance(value, str):
                problems.append(f"{key}: fonts are font.family.* file paths")
            elif check_fonts:
                try:
                    if not font_asset(value).is_file():
                        problems.append(f"{key}: font file {value} is not installed")
                except ValueError as error:
                    problems.append(f"{key}: {error}")
        elif group == "type":
            parts = key.split(".")
            if len(parts) < 3:
                problems.append(f"{key}: type keys are type.<style>.<field>")
                continue
            field = ".".join(parts[2:])
            styles.setdefault(parts[1], set()).add(field)
            if field in {"family", "cjk.family"}:
                if value not in tokens or not str(value).startswith("font.family."):
                    problems.append(f"{key}: {value!r} is not a font.family token")
            elif field == "case":
                if value not in {"upper", "none"}:
                    problems.append(f"{key}: case is upper or none")
            elif field not in TYPE_FIELDS:
                problems.append(f"{key}: unknown type field {field!r}")
            elif not isinstance(value, (int, float)) or isinstance(value, bool):
                problems.append(f"{key}: must be a number")
        elif key in NOTE_KEYS:
            if not isinstance(value, str):
                problems.append(f"{key}: note must be a string")
        elif key.startswith("motion.easing."):
            if not (isinstance(value, str) and BEZIER.match(value)):
                problems.append(f"{key}: easing must be cubic-bezier(a, b, c, d)")
        elif not isinstance(value, (int, float)) or isinstance(value, bool):
            problems.append(f"{key}: must be a number, got {value!r}")
    for style, fields in sorted(styles.items()):
        missing = TYPE_FIELDS - fields
        if missing:
            problems.append(f"type.{style}: missing {', '.join(sorted(missing))}")
    return problems


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sync(handoff: Path) -> None:
    source = handoff / "tokens.json"
    tokens = json.loads(source.read_text())
    problems = validate(tokens)
    if problems:
        raise SystemExit("handoff tokens are invalid:\n  " + "\n  ".join(problems))
    COPY.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, COPY)
    LOCK.write_text(json.dumps({
        "source": "omoba-ui/handoff/tokens.json",
        "sha256": sha256(COPY),
        "keys": sum(1 for key in tokens if not key.startswith("$")),
    }, indent=2) + "\n")
    print(f"copied {source} -> {COPY.relative_to(ROOT)} ({sha256(COPY)[:12]})")


def check(handoff: Path | None) -> list[str]:
    problems = []
    if not COPY.is_file() or not LOCK.is_file():
        return [f"missing {COPY.relative_to(ROOT)} or its lock; run scripts/sync_ui_tokens.py"]
    tokens = json.loads(COPY.read_text())
    problems += validate(tokens)
    lock = json.loads(LOCK.read_text())
    if lock.get("sha256") != sha256(COPY):
        problems.append("the token copy was edited by hand: change omoba-ui/handoff/tokens.json "
                        "and re-run scripts/sync_ui_tokens.py")
    if handoff is not None and (handoff / "tokens.json").is_file():
        if (handoff / "tokens.json").read_bytes() != COPY.read_bytes():
            problems.append(f"the token copy differs from {handoff / 'tokens.json'}; "
                            "run scripts/sync_ui_tokens.py")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--source", help="handoff folder or tokens.json (default: search omoba-ui/handoff)")
    parser.add_argument("--check", action="store_true", help="validate the copy instead of syncing")
    args = parser.parse_args(argv)
    handoff = find_handoff(args.source)
    if args.check:
        problems = check(handoff)
        for problem in problems:
            print(f"ui tokens: {problem}", file=sys.stderr)
        if not problems:
            where = handoff / "tokens.json" if handoff else "no handoff checkout (copy and hash only)"
            print(f"ui tokens OK ({where})")
        return 1 if problems else 0
    if handoff is None:
        raise SystemExit(f"handoff not found; pass --source or set {HANDOFF_ENV}")
    sync(handoff)
    return 0


if __name__ == "__main__":
    sys.exit(main())
