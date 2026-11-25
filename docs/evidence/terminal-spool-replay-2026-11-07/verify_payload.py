#!/usr/bin/env python3
"""Check the archived real terminal receipt against the restored SQL result."""
import json
from pathlib import Path

root = Path(__file__).resolve().parent / "pg16"
receipt = json.loads((root / "actual-terminal-receipt.json").read_text())
actual = json.loads((root / "restored-terminal-result.json").read_text())
assert receipt["kind"] == "Settle"
expected = receipt["result"]
assert expected["saved"] is False and actual["saved"] is True
expected = {**expected, "saved": True}
assert actual == expected, "Restored settlement changed more than its SQL acknowledgement"
print("PASS: complete terminal payload preserved; only saved=false becomes saved=true")
