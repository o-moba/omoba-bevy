"""Returning-player state for native captures, inside their own scratch folder.

First-launch/account-region tests deliberately do not call this helper. It writes
ordinary client metadata, not a runtime exception to the regional traffic gate.
"""
import json
import os
from pathlib import Path


def prepare_capture_account(scratch):
    """Seed Global routing only, never replacing a caller's existing state.

    Call only with the capture's own TemporaryDirectory. No user settings,
    acceptance record, private key or account credential is read or copied.
    """
    root = Path(scratch).resolve(strict=True)
    directory = root / "config"
    directory.mkdir(exist_ok=True)
    path = directory / "account_region.json"
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        return False
    with os.fdopen(descriptor, "w") as output:
        json.dump({"schema": 1, "selected": "global", "pending": None, "account": None}, output)
    return True
