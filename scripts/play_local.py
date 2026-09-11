#!/usr/bin/env python3
"""Build current locked sources and play a local 3D match with nine bots."""
import argparse
from pathlib import Path
import signal
import sys

import beta_launcher
from package_native import build_executables


ROOT = Path(__file__).resolve().parents[1]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind", type=beta_launcher.address, default="127.0.0.1:4000",
                        help="local server address (default: 127.0.0.1:4000)")
    parser.set_defaults(action="practice")
    args = parser.parse_args(argv)

    def interrupted(_number, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        print("Building current sources with Cargo.lock (dev profile)...", flush=True)
        executables = build_executables("dev")
        print("Choose a hero, then Join. Close the game or press Ctrl+C to stop the session.", flush=True)
        return beta_launcher.run(args, ROOT / "target/local-play", executables=executables,
                                 assets=ROOT / "client/assets")
    except KeyboardInterrupt:
        print("Local session stopped.", file=sys.stderr)
        return 130
    except (OSError, RuntimeError) as error:
        print(f"Local launch failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
