# Player handles

Since 0.20.0-rc.2 a public player address is `Nickname#1234`. The existing
`career_profiles.nickname` stores the full address. The immutable random
`profile_id` is internal: keys, friendships, match membership and rewards keep
using it. Renaming does not replace an account or rewrite historical match names.

## Rules

- Base: 1–20 Unicode codepoints, letters/numbers, spaces, `_`, `-`, `.`;
  surrounding spaces are trimmed. Suffix: exactly four ASCII digits, including 0000.
- Both base and suffix can be edited in the native profile or web settings.
  The complete address must be unique, ignoring Unicode case. Conflicts leave
  the previous name intact and ask for another tag.
- New default `Player` accounts receive a random adjective/animal combination
  from a bundled name pool and a random four-digit tag. Authentication of an
  existing key preserves the current address. Existing custom names keep their
  base during migration. Bare-name legacy renames preserve the current tag.
- A base has 10,000 possible tags. Names/tags are cosmetic addresses, never
  credentials or proof of identity. Old addresses are not reserved after rename.

## Invitations and privacy

Native clients send authenticated, replay-protected `LookupPlayer` actions;
web clients use authenticated `/v1/players?query=Nickname%231234`. Exact lookup
returns only `{profile_id,nickname}`, including private/non-discoverable accounts.
It reveals no rating, history or friends. Partial web discovery remains opt-in.
The UI shows the resolved player before invitation; the request uses their stable
ID, so a later rename cannot redirect it to a new owner of the old address.

## Native editor

Profile and friend field text is updated in place. Typing no longer despawns the
modal tree, which previously disrupted pointer focus, scroll and phone IME.
Fields have fixed height and bounded input; relevant state changes render after
leaving editing, with layout changes still triggering a layout rebuild.

## Migration and rollout

Apply owner migration 002 before starting the matching v2 runtime. It locks
profiles, assigns unique tags to existing names, installs generation/rename
triggers and a unique index, then records schema versions `[1,2]` atomically.
PostgreSQL must provide ICU collation `"und-x-icu"`; no extension or new library
is installed. Allow a maintenance window for the profile table lock on large DBs.
Runtime roles perform no migration. Old binaries requiring core v1 must be upgraded;
no production migration or deployment is performed by the source change.
