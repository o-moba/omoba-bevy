#!/usr/bin/env python3
"""Read-only interoperability check for receipts written by the real Rust test."""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ops/vps/ru"))
import ru_monitor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--now-ms", type=int, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    if not root.name.startswith("omoba-outbox-unit-"):
        raise ValueError("expected the Rust test's own temporary fixture root")
    monitor = ru_monitor.Monitor(ru_monitor.Config(str(root / "missing-alerts.env")),
                                str(root / "unused-state"), root=str(root),
                                now=lambda: args.now_ms / 1000)
    result_problems, results = monitor.settlement_problems()
    queue_problems, queue = monitor.queue_dodge_problems()
    if ([p[0] for p in result_problems] != ["settlement:outbox"]
            or [p[0] for p in queue_problems] != ["queue-dodge:journal"]
            or results["stale"] != 1 or queue["stale"] != 1
            or results["unreadable"] or queue["unreadable"]):
        raise RuntimeError("real native receipt/monitor contract mismatch")
    print(json.dumps({"status": "PASS", "settlements": results, "queue_dodges": queue}))


if __name__ == "__main__":
    main()
