# Low-population matchmaking for OMOBA

## Launch policy

| Search outcome | When it starts | Career result |
| --- | --- | --- |
| Ranked | A compatible 5v5 human roster accepts the ready check | Team Elo and ranked XP for an eligible completion |
| Casual PvP | A full human roster is available, or an incomplete roster has waited 30 seconds and fills vacancies with bots | Match history and 50/25 win/loss XP under `public-casual-v1` |
| vs Bots | The solo player or full party is ready and an arena is available | The same public casual policy |

Ranked and Casual PvP are separate queues. A player waiting in Ranked can opt
into a Casual search after the existing offer without leaving Ranked. A ready
Ranked roster takes priority over a competing Casual roster. The lobby keeps
parties together and freezes the roster before draft. Reconnect restores the
original seat.

Ranked is the mode that awards competitive Elo. Draft and loading show the
assigned mode and rating status. Bot names and portraits sit alongside a
small roster indicator. Bots never count toward player presence or career profiles.

## Server ownership

1. The public lobby selects authenticated humans and passes the immutable
   manifest to the worker. The existing lane AI fills the designated vacancies
   with regular heroes and follows the same combat rules. Temporary takeover
   stays attached to its human seat.
2. The persona ID connects a bot's nickname, bundled portrait and chat voice.
   Each worker draws distinct personas from a pool of 30 in a new order each
   round. Every arena has its own bot instances and can use the same catalog.
   The worker records the ID and name with the participant so draft, loading,
   scoreboard and results see the same identity. Legacy records and packets use
   their existing hero portraits. Images load from the game package.
3. The worker reads public round events and publishes authored lines through
   the bounded Match chat stream. A bot has no client session for sending
   messages. Existing user mute controls apply to the sender.
4. The career store validates eligibility and settles mixed matches under the
   existing casual policy. Retries, abandonment and recovery continue through
   durable receipts.

## Match chat

| Public cue | Example | Scope |
| --- | --- | --- |
| Match starts | «Всем удачи.» or «Погнали, удачи всем» | Up to one greeting shortly after play starts |
| A connected human gets a hero kill | «Красава, {ник}!» or «Вовремя, {ник}.» | One allied bot can react to a fresh confirmed kill |
| Victory | «Спасибо за игру!» or «гг» | Up to one closing line, subject to cooldown |

The server rotates eligible speakers and waits briefly before delivering a
line. Bot chat has a 24-second global cooldown, a 65-second per-speaker cooldown
and up to seven lines during play. It discards superseded cues. Authored RU, EN
and zh-Hans text uses the admitted nickname as its only variable. The 30
personas have [individual portrait styles](../../client/assets/ui/bot-profiles/README.md).

Public tower and boss takedowns can use the same scheduler once the event stream
has dedicated, deduplicated receipts for those milestones. Test tactical pings
and AI difficulty separately with playtesters.

## Release acceptance

- Exercise a RU phone-sized flow from Home through Casual PvP, draft and loading.
  Verify the rating policy, Cyrillic names, portraits and chat mute on a
  physical phone before publishing the mobile build.
- Run 1-human, small-party and full PvP cases on a public-role test lobby.
  Verify allocation, team composition, identity, reconnect, and that completed
  matches apply the rewards of their assigned policy exactly once.
- Measure on the intended launch host with the expected mix of matches. Track
  time to draft p50/p95, human seats per match, draft no-shows, early quits,
  rematch rate, arena CPU and snapshot p95/p99. Compare full PvP and mixed
  cohorts. Follow the [capacity and recovery guide](../public-mvp.md).

The default pool of 16 arenas can allocate up to 144 bot seats across solo
matches. Validate that workload on the launch host, including queueing a 17th
player and serving them when a slot opens. The persona catalog places no shared
reservation on those seats.

## If mixed matches later award rating

Add a versioned, immutable reward policy. Start with an opt-in pilot with at
least three humans per team, equal bot counts, and the same calibrated bot
strength tier on both teams. Keep a provisional score separate from full-PvP
Elo and placements during the pilot. Compare win rates by side and party size,
disconnects, and repeat opponents before considering a capped Elo impact.

Persist the policy version, human seats and bot strength tier with the frozen
allocation. Validate eligibility at both allocation/start and settlement.
Single-human fallback stays casual. Verify invalid compositions, repeated
settlement, party abuse, disconnect takeover and interrupted recovery before
enabling a new competitive reward policy.
