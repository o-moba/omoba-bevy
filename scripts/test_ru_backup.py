"""ops/vps/ru/ru_backup.py: encrypted off-host backups, escrow and the restore drill (LR-03).

Without a database: settings, targets (a folder, a fake S3 server that checks every
SigV4 signature, a fake `sftp` that executes the batch commands), retention, alerts,
the health check, and whole backup runs over many simulated nights with fake pg_dump /
pg_restore / psql and the real GnuPG. With a disposable PostgreSQL
(OMOBA_BACKUP_TEST_PG=postgres://owner@127.0.0.1:55581/postgres, a server where the test
may create and drop the databases omoba_backup_e2e and omoba_backup_e2e_restore_drill;
psql, pg_dump and pg_restore on PATH or in OMOBA_BACKUP_TEST_PG_BIN): the real career
and portal migrations, a backup, the monthly drill, fetch, and drills that must fail
on a damaged set or on restored rows that differ from the backup.
"""
from __future__ import annotations

import atexit
import contextlib
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/vps/ru"))

import ru_backup as rb  # noqa: E402

Context = rb.Context   # tests patch rb.Context to give main() a clock; helpers keep the real one

HAS_GPG = bool(shutil.which("gpg") and shutil.which("gpgconf"))
DAY = 86400
T0 = 1_790_000_000  # 2026-09-21
_KEYS = {}


def keys() -> dict:
    """Two backup key pairs, made once with `ru_backup.py keygen`."""
    if not _KEYS:
        folder = tempfile.mkdtemp(prefix="omoba-backup-keys-")
        atexit.register(shutil.rmtree, folder, True)
        for name in ("a", "b"):
            _KEYS[name] = rb.run_keygen(os.path.join(folder, name))
    return _KEYS


def write(path, text, mode=0o600):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(text)
    os.chmod(path, mode)
    return str(path)


def env_file(folder, **values) -> str:
    return write(Path(folder) / "backup.env", "".join(f"{key}={value}\n" for key, value in values.items()))


class Clock:
    def __init__(self, start=T0):
        self.value = start

    def __call__(self):
        return self.value


class Telegram:
    """Collects the Telegram requests a Context sends."""

    def __init__(self, fail=None):
        self.sent, self.fail = [], fail

    def __call__(self, request, timeout=None):
        if self.fail:
            raise self.fail
        self.sent.append((request.full_url, urllib.parse.parse_qs(request.data.decode())["text"][0]))
        return contextlib.nullcontext()


def fake_bin(folder, scripts: dict) -> str:
    """Executables written as Python scripts into folder/bin; returns that folder."""
    bin_dir = Path(folder) / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    for name, body in scripts.items():
        path = bin_dir / name
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)
    return str(bin_dir)


# A dump as `pg_restore --data-only -f -` prints it (PostgreSQL 18 adds the \restrict lines).
DATA = '''--
-- PostgreSQL database dump
--
\\restrict abc
SET statement_timeout = 0;
COPY portal.schema_version (version) FROM stdin;
1
2
3
4
5
6
7
\\.
COPY public.career_schema_version (version) FROM stdin;
1
2
3
\\.
COPY public.career_profiles (profile_id, nickname, rating) FROM stdin;
aaaa\tНика\t1234
bbbb\ttab\\there\t987
\\.
COPY public."Weird ""Name""" ("x y") FROM stdin;
\\.
SELECT pg_catalog.setval('public.career_matches_seq_seq', 4, true);
\\unrestrict abc
'''
VERSIONS = {"career": [1, 2, 3], "portal": [1, 2, 3, 4, 5, 6, 7]}

FAKE_PG = {
    "pg_dump": """import os, sys
folder = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "pg")
with open(os.path.join(folder, "log"), "a") as log:
    log.write("pg_dump " + " ".join(sys.argv[1:]) + "\\n")
if os.path.exists(os.path.join(folder, "fail_dump")):
    sys.stderr.write("pg_dump: error: connection to server failed\\n")
    sys.exit(1)
sys.stdout.buffer.write(b"PGDMP" + open(os.path.join(folder, "data.txt"), "rb").read())
""",
    "pg_restore": """import sys
assert sys.argv[1:4] == ["--data-only", "-f", "-"], sys.argv
data = open(sys.argv[4], "rb").read()
assert data.startswith(b"PGDMP")
sys.stdout.buffer.write(data[5:])
""",
    "psql": """import json, os, sys
sql = sys.stdin.read()
folder = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "pg")
versions = json.load(open(os.path.join(folder, "versions.json")))
if "to_regclass" in sql:
    print("|".join("t" if key in versions else "f" for key in ("career", "portal")))
elif "json_build_object" in sql:
    print(json.dumps(versions))
else:
    sys.exit("fake psql: unexpected SQL " + sql[:80])
""",
}


# ---------------------------------------------------------------- pure functions

class ParsingTest(unittest.TestCase):
    def test_settings_files_reject_typos_and_duplicates(self):
        values, problems = rb.parse_env_text("# c\nOMOBA_BACKUP_TARGET='s3://b-1/x'\nOMOBA_BACKUP_TARGT=x\n"
                                             "OMOBA_BACKUP_KEEP_DAILY=3\nOMOBA_BACKUP_KEEP_DAILY=4\nnot a line\n")
        self.assertEqual(values["OMOBA_BACKUP_TARGET"], "s3://b-1/x")
        self.assertEqual(problems, ["unknown setting OMOBA_BACKUP_TARGT", "OMOBA_BACKUP_KEEP_DAILY is set twice",
                                    "line 6 is not KEY=VALUE"])
        config = rb.Config("/nonexistent", values={"OMOBA_BACKUP_TARGET": "CHANGE_ME"})
        self.assertEqual(config.get("OMOBA_BACKUP_TARGET"), "", "a placeholder is unset")
        self.assertEqual(config.get("OMOBA_BACKUP_DATABASE"), "omoba_ru")
        self.assertEqual(config.drill_database, "omoba_ru_restore_drill")
        self.assertEqual(config.paths("OMOBA_BACKUP_STATE_DIRS"),
                         ["/var/lib/omoba-ru/matches", "/var/lib/omoba-ru/lobby-outbox"])

    def test_targets(self):
        self.assertEqual(rb.parse_target("s3://omoba-ru-backups/prod/db"),
                         {"scheme": "s3", "bucket": "omoba-ru-backups", "prefix": "prod/db"})
        self.assertEqual(rb.parse_target("sftp://backup@store.example.ru:2222/srv/omoba/"),
                         {"scheme": "sftp", "user": "backup", "host": "store.example.ru", "port": 2222,
                          "path": "/srv/omoba"})
        self.assertEqual(rb.parse_target("file:///mnt/backup")["path"], "/mnt/backup")
        for bad in ("s3://Bad_Bucket/x", "s3://bucket/../x", "sftp://u:pw@h/srv", "sftp://h/srv", "sftp://u@h",
                    "sftp://u@h/srv/../etc", "sftp://u@h/srv x", "sftp://u@h:99999/x", "s3://b-1/x?versionId=1",
                    "file://relative", "rsync://u@h/x", "", "CHANGE_ME"):
            with self.assertRaises(rb.BackupError, msg=bad):
                rb.parse_target(bad)

    def test_copy_headers(self):
        self.assertEqual(rb.copy_table("COPY public.career_profiles (profile_id, nickname) FROM stdin;"),
                         "public.career_profiles")
        self.assertEqual(rb.copy_table('COPY public."Weird ""Name""" ("x y") FROM stdin;'), 'public."Weird ""Name"""')
        self.assertEqual(rb.copy_table('COPY "my.schema"."t (x)" (a) FROM stdin;'), '"my.schema"."t (x)"')
        self.assertIsNone(rb.copy_table("COPY career_profiles (a) FROM stdin;"), "always schema-qualified")
        self.assertIsNone(rb.copy_table("SELECT 1;"))

    def test_retention_keeps_14_daily_and_8_weekly(self):
        names = [f"omoba-ru-{time.strftime(rb.STAMP, time.gmtime(T0 + day * DAY + 1800))}-nightly.tar.gpg"
                 for day in range(120)]
        keep = rb.retained(names)
        newest = sorted(names, reverse=True)
        self.assertTrue(set(newest[:14]) <= keep, "the last 14 nights")
        weekly = keep - set(newest[:14])
        self.assertEqual(len(keep), 14 + len(weekly))
        weeks = {rb.set_time(name).isocalendar()[:2] for name in keep}
        self.assertEqual(len(weeks), 8, "eight distinct weeks")
        self.assertTrue(all(rb.set_time(name).weekday() == 6 for name in weekly), "the newest set of a week")
        self.assertEqual(rb.retained(names, daily=3, weekly=0), set(newest[:3]))
        # A month without backups never deletes the last good ones.
        old = names[:20]
        self.assertTrue(set(sorted(old, reverse=True)[:14]) <= rb.retained(old))
        # Several sets a day: the newest of that day; foreign names are never touched.
        day = ["omoba-ru-20260921T003000Z-nightly.tar.gpg", "omoba-ru-20260921T150000Z-pre-migration.tar.gpg",
               "notes.txt", "omoba-ru-20260921T150000Z-nightly.tar.gpg.part"]
        self.assertEqual(rb.retained(day), {"omoba-ru-20260921T150000Z-pre-migration.tar.gpg"})
        self.assertEqual(rb.retained([]), set())

    def test_sigv4_matches_the_published_s3_examples(self):
        secret = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        host = "examplebucket.s3.amazonaws.com"
        get = rb.sigv4("GET", f"https://{host}/test.txt",
                       {"host": host, "range": "bytes=0-9", "x-amz-content-sha256": rb.EMPTY_SHA256,
                        "x-amz-date": "20130524T000000Z"}, rb.EMPTY_SHA256, "us-east-1", "AKIAIOSFODNN7EXAMPLE",
                       secret, "20130524T000000Z")
        self.assertTrue(get.endswith("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"))
        self.assertIn("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date", get)
        listing = rb.sigv4("GET", f"https://{host}/?max-keys=2&prefix=J",
                           {"host": host, "x-amz-content-sha256": rb.EMPTY_SHA256, "x-amz-date": "20130524T000000Z"},
                           rb.EMPTY_SHA256, "us-east-1", "AKIAIOSFODNN7EXAMPLE", secret, "20130524T000000Z")
        self.assertTrue(listing.endswith("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"))

    def test_redaction(self):
        text = rb.redact("https://api.telegram.org/bot123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/sendMessage failed; "
                         "postgres://u:pw@h/db; key s3cr3t-value", ["s3cr3t-value"])
        self.assertNotIn("AAAAAAAA", text)
        self.assertNotIn(":pw@", text)
        self.assertNotIn("s3cr3t-value", text)


class ScanDumpTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        folder = self.work.name
        bin_dir = fake_bin(folder, {"pg_restore": FAKE_PG["pg_restore"]})
        patcher = mock.patch.dict(os.environ, {"PATH": bin_dir + os.pathsep + os.environ.get("PATH", "")})
        patcher.start()
        self.addCleanup(patcher.stop)

    def dump(self, text: str, name="db.dump") -> str:
        path = Path(self.work.name) / name
        path.write_bytes(b"PGDMP" + text.encode())
        return str(path)

    def test_rows_digests_and_versions_come_from_the_copy_data(self):
        scan = rb.scan_dump(self.dump(DATA))
        tables = scan["tables"]
        self.assertEqual(scan["schema_versions"], VERSIONS)
        self.assertEqual(tables["public.career_profiles"]["rows"], 2)
        self.assertEqual(tables['public."Weird ""Name"""'], {"rows": 0, "digest": "0" * 32})
        self.assertEqual(set(tables), {"portal.schema_version", "public.career_schema_version",
                                       "public.career_profiles", 'public."Weird ""Name"""'})
        # Row order does not matter; a changed rating does.
        swapped = DATA.replace("aaaa\tНика\t1234\nbbbb\ttab\\there\t987", "bbbb\ttab\\there\t987\naaaa\tНика\t1234")
        self.assertNotEqual(swapped, DATA)
        self.assertEqual(rb.scan_dump(self.dump(swapped, "b.dump"))["tables"], tables)
        changed = rb.scan_dump(self.dump(DATA.replace("1234", "1235"), "c.dump"))["tables"]
        self.assertNotEqual(changed["public.career_profiles"]["digest"], tables["public.career_profiles"]["digest"])

    def test_a_truncated_dump_is_an_error(self):
        with self.assertRaises(rb.BackupError):
            rb.scan_dump(self.dump(DATA.split("\\.\nCOPY public.career_profiles")[0] + "\\.\nCOPY public.t (a) FROM stdin;\n1\n"))


# ---------------------------------------------------------------- targets

class FakeS3(BaseHTTPRequestHandler):
    """A bucket that refuses any request whose SigV4 signature or payload hash is wrong."""
    objects: dict = {}
    region, access, secret, page = "ru-central1", "AKIDOMOBA", "s3-secret-value", 2
    requests: list = []

    def log_message(self, *args):
        pass

    def verified(self, body=b"") -> bool:
        claimed = self.headers.get("x-amz-content-sha256")
        url = f"http://{self.headers['Host']}{self.path}"
        expected = rb.sigv4(self.command, url, {"host": self.headers["Host"], "x-amz-content-sha256": claimed,
                                                "x-amz-date": self.headers["x-amz-date"]},
                            claimed, self.region, self.access, self.secret, self.headers["x-amz-date"])
        if self.headers.get("Authorization") != expected or (self.command == "PUT" and
                                                              hashlib.sha256(body).hexdigest() != claimed):
            self.reply(403, b"<Error><Code>SignatureDoesNotMatch</Code></Error>")
            return False
        type(self).requests.append((self.command, self.path))
        return True

    def reply(self, code, body=b"", length=None):
        self.send_response(code)
        self.send_header("Content-Length", str(len(body) if length is None else length))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def key(self):
        path = urllib.parse.urlsplit(self.path).path
        assert path.startswith("/omoba-ru/"), path
        return urllib.parse.unquote(path[len("/omoba-ru/"):])

    def do_PUT(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        if self.verified(body):
            self.objects[self.key()] = body
            self.reply(200)

    def do_HEAD(self):
        if self.verified():
            data = self.objects.get(self.key())
            self.reply(404) if data is None else self.reply(200, length=len(data))

    def do_DELETE(self):
        if self.verified():
            self.objects.pop(self.key(), None)
            self.reply(204)

    def do_GET(self):
        if not self.verified():
            return
        parts = urllib.parse.urlsplit(self.path)
        if parts.path != "/omoba-ru":
            data = self.objects.get(self.key())
            return self.reply(404) if data is None else self.reply(200, data)
        query = dict(urllib.parse.parse_qsl(parts.query))
        keys = sorted(key for key in self.objects if key.startswith(query.get("prefix", "")))
        start = int(query.get("continuation-token", "+/=0").split("=")[-1])
        page, more = keys[start:start + self.page], start + self.page < len(keys)
        xml = '<?xml version="1.0"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
        xml += "".join(f"<Contents><Key>{key}</Key><Size>{len(self.objects[key])}</Size></Contents>" for key in page)
        xml += f"<IsTruncated>{'true' if more else 'false'}</IsTruncated>"
        if more:   # a token with characters that must be encoded the same way on both sides
            xml += f"<NextContinuationToken>+/={start + self.page}</NextContinuationToken>"
        self.reply(200, (xml + "</ListBucketResult>").encode())


class TargetTest(unittest.TestCase):
    NAMES = [f"omoba-ru-2026092{day}T003000Z-nightly.tar.gpg" for day in range(1, 6)]

    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.local = Path(self.work.name) / "set.tar.gpg"
        self.local.write_bytes(os.urandom(3000))

    def exercise(self, target):
        for name in self.NAMES:
            target.put(str(self.local), name)
        self.assertEqual(target.list(), {name: 3000 for name in self.NAMES})
        out = Path(self.work.name) / "back"
        target.get(self.NAMES[2], str(out))
        self.assertEqual(out.read_bytes(), self.local.read_bytes())
        target.delete(self.NAMES[:2])
        self.assertEqual(sorted(target.list()), self.NAMES[2:])

    def test_a_folder(self):
        folder = Path(self.work.name) / "off"
        target = rb.FileTarget(str(folder))
        self.assertEqual(target.list(), {})
        self.exercise(target)
        (folder / "x.tar.gpg.part").write_text("partial")
        self.assertNotIn("x.tar.gpg.part", target.list())
        self.assertEqual(os.stat(folder / self.NAMES[3]).st_mode & 0o777, 0o600)

    def s3(self, secret=FakeS3.secret, prefix="prod/db"):
        server = ThreadingHTTPServer(("127.0.0.1", 0), FakeS3)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        FakeS3.objects, FakeS3.requests = {"prod/db/notes.txt": b"x", "other/omoba-ru-x.tar.gpg": b"y"}, []
        return rb.S3Target(f"http://127.0.0.1:{server.server_port}", FakeS3.region, "omoba-ru", prefix,
                           FakeS3.access, secret, clock=lambda: T0)

    def test_s3_compatible_storage(self):
        target = self.s3()
        self.exercise(target)
        self.assertIn("other/omoba-ru-x.tar.gpg", FakeS3.objects, "only our prefix is listed and pruned")
        self.assertIn("prod/db/notes.txt", FakeS3.objects)
        self.assertIn(("HEAD", f"/omoba-ru/prod/db/{self.NAMES[0]}"), FakeS3.requests, "every upload is checked")
        listings = [path for method, path in FakeS3.requests if method == "GET" and "list-type=2" in path]
        self.assertTrue(any("continuation-token=%2B%2F%3D2" in path for path in listings), "paged listing")

    def test_s3_errors_name_the_problem_and_never_the_secret(self):
        target = self.s3(secret="a-wrong-secret")
        with self.assertRaises(rb.BackupError) as failed:
            target.put(str(self.local), self.NAMES[0])
        self.assertIn("403 SignatureDoesNotMatch", str(failed.exception))
        self.assertNotIn("a-wrong-secret", str(failed.exception))
        with self.assertRaises(rb.BackupError) as failed:
            rb.S3Target("http://127.0.0.1:9", "r", "omoba-ru", "", "a", "b").list()
        self.assertIn("did not answer", str(failed.exception))

    def sftp(self, fail=False):
        root = Path(self.work.name) / "remote"
        root.mkdir(exist_ok=True)
        log = Path(self.work.name) / "sftp.log"
        if fail:
            (Path(self.work.name) / "sftp_fail").write_text("")
        bin_dir = fake_bin(self.work.name, {"sftp": SFTP})
        patcher = mock.patch.dict(os.environ, {"PATH": bin_dir + os.pathsep + os.environ.get("PATH", "")})
        patcher.start()
        self.addCleanup(patcher.stop)
        return rb.SftpTarget("backup", "store.example.ru", 2222, "/srv/omoba", "/opt/omoba-ru/backup-keys/ssh_key",
                             "/opt/omoba-ru/backup-keys/known_hosts"), log, root

    def test_an_sftp_account_with_a_pinned_host_key(self):
        target, log, root = self.sftp()
        self.assertEqual(target.list(), {}, "a folder that does not exist yet lists empty")
        self.exercise(target)
        argv = json.loads(log.read_text().splitlines()[0])
        for option in ("BatchMode=yes", "StrictHostKeyChecking=yes", "IdentitiesOnly=yes",
                       "UserKnownHostsFile=/opt/omoba-ru/backup-keys/known_hosts",
                       "IdentityFile=/opt/omoba-ru/backup-keys/ssh_key", "GlobalKnownHostsFile=/dev/null"):
            self.assertIn(option, argv)
        self.assertEqual(argv[:4], ["-F", "none", "-b", "-"])
        self.assertEqual(argv[-1], "backup@store.example.ru")
        self.assertEqual(argv[argv.index("-P") + 1], "2222")
        self.assertFalse(any(name.endswith(".part") for name in os.listdir(root / "srv/omoba")))
        # A truncated copy under the final name is replaced (the next night's upload of a backlog).
        (root / "srv/omoba" / self.NAMES[3]).write_bytes(b"cut")
        target.put(str(self.local), self.NAMES[3])
        self.assertEqual(target.list()[self.NAMES[3]], 3000)
        with self.assertRaises(rb.BackupError):
            target.put(str(self.local), "name with space")

    def test_an_unreachable_sftp_account_fails(self):
        target, _, _ = self.sftp(fail=True)
        with self.assertRaises(rb.BackupError) as failed:
            target.list()
        self.assertIn("Connection refused", str(failed.exception))


SFTP = """import json, os, shutil, sys
work = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
root, log = os.path.join(work, "remote"), os.path.join(work, "sftp.log")
with open(log, "a") as handle:
    handle.write(json.dumps(sys.argv[1:]) + "\\n")
if os.path.exists(os.path.join(work, "sftp_fail")):
    sys.stderr.write("ssh: connect to host store.example.ru port 2222: Connection refused\\n")
    sys.exit(255)
def local(path):
    return os.path.join(root, path.lstrip("/"))
for raw in sys.stdin.read().splitlines():
    ignore, line = raw.startswith("-"), raw.lstrip("-")
    print("sftp> " + line)
    parts = line.split()
    try:
        if parts[0] == "mkdir":
            os.makedirs(local(parts[1]))
        elif parts[0] == "rm":
            os.unlink(local(parts[1]))
        elif parts[0] == "put":
            shutil.copyfile(parts[1], local(parts[2]))
        elif parts[0] == "rename":   # SFTP v3: never replaces an existing file
            if os.path.exists(local(parts[2])):
                raise OSError("Failure")
            os.rename(local(parts[1]), local(parts[2]))
        elif parts[0] == "get":
            shutil.copyfile(local(parts[1]), parts[2])
        elif parts[:2] == ["ls", "-ln"]:
            for name in sorted(os.listdir(local(parts[2]))):
                size = os.path.getsize(os.path.join(local(parts[2]), name))
                print(f"-rw-------    1 1001     1001     {size:>8} Oct 24 01:30 {parts[2]}/{name}")
        else:
            sys.exit("fake sftp: unknown command " + line)
    except OSError as error:
        if not ignore:
            sys.stderr.write(f"{line}: {error}\\n")
            sys.exit(1)
"""


# ---------------------------------------------------------------- settings with real keys

@unittest.skipUnless(HAS_GPG, "needs GnuPG (gpg, gpgconf)")
class ConfigTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.keys = keys()

    def config(self, **values) -> rb.Config:
        base = {"OMOBA_BACKUP_TARGET": f"file://{self.work.name}/off",
                "OMOBA_BACKUP_GPG_RECIPIENT_FILE": self.keys["a"]["public"],
                "OMOBA_BACKUP_GPG_IDENTITY_FILE": self.keys["a"]["secret"],
                "OMOBA_BACKUP_DIR": f"{self.work.name}/backups"}
        return rb.Config(env_file(self.work.name, **{**base, **values}))

    def codes(self, config):
        return [code for code, _ in rb.config_problems(config)]

    def test_a_complete_configuration(self):
        self.assertEqual(rb.config_problems(self.config()), [])
        info = rb.key_info(self.keys["a"]["public"])
        self.assertFalse(info["secret"])
        self.assertEqual(len(info["encrypt"]), 1)
        self.assertNotIn(info["fingerprint"], info["encrypt"], "the ed25519 primary key does not encrypt")
        self.assertEqual(info["sign"], [info["fingerprint"]], "it signs")

    def test_what_is_missing_is_named(self):
        example = (ROOT / "ops/vps/ru/backup.env.example").read_text()
        fresh = rb.Config(write(Path(self.work.name) / "fresh.env", example))
        texts = " ".join(text for _, text in rb.config_problems(fresh))
        self.assertIn("OMOBA_BACKUP_TARGET is not set", texts)
        self.assertIn("encryption key", texts)
        self.assertIn(["config-missing"], [self.codes(rb.Config(f"{self.work.name}/none.env"))])
        cases = {
            "OMOBA_BACKUP_S3_ENDPOINT": dict(OMOBA_BACKUP_TARGET="s3://omoba-ru/prod"),
            "OMOBA_BACKUP_SSH_KEY": dict(OMOBA_BACKUP_TARGET="sftp://b@store.example.ru/srv/omoba",
                                         OMOBA_BACKUP_SSH_KEY=f"{self.work.name}/none"),
            "holds a secret key": dict(OMOBA_BACKUP_GPG_RECIPIENT_FILE=self.keys["a"]["secret"]),
            "is not the secret key of": dict(OMOBA_BACKUP_GPG_IDENTITY_FILE=self.keys["b"]["secret"]),
            "OMOBA_BACKUP_GPG_IDENTITY_FILE": dict(OMOBA_BACKUP_GPG_IDENTITY_FILE=f"{self.work.name}/none"),
            "must be a whole number": dict(OMOBA_BACKUP_KEEP_DAILY="0"),
            "OMOBA_TELEGRAM_CHAT_ID": dict(OMOBA_TELEGRAM_BOT_TOKEN="1:x"),
        }
        for needle, values in cases.items():
            texts = " ".join(text for _, text in rb.config_problems(self.config(**values)))
            self.assertIn(needle, texts, values)
        loose = Path(self.work.name) / "loose.asc"
        shutil.copyfile(self.keys["a"]["secret"], loose)
        loose.chmod(0o644)
        texts = " ".join(text for _, text in rb.config_problems(self.config(OMOBA_BACKUP_GPG_IDENTITY_FILE=loose)))
        self.assertIn("must be 0600", texts)
        # Every backup needs the secret key too: it signs the sets.
        no_key = self.config(OMOBA_BACKUP_GPG_IDENTITY_FILE=f"{self.work.name}/none")
        self.assertEqual(self.codes(no_key), ["identity"])
        real = rb.key_info

        def cannot_sign(path, text=False):
            info = real(path, text)
            return {**info, "sign": []} if info["secret"] else info
        with mock.patch.object(rb, "key_info", side_effect=cannot_sign):
            texts = " ".join(text for _, text in rb.config_problems(self.config()))
        self.assertIn("has no usable signing key", texts)

    def test_the_config_command(self):
        out = io.StringIO()
        code = rb.main(["--env-file", self.config().path, "config", "--json"], stdout=out)
        self.assertEqual((code, json.loads(out.getvalue())["ok"]), (0, True))
        out = io.StringIO()
        broken = self.config(OMOBA_BACKUP_TARGET="CHANGE_ME").path
        self.assertEqual(rb.main(["--env-file", broken, "config", "--json"], stdout=out), 1)
        self.assertIn("OMOBA_BACKUP_TARGET is not set", json.loads(out.getvalue())["problems"][0])


# ---------------------------------------------------------------- health and alerts

class HealthTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        folder = self.work.name
        self.escrowed = write(Path(folder) / "account-api.env", "OMOBA_SOCIAL_SECRET=x\n")
        self.cfg = rb.Config(env_file(folder, OMOBA_BACKUP_DIR=f"{folder}/backups",
                                      OMOBA_BACKUP_TARGET=f"file://{folder}/off",
                                      OMOBA_BACKUP_ESCROW=self.escrowed,
                                      OMOBA_TELEGRAM_BOT_TOKEN="123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                                      OMOBA_TELEGRAM_CHAT_ID="-1001"))
        self.clock, self.telegram, self.journal = Clock(), Telegram(), io.StringIO()
        patcher = mock.patch.object(rb, "config_problems", side_effect=lambda *args, **kwargs: [])
        patcher.start()
        self.addCleanup(patcher.stop)

    def ctx(self):
        return Context(self.cfg, now=self.clock, notify_opener=self.telegram, stderr=self.journal)

    def codes(self, **kwargs):
        return rb.health(self.ctx(), **kwargs)["codes"]

    def state(self, **values):
        ctx = self.ctx()
        ctx.state.update(values)
        ctx.save()

    def test_backup_age_failures_drill_and_escrow(self):
        self.assertEqual(self.codes(), ["backup-missing", "escrow-missing"])
        at = rb.iso(T0 - 25 * 3600)
        self.state(last_success={"at": at, "name": "x"}, first_success_at=at,
                   last_attempt={"started_at": at, "finished_at": at, "ok": True},
                   escrow={"at": at, "files": {self.escrowed: rb.sha256_file(self.escrowed)}})
        self.assertEqual(self.codes(), [])
        warnings = rb.health(self.ctx())["warnings"]
        self.assertTrue(any("no restore drill has run yet" in warning for warning in warnings))
        self.assertTrue(any("local folder" in warning for warning in warnings))
        self.clock.value += 2 * 3600
        self.assertEqual(self.codes(), ["backup-stale"], "more than 26 h")
        self.state(last_attempt={"started_at": rb.iso(T0), "finished_at": rb.iso(T0), "ok": False,
                                 "error": "sftp failed"})
        self.assertEqual(self.codes(), ["backup-stale", "backup-failed"])
        self.state(last_attempt={"started_at": rb.iso(T0 - 7 * 3600)})
        self.assertIn("backup-interrupted", self.codes())
        # Drill: failed, then stale, then never in 35 days.
        self.state(last_success={"at": rb.iso(self.clock.value), "name": "y"},
                   last_attempt={"started_at": rb.iso(T0), "finished_at": rb.iso(T0), "ok": True},
                   last_drill={"ok": False, "finished_at": rb.iso(T0), "errors": ["rows differ"]})
        self.assertEqual(self.codes(), ["drill-failed"])
        done = rb.iso(T0)
        self.state(last_drill={"ok": True, "finished_at": done}, last_drill_success={"finished_at": done})
        self.assertEqual(self.codes(), [])
        self.clock.value = T0 + 36 * DAY
        self.state(last_success={"at": rb.iso(self.clock.value), "name": "z"})
        self.assertEqual(self.codes(), ["drill-stale"])
        self.state(last_drill={}, last_drill_success={})
        self.assertEqual(self.codes(), ["drill-missing"])
        # Escrow goes stale when a secret file changes.
        Path(self.escrowed).write_text("OMOBA_SOCIAL_SECRET=y\n")
        report = rb.health(self.ctx())
        self.assertIn("escrow-stale", report["codes"])
        self.assertIn(self.escrowed, " ".join(report["problems"]))

    def test_the_off_host_listing(self):
        off = Path(self.work.name) / "off"
        off.mkdir()
        report = rb.health(self.ctx(), remote=True)
        self.assertIn("off-host-empty", report["codes"])
        (off / f"omoba-ru-{time.strftime(rb.STAMP, time.gmtime(T0 - 3600))}-nightly.tar.gpg").write_bytes(b"x")
        report = rb.health(self.ctx(), remote=True)
        self.assertNotIn("off-host-empty", report["codes"])
        self.assertEqual(report["facts"]["off_host_sets"], 1)

    def test_alerts_are_grouped_repeated_every_6_hours_and_resolved_once(self):
        self.state(last_success={"at": rb.iso(T0 - 30 * 3600)}, first_success_at=rb.iso(T0 - 30 * 3600),
                   escrow={"at": "x", "files": {self.escrowed: rb.sha256_file(self.escrowed)}})
        argv = ["--env-file", self.cfg.path, "check", "--alert"]
        with contextlib.redirect_stderr(io.StringIO()):
            with mock.patch.object(rb, "Context", side_effect=lambda cfg: Context(
                    cfg, now=self.clock, notify_opener=self.telegram, stderr=self.journal)):
                self.assertEqual(rb.main(argv, stdout=io.StringIO()), 1)
                self.assertEqual(rb.main(argv, stdout=io.StringIO()), 1)
                self.assertEqual(len(self.telegram.sent), 1, "not again within 6 h")
                url, text = self.telegram.sent[0]
                self.assertTrue(url.startswith("https://api.telegram.org/bot123456:"))
                self.assertIn("the last successful backup is 30 h old", text)
                self.clock.value += 6 * 3600 + 1
                rb.main(argv, stdout=io.StringIO())
                self.assertEqual(len(self.telegram.sent), 2)
                self.state(last_success={"at": rb.iso(self.clock.value)})
                self.assertEqual(rb.main(argv, stdout=io.StringIO()), 0)
                self.assertEqual(self.telegram.sent[-1][1].split(": ", 1)[1], "all checks pass again")
                self.assertEqual(rb.main(argv, stdout=io.StringIO()), 0)
                self.assertEqual(len(self.telegram.sent), 3, "the recovery message is sent once")
        journal = self.journal.getvalue()
        self.assertIn("<3>the last successful backup is 30 h old", journal)
        self.assertNotIn("AAAAAAAAAAAA", journal)

    def test_an_undelivered_alert_is_logged_without_the_token(self):
        self.telegram.fail = urllib.error.URLError("bot123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA unreachable")
        self.assertFalse(self.ctx().alert("x", "backup failed"))
        journal = self.journal.getvalue()
        self.assertIn("<4>the Telegram alert was not delivered", journal)
        self.assertNotIn("AAAAAAAAAAAA", journal)


# ---------------------------------------------------------------- whole runs with fake PostgreSQL tools

@unittest.skipUnless(HAS_GPG, "needs GnuPG (gpg, gpgconf)")
class BackupRunTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        folder = Path(self.work.name)
        self.pg = folder / "pg"
        self.pg.mkdir()
        (self.pg / "data.txt").write_text(DATA)
        (self.pg / "versions.json").write_text(json.dumps(VERSIONS))
        bin_dir = fake_bin(folder, FAKE_PG)
        patcher = mock.patch.dict(os.environ, {"PATH": bin_dir + os.pathsep + os.environ.get("PATH", "")})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.state_root = folder / "state"
        write(self.state_root / "matches/archive/m1/manifest.json", '{"allocation": 1}')
        write(self.state_root / "matches/archive/m1/outbox/r1.json", '{"result": 1}')
        write(self.state_root / "matches/.coordinator.lock", "")
        write(self.state_root / "lobby-outbox/r2.json", '{"result": 2}')
        self.off = folder / "off"
        self.keys = keys()
        self.clock, self.telegram, self.journal = Clock(), Telegram(), io.StringIO()
        self.cfg = self.config()

    def config(self, **values):
        folder = self.work.name
        base = dict(OMOBA_BACKUP_TARGET=f"file://{self.off}", OMOBA_BACKUP_DIR=f"{folder}/backups",
                    OMOBA_BACKUP_GPG_RECIPIENT_FILE=self.keys["a"]["public"],
                    OMOBA_BACKUP_GPG_IDENTITY_FILE=self.keys["a"]["secret"], OMOBA_BACKUP_DATABASE="omoba_ru",
                    OMOBA_BACKUP_STATE_DIRS=f"{self.state_root}/matches:{self.state_root}/lobby-outbox:/nonexistent",
                    OMOBA_BACKUP_RUN_AS="", OMOBA_TELEGRAM_BOT_TOKEN="123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                    OMOBA_TELEGRAM_CHAT_ID="-1001")
        return rb.Config(env_file(folder, **{**base, **values}))

    def ctx(self):
        return Context(self.cfg, now=self.clock, notify_opener=self.telegram, stderr=self.journal)

    def backup(self, **kwargs):
        return rb.run_backup(self.ctx(), **kwargs)

    def test_a_set_is_sealed_uploaded_and_restorable(self):
        summary = self.backup(reason="nightly")
        name = summary["name"]
        self.assertEqual(name, f"omoba-ru-{time.strftime(rb.STAMP, time.gmtime(T0))}-nightly.tar.gpg")
        self.assertEqual(summary["uploaded"], [name])
        self.assertEqual((summary["tables"], summary["rows"], summary["schema_versions"]), (4, 12, VERSIONS))
        self.assertEqual(summary["state_files"], 4)
        self.assertEqual(summary["state_missing"], ["/nonexistent"])
        local = Path(self.cfg.sets_dir) / name
        self.assertEqual((self.off / name).read_bytes(), local.read_bytes())
        self.assertEqual(os.stat(local).st_mode & 0o777, 0o600)
        self.assertEqual(os.listdir(self.cfg.work_dir), [], "no plaintext stays behind")
        self.assertNotIn(b"PGDMP", local.read_bytes(), "encrypted")
        self.assertIn("pg_dump -Fc -w -d omoba_ru", (self.pg / "log").read_text())
        state = self.ctx().state
        self.assertEqual(state["last_success"]["name"], name)
        self.assertTrue(state["last_attempt"]["ok"])
        # The secret key opens it; the manifest describes what is inside.
        out = Path(self.work.name) / "fetched"
        result = rb.run_fetch(self.ctx(), "latest", str(out), "remote")
        self.assertEqual(result["set"], name)
        manifest = json.loads((out / "manifest.json").read_text())
        self.assertEqual(manifest["database"]["schema_versions"], VERSIONS)
        self.assertEqual(manifest["database"]["tables"]["public.career_profiles"]["rows"], 2)
        self.assertEqual(manifest["state"]["files"], 4)
        with tarfile.open(out / "state.tar.gz") as archive:
            names = archive.getnames()
        self.assertIn(str(self.state_root / "lobby-outbox/r2.json").lstrip("/"), names)
        self.assertIn(str(self.state_root / "matches/archive/m1/outbox/r1.json").lstrip("/"), names)
        # The other key cannot.
        other = self.config(OMOBA_BACKUP_GPG_IDENTITY_FILE=self.keys["b"]["secret"])
        with self.assertRaises(rb.BackupError) as failed:
            rb.run_fetch(Context(other, now=self.clock), "latest", str(Path(self.work.name) / "f2"), "local")
        self.assertIn("could not be decrypted", str(failed.exception))
        self.assertEqual(os.listdir(Path(self.work.name) / "f2"), [])

    def test_a_set_the_backup_key_did_not_sign_is_never_restored(self):
        real = self.backup(reason="nightly")["name"]
        unpacked = Path(self.work.name) / "unpacked"
        unpacked.mkdir()
        manifest = rb.unseal(self.cfg, os.path.join(self.cfg.sets_dir, real), str(unpacked))
        # Anyone with the public key (or write access to the storage) can encrypt a set.
        unsigned = "omoba-ru-20300101T000000Z-manual.tar.gpg"
        with rb.gpg_home() as home:
            process, errors = rb.encrypt_to(self.cfg.recipient, str(self.off / unsigned), home)
            with tarfile.open(fileobj=process.stdin, mode="w|") as archive:
                for member in rb.SET_MEMBERS:
                    archive.add(str(unpacked / member), arcname=member)
            process.stdin.close()
            rb.finish(process, errors, "gpg")
        # Or sign one with another key.
        other = "omoba-ru-20300102T000000Z-manual.tar.gpg"
        rb.seal(self.config(OMOBA_BACKUP_GPG_IDENTITY_FILE=self.keys["b"]["secret"]), str(unpacked), manifest,
                str(self.off / other))
        for name in (unsigned, other):
            out = Path(self.work.name) / f"restore-{name}"
            with self.assertRaises(rb.BackupError) as refused:
                rb.run_fetch(self.ctx(), name, str(out), "remote")
            self.assertIn(f"{name} is not signed by the backup key", str(refused.exception))
            self.assertEqual(sorted(os.listdir(out)), [], "nothing of it is left to restore")
        with self.assertRaises(rb.BackupError) as refused:
            rb.run_drill(self.ctx())   # the newest set off-host is the forged one
        self.assertIn("is not signed by the backup key", str(refused.exception))
        self.assertNotIn("psql", (self.pg / "log").read_text() if (self.pg / "log").exists() else "")
        self.assertIn("drill-failed", self.ctx().state["alerts"])
        self.assertEqual(rb.run_fetch(self.ctx(), real, str(Path(self.work.name) / "good"), "remote")["set"], real)

    def test_retention_off_host_and_on_the_host_over_many_nights(self):
        self.cfg = self.config(OMOBA_BACKUP_KEEP_DAILY="3", OMOBA_BACKUP_KEEP_WEEKLY="2")
        made = []
        for night in range(20):   # Monday 2026-09-21 to Saturday 2026-10-10
            self.clock.value = T0 + night * DAY
            made.append(self.backup(reason="nightly")["name"])
        days = {name[9:17] for name in os.listdir(self.off)}
        self.assertEqual(days, {"20261008", "20261009", "20261010", "20261004"},
                         "the last three nights and the newest set of the week before")
        self.assertEqual(set(os.listdir(self.off)), rb.retained(made, 3, 2))
        self.assertEqual(sorted(os.listdir(self.cfg.sets_dir)), sorted(made)[-2:], "two stay on the host")

    def test_a_failed_upload_alerts_keeps_the_set_and_goes_up_the_next_night(self):
        first = self.backup(reason="nightly")["name"]
        self.clock.value += DAY
        broken = mock.patch.object(rb.FileTarget, "put", side_effect=rb.BackupError("storage refused the upload"))
        with broken, self.assertRaises(rb.BackupError):
            self.backup(reason="nightly")
        second = rb.newest(rb.local_sets(self.cfg))
        self.assertNotEqual(second, first)
        self.assertNotIn(second, os.listdir(self.off))
        self.assertEqual(len(self.telegram.sent), 1)
        self.assertIn("storage refused the upload", self.telegram.sent[0][1])
        self.assertIn("backup-failed", rb.health(self.ctx())["codes"])
        self.clock.value += DAY
        third = self.backup(reason="nightly")
        self.assertEqual(third["uploaded"], [second, third["name"]], "the backlog goes up first")
        self.assertNotIn("backup-failed", self.ctx().state.get("alerts", {}))

    def test_unconfigured_backups_fail_loudly(self):
        self.cfg = self.config(OMOBA_BACKUP_TARGET="CHANGE_ME")
        with self.assertRaises(rb.BackupError) as failed:
            self.backup(reason="nightly")
        self.assertIn("backups are not configured: OMOBA_BACKUP_TARGET is not set", str(failed.exception))
        self.assertIn("<3>backup omoba-ru-", self.journal.getvalue())
        self.assertFalse(self.ctx().state["last_attempt"]["ok"])
        self.assertEqual(len(self.telegram.sent), 1)

    def test_pre_migration_rollback_dumps_and_local_only(self):
        for index in range(4):
            self.clock.value = T0 + index * 3600
            summary = self.backup(reason="pre-migration", release=f"0.45.{index}-ru-abc1234")
            self.assertTrue(os.path.isfile(summary["rollback_dump"]))
            self.assertEqual(Path(summary["rollback_dump"]).read_bytes()[:5], b"PGDMP")
            self.assertEqual(os.stat(summary["rollback_dump"]).st_mode & 0o777, 0o600)
        dumps = sorted(name for name in os.listdir(self.cfg.backup_dir) if name.endswith(".dump"))
        self.assertEqual(dumps, [f"omoba_ru-before-0.45.{index}-ru-abc1234-"
                                 f"{time.strftime(rb.STAMP, time.gmtime(T0 + index * 3600))}.dump"
                                 for index in (1, 2, 3)])
        # Before launch, without storage: the rollback dump only, no set, no "last success".
        self.cfg = self.config(OMOBA_BACKUP_TARGET="CHANGE_ME", OMOBA_BACKUP_GPG_RECIPIENT_FILE="/none")
        success = self.ctx().state["last_success"]
        self.clock.value += 3600
        local = self.backup(reason="pre-migration", release="0.46.0-ru-abc1234", local_only=True)
        self.assertEqual((local["mode"], local["name"]), ("local-only", None))
        self.assertTrue(os.path.isfile(local["rollback_dump"]))
        self.assertEqual(self.ctx().state["last_success"], success)
        out = io.StringIO()
        context = self.ctx()
        with mock.patch.object(rb, "Context", side_effect=lambda cfg: context):
            code = rb.main(["--env-file", self.cfg.path, "backup", "--reason", "pre-migration", "--release",
                            "0.46.1-ru-abc1234", "--local-only"], stdout=out)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out.getvalue().splitlines()[-1])["mode"], "local-only", "ru_host reads it")

    def test_a_migration_during_the_dump_or_a_failing_dump_is_an_error(self):
        (self.pg / "versions.json").write_text(json.dumps({**VERSIONS, "portal": [1, 2, 3, 4, 5, 6, 7, 8]}))
        with self.assertRaises(rb.BackupError) as failed:
            self.backup(reason="nightly")
        self.assertIn("a migration ran during the backup", str(failed.exception))
        (self.pg / "versions.json").write_text(json.dumps(VERSIONS))
        (self.pg / "fail_dump").write_text("")
        with self.assertRaises(rb.BackupError) as failed:
            self.backup(reason="nightly")
        self.assertIn("pg_dump omoba_ru failed: pg_dump: error: connection to server failed", str(failed.exception))
        self.assertEqual(rb.local_sets(self.cfg), {})

    def test_one_run_at_a_time(self):
        with rb.locked(self.cfg), mock.patch.object(rb, "LOCK_WAIT_SECONDS", 0):
            self.assertTrue(rb.lock_busy(self.cfg))
            with self.assertRaises(rb.BackupError) as failed:
                self.backup(reason="nightly")
        self.assertIn("still running", str(failed.exception))
        self.assertFalse(rb.lock_busy(self.cfg))

    def test_escrow_records_what_it_sealed(self):
        secrets_dir = Path(self.work.name) / "opt"
        api = write(secrets_dir / "account-api.env", "OMOBA_SOCIAL_SECRET=" + "c" * 64 + "\n")
        key = write(secrets_dir / "backup-keys/backup-secret.asc", "key\n")
        self.cfg = self.config(OMOBA_BACKUP_ESCROW=f"{api}:{secrets_dir}/backup-keys:{secrets_dir}/missing.env")
        out = io.BytesIO()
        summary = rb.run_escrow(self.ctx(), Path(self.keys["b"]["public"]).read_text(), None, out, record=True)
        self.assertEqual(summary["files"], [api, key])
        ciphertext = out.getvalue()
        self.assertTrue(ciphertext.startswith(b"-----BEGIN PGP MESSAGE-----"))
        self.assertNotIn(b"c" * 64, ciphertext)
        with rb.gpg_home(agent=True) as home:
            subprocess.run(rb.gpg(home, "--import", self.keys["b"]["secret"]), check=True, capture_output=True)
            plain = subprocess.run(rb.gpg(home, "--decrypt"), input=ciphertext, check=True, capture_output=True).stdout
        with tarfile.open(fileobj=io.BytesIO(plain)) as archive:
            self.assertEqual(sorted(archive.getnames()), sorted(["ESCROW.txt", api.lstrip("/"), key.lstrip("/")]))
            self.assertIn(b"c" * 64, archive.extractfile(api.lstrip("/")).read())
        self.assertEqual(rb.escrow_problems(self.ctx()), [])
        write(secrets_dir / "backup-keys/ssh_key", "new\n")
        self.assertEqual([code for code, _ in rb.escrow_problems(self.ctx())], ["escrow-stale"])
        with self.assertRaises(rb.BackupError):
            rb.run_escrow(self.ctx(), Path(self.keys["b"]["secret"]).read_text(), None, io.BytesIO(), record=True)

    def test_escrow_of_local_files_from_the_command_line(self):
        keystore = write(Path(self.work.name) / "android-release.jks", "keystore")
        out = io.BytesIO()
        stdout = mock.Mock(buffer=out)
        with contextlib.redirect_stderr(io.StringIO()) as err:
            code = rb.main(["escrow", "--recipient-file", "-", "--file", keystore],
                           stdin=io.StringIO(Path(self.keys["a"]["public"]).read_text()), stdout=stdout)
        self.assertEqual(code, 0)
        self.assertTrue(out.getvalue().startswith(b"-----BEGIN PGP MESSAGE-----"))
        self.assertIn(keystore, json.loads(err.getvalue())["files"])


# ---------------------------------------------------------------- PostgreSQL round trip

PG_URL = os.environ.get("OMOBA_BACKUP_TEST_PG", "")
E2E_DB = "omoba_backup_e2e"
SEED = """
INSERT INTO career_profiles(profile_id, nickname, rating, rated_matches, matches_played, wins, losses)
VALUES (repeat('a', 64), 'Ника', 1234, 3, 5, 3, 2), (repeat('b', 64), 'Bob', 987, 2, 4, 1, 3);
INSERT INTO career_matches(result_id, server_epoch, match_id, owner_id, lease_until, allocation, checkpoint)
VALUES ('r1', '1', '1', 'lobby', now(), '{}', '{}');
INSERT INTO career_participants(result_id, seat, player_id, profile_id)
VALUES ('r1', 0, '1', repeat('a', 64)), ('r1', 1, '2', repeat('b', 64));
UPDATE career_matches SET status = 'settled', intent = '{"x": 1}', result = '{"winner": "green"}', settled_at = now()
 WHERE result_id = 'r1';
INSERT INTO portal.recovery_codes(code_hash, profile_id, created_at) VALUES ('hash-of-a-code', repeat('a', 64), 1);
INSERT INTO portal.social_identities(provider, subject_hash, method, profile_id, created_at, last_login_at, linked_at)
VALUES ('vk', repeat('c', 64), 'vkid', repeat('a', 64), 1, 2, 2);
"""


def pg_env() -> dict:
    parts = urllib.parse.urlsplit(PG_URL)
    env = {"PGHOST": parts.hostname or "127.0.0.1", "PGPORT": str(parts.port or 5432),
           "PGUSER": urllib.parse.unquote(parts.username or "postgres")}
    if parts.password:
        env["PGPASSWORD"] = urllib.parse.unquote(parts.password)
    bin_dir = os.environ.get("OMOBA_BACKUP_TEST_PG_BIN")
    env["PATH"] = (bin_dir + os.pathsep if bin_dir else "") + os.environ.get("PATH", "")
    return env


@unittest.skipUnless(PG_URL and HAS_GPG, "set OMOBA_BACKUP_TEST_PG to a disposable PostgreSQL server; needs GnuPG")
class PostgresRoundTripTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.patcher = mock.patch.dict(os.environ, pg_env())
        cls.patcher.start()
        cls.sql("postgres", f"DROP DATABASE IF EXISTS {E2E_DB}_restore_drill WITH (FORCE);"
                            f"DROP DATABASE IF EXISTS {E2E_DB} WITH (FORCE);")
        cls.sql("postgres", f"CREATE DATABASE {E2E_DB};")
        # As CareerStore and `omoba-account-api migrate` apply them: 001 has no version row of its own.
        career = sorted((ROOT / "career-store/migrations/postgres").glob("0*.sql"))
        portal = sorted((ROOT / "account-api/migrations").glob("0*.sql"))
        migrations = ["BEGIN;", "CREATE TABLE career_schema_version (version INTEGER PRIMARY KEY);",
                      career[0].read_text(), "INSERT INTO career_schema_version VALUES (1);"]
        migrations += [path.read_text() for path in career[1:] + portal]
        cls.sql(E2E_DB, "\n".join(migrations + [SEED, "COMMIT;"]))
        cls.versions = {"career": [int(path.name[:3]) for path in career],
                        "portal": [int(path.name[:3]) for path in portal]}

    @classmethod
    def tearDownClass(cls):
        cls.sql("postgres", f"DROP DATABASE IF EXISTS {E2E_DB} WITH (FORCE);")
        cls.patcher.stop()

    @staticmethod
    def sql(database, text) -> str:
        return subprocess.run(["psql", "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", "-d", database], input=text,
                              capture_output=True, text=True, check=True).stdout

    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        folder = Path(self.work.name)
        write(folder / "state/matches/archive/m1/manifest.json", '{"allocation": 1}')
        write(folder / "state/lobby-outbox/r2.json", '{"result": 2}')
        self.off = folder / "off"
        self.keys = keys()
        self.cfg = rb.Config(env_file(
            folder, OMOBA_BACKUP_TARGET=f"file://{self.off}", OMOBA_BACKUP_DIR=f"{folder}/backups",
            OMOBA_BACKUP_GPG_RECIPIENT_FILE=self.keys["a"]["public"],
            OMOBA_BACKUP_GPG_IDENTITY_FILE=self.keys["a"]["secret"], OMOBA_BACKUP_DATABASE=E2E_DB,
            OMOBA_BACKUP_STATE_DIRS=f"{folder}/state/matches:{folder}/state/lobby-outbox", OMOBA_BACKUP_RUN_AS=""))
        self.journal = io.StringIO()

    def ctx(self):
        return Context(self.cfg, stderr=self.journal)

    def counts(self, database=E2E_DB):
        return self.sql(database, "SELECT (SELECT count(*) FROM career_profiles), (SELECT sum(rating) FROM "
                                  "career_profiles), (SELECT count(*) FROM career_participants), (SELECT count(*) "
                                  "FROM portal.recovery_codes), (SELECT count(*) FROM portal.social_identities);")

    def reseal(self, source: str, name: str, change) -> str:
        """A copy of the off-host set `source` under `name` whose manifest `change` edits."""
        unpacked = Path(self.work.name) / f"unpacked-{name}"
        unpacked.mkdir()
        manifest = rb.unseal(self.cfg, str(self.off / source), str(unpacked))
        change(manifest)
        rb.seal(self.cfg, str(unpacked), manifest, str(self.off / name))
        return name

    def test_backup_drill_fetch_and_drills_that_must_fail(self):
        before = self.counts()
        self.assertEqual(before.strip(), "2|2221|2|1|1")
        summary = rb.run_backup(self.ctx(), reason="nightly")
        self.assertEqual(summary["schema_versions"], self.versions)
        self.assertGreater(summary["tables"], 25)
        drill = rb.run_drill(self.ctx())
        self.assertTrue(drill["ok"], drill)
        self.assertEqual((drill["errors"], drill["warnings"]), ([], []))
        self.assertEqual(drill["tables"], summary["tables"])
        self.assertEqual(self.sql("postgres", f"SELECT count(*) FROM pg_database WHERE datname LIKE '{E2E_DB}%';"),
                         "1\n", "the scratch database is dropped")
        self.assertEqual(self.counts(), before, "the live database is untouched")
        self.assertEqual(rb.health(self.ctx())["facts"]["last_drill_success"], drill["finished_at"])
        out = Path(self.work.name) / "restore"
        fetched = rb.run_fetch(self.ctx(), "latest", str(out), "remote")
        self.assertEqual(fetched["schema_versions"], summary["schema_versions"])
        listing = subprocess.run(["pg_restore", "-l", str(out / "database.dump")], capture_output=True, text=True,
                                 check=True).stdout
        self.assertIn("TABLE DATA portal recovery_codes", listing)

        # A rating that differs after the restore, a lost row, a missing version table.
        def rating(manifest):
            manifest["database"]["tables"]["public.career_profiles"]["digest"] = "f" * 32
        failed = rb.run_drill(self.ctx(), self.reseal(summary["name"], "omoba-ru-20300101T000000Z-manual.tar.gpg", rating))
        self.assertFalse(failed["ok"])
        self.assertEqual(failed["errors"], ["public.career_profiles: the restored rows differ from the backup"])

        def lost(manifest):
            manifest["database"]["tables"]["portal.recovery_codes"]["rows"] = 2
        failed = rb.run_drill(self.ctx(), self.reseal(summary["name"], "omoba-ru-20300102T000000Z-manual.tar.gpg", lost))
        self.assertEqual(failed["errors"], ["portal.recovery_codes: 1 rows restored, 2 in the backup"])
        self.assertIn("drill-failed", rb.health(self.ctx())["codes"])
        self.assertIn("<3>the restore drill failed: portal.recovery_codes", self.journal.getvalue())

        # A damaged off-host set.
        newest = rb.newest(os.listdir(self.off))
        data = bytearray((self.off / newest).read_bytes())
        data[len(data) // 2] ^= 0xFF
        (self.off / newest).write_bytes(bytes(data))
        with self.assertRaises(rb.BackupError) as broken:
            rb.run_drill(self.ctx())
        self.assertIn("decrypt", str(broken.exception))
        self.assertFalse(self.ctx().state["last_drill"]["ok"])
        self.assertEqual(self.sql("postgres", f"SELECT count(*) FROM pg_database WHERE datname LIKE '{E2E_DB}%';"),
                         "1\n")
        self.assertEqual(self.counts(), before)


if __name__ == "__main__":
    unittest.main()
