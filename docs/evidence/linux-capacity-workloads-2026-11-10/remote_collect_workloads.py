"""Archive only selected non-secret acceptance records from three finished runs."""
from pathlib import Path
import io
import tarfile

root = Path("/root/omoba-capacity-73651bc6")
out = Path("/root/omoba-capacity-73651bc6-evidence.tar.gz")
labels = ("native-bots-10-30", "native-ranked-10-30", "native-mixed-30-30")
assert not out.exists()
selected = []
for label in labels:
    run = root / label
    selected.extend((run / "report.json", run / "owned-processes.json", root / f"{label}-summary.json"))
    matches = sorted(path for path in (run / "matches").iterdir() if path.is_dir())
    assert matches
    for match in matches:
        selected.extend((
            match / "manifest.json", match / "status.json", match / "worker.log",
            match / "outbox/health/status.json",
        ))
for path in selected:
    assert path.is_file(), path
    if path.suffix in (".json", ".log"):
        text = path.read_text(encoding="utf-8", errors="strict")
        assert "postgres://" not in text and "postgresql://" not in text, path
        assert "PRIVATE KEY" not in text, path
with tarfile.open(out, "w:gz") as archive:
    for path in selected:
        member = archive.gettarinfo(str(path), arcname=str(path.relative_to(root)))
        member.mtime = 0
        member.uid = member.gid = 0
        member.uname = member.gname = ""
        with path.open("rb") as source:
            archive.addfile(member, source)
print(f"{out} {len(selected)} files {out.stat().st_size} bytes")
