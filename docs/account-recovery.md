# Account recovery: players and support (LR-18)

An OMOBA account is the device key of one installation (`career_identity.json` in the
settings folder, [ADR-001](architecture/decisions.md#adr-001-the-installation-device-key-is-the-account)).
The server keeps the profile (rating, level, history, friends); the key opens it.
Uninstalling the game, clearing its data or a new phone creates a new key, and the
first connection makes a new profile. Android keeps no cloud backup of the key
([ADR-026](architecture/decisions.md#adr-026-android-keeps-allowbackupfalse-progress-survives-a-reinstall-through-vk-id-or-recovery-codes)).

What brings a profile back, and nothing else:

| Proof | Set up in the game | Set up on the portal |
| --- | --- | --- |
| A linked VK ID (VK, OK or Mail.ru), in regions that sign in with VK ID | Profile → «Войти через VK ID», or Protect progress → «Привязать VK ID» | Sign in with VK ID, then confirm the link in the game |
| An unused recovery code (eight per set, each works once) | Profile → «Защитить прогресс» → «Создать коды восстановления» | Devices → recovery codes |
| Another device that still plays the account | — | Devices → Link device |

The game asks for the first two after the first completed match and again after
the first ranked match (the Home card, until the profile is protected or the player
declined three times), and Profile shows which of them the account has.

`<RU portal>` below is the RU profile's portal in [scripts/markets.py](../scripts/markets.py)
(`python3 ops/vps/ru/ru_host.py names` prints it).

## Self-service

**With VK ID (progress follows the VK-linked account).**

1. Install the game, accept the documents, open Profile → «Войти через VK ID».
2. Sign in in the browser with the same VK ID (VK, OK or Mail.ru) that was linked.
3. The game shows «Использовать аккаунт: <ник>». Press «Использовать этот аккаунт после
   перезапуска» and restart. The game now plays the old profile: same `profile_id`,
   rating, level and history. The empty profile the fresh install made is kept apart,
   never merged, and is deleted after 30 days if it never played (LR-16).

**With a recovery code.**

1. Profile → «Привязать или восстановить аккаунт» → «Ввести код восстановления».
2. Type or paste one saved code (spaces and dashes are ignored) → «Восстановить с
   помощью этого кода» → confirm the account → restart.
3. The code is spent, and every website session of the account is signed out. Make a
   new set later (Profile → «Защитить прогресс»).

**With another device.** On the portal (signed in through that device or VK ID):
Devices → Link device, enter the new device's eight-character code and compare the
fingerprints.

**Afterwards.** On the portal's Devices page, revoke the lost device.

## Support procedure (VK-verified)

Support never moves, merges, links or hands out a profile because of a nickname, a
player tag, a screenshot, a payment receipt or anyone's word. The only proofs are the
ones the system checks itself: a VK ID sign-in linked to the profile, a recovery code
or an active device. Support routes the player to the self-service path that uses one
of them and finds out, read-only, why it fails.

1. **Ask what the player still has**: a VK ID they linked, saved codes, another device
   that still plays.
2. **VK verification.** Ask the player to open `<RU portal>` and press «Войти через VK
   ID» with the VK, OK or Mail.ru account they believe they linked. The VK sign-in
   is the verification; support checks nothing else.
   - **The portal shows their profile** (nickname, history): the VK ID is linked and
     the player controls it. Walk them through *With VK ID* above. If the switch fails
     because the account already has 16 active devices (`device_limit`), they revoke
     old devices on the portal's Devices page first. Signed in there, they can also make new recovery codes.
   - **The portal asks them to install the game and link it** (the VK account is not
     linked to any profile): this VK ID never protected a profile, or protected another
     one. Nothing proves the old profile is theirs, so support cannot bring it back.
     Say so plainly, and help them protect the new profile.
3. **No VK ID, no code, no other device**: the profile cannot be recovered. This is by
   design (ADR-001); support does not make exceptions.
4. **Never ask for** recovery codes, the `career_identity.json` file, VK passwords or
   VK confirmation codes. A player who sends a code is told to make a new set at once.

**Operator diagnostics (owner or operator, read-only).** When a self-service path fails
for a reason the player cannot see, an operator looks the profile up on the RU host
(`psql` as the owner role) from the player tag the player gives (`Nickname#1234`,
[player handles](player-handles.md)) and reads:

```sql
SELECT profile_id FROM career_profiles WHERE lower(nickname) = lower('Nickname#1234');
SELECT count(*) FILTER (WHERE revoked_at IS NULL) AS active_keys FROM career_keys WHERE profile_id = :'profile';
SELECT provider, method, to_timestamp(linked_at) FROM portal.social_identities WHERE profile_id = :'profile';
SELECT count(*) AS unused_codes FROM portal.recovery_codes WHERE profile_id = :'profile' AND consumed_at IS NULL;
SELECT event, created_at FROM portal.audit_events WHERE profile_id = :'profile' ORDER BY created_at DESC LIMIT 20;
```

The answers say which path can work (a linked VK ID, unused codes, active devices) and
what happened recently (`account_recovered`, `device_linked`, `device_revoked`,
`recovery_codes_rotated`, `recovery_codes_rotated_in_game`, `social_linked`,
`social_unlinked`; audit events are kept 30 days). They never change anything.
Writing `career_keys`, `portal.social_identities` or `portal.recovery_codes` by hand is
not allowed: a hand-made link hands the profile to whoever asked. A suspected bug in
these tables goes to the owner with the request id from the error.

Contract and limits: [identity and accounts](architecture/identity-and-accounts.md#one-account-on-many-devices).
