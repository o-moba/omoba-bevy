# Durable queue-dodge delivery checkpoint

The real backend, port, admission, typed journal and career store pass all60
native tests, including all7 real PostgreSQL cases, with0 failures/ignores.
The three new PG worker cases prove offline persistence/restart/exact replay,
a full reply channel without lost commit acknowledgement, and corrupt-journal
queue refusal while existing authenticated access refresh remains available.
Sixteen journal units and five submission/acknowledgement units are included.
[Complete log](all-backend-final.log), [targeted log](backend-v5.log),
[source hashes and limits](summary.json), [compiler inputs](backend-compile.json).

The renderer-free crate references actual production modules; the engine-coupled
transport contributes only its verbatim MAX_ENDPOINTS constant. It allows dead
code for omitted engine callers. Strict Clippy uses the existing workspace
allowances. MatchService/engine integration is excluded: its new storage-failure
ready test and extended stable-ID recovery test require fresh standard CI.

Producer snapshots retain stable event identities until the exact full payload
is SQL-committed. The separate bounded, exclusively locked journal fsyncs files
and directories, retries without turning receipts into MatchResult history, and
participates in shutdown flush. Corrupt producer snapshots close new search and
are preserved; storage failure pauses ready checks without blaming all players.
The pre-existing15-second unanimous ready protocol is unchanged.

The exact two source DML grants and SQL were exercised under a temporary NOLOGIN
role in one rolled-back transaction on task-owned local PostgreSQL18.6. Event
UPDATE/DELETE remain denied; role/profile/event leftovers are all zero.
[ACL proof](grants-proof.json). No production role or credential was changed.

Scripts are archived diagnostic drivers for this workspace/cache, not portable
release tools; their prior shared/store compiler input comes from the adjacent
queue-discipline evidence and retained .agent artifacts. With sufficient disk,
the ordinary complete server verification is:

```sh
cargo test --locked -p server -- --include-ignored --test-threads=1
```

Supply only a disposable migrated test database through OMOBA_TEST_DATABASE_URL.
Do not use production. Native artifacts remain in the ignored .agent directory;
no Bevy build, cache deletion, new dependency, or third Cargo cache occurred.

This closes only the delivery component. Shared queue reads/final-start fences,
ready ownership/deadlines, legacy penalty floors and two real lobbies remain.
Add typed-journal monitoring and verify schema11 recovery, including receipt
replay against an explicitly chosen restore target, before release acceptance.
C2/C4 host-tool review patches remain unapplied pending existing owner questions.

Archived test logs omit only trailing blank lines; raw originals remain in .agent.
