#!/usr/bin/env python3
"""Run the runtime phone geometry contract with rustc, without Bevy or rendering."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    with tempfile.TemporaryDirectory(prefix="omoba-hud-layout-") as scratch:
        binary = Path(scratch) / "phone-layout-tests"
        subprocess.run(["rustc", "--edition=2024", "--test",
                        str(ROOT / "client/src/hud_layout/phone_rows.rs"),
                        "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)

if __name__ == "__main__":
    main()
