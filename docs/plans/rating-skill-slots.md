# Rating-unlocked skill slots

Status: **idea / TODO**, recorded 2026-10-12 from the owner. Nothing here is
implemented or decided. This page keeps the idea and its open design questions
together until it is scheduled.

## Idea

As a character's rating grows, the player can add more skills to it. A hero would
no longer be limited to four skills: progression unlocks extra slots, possibly up
to about ten. The exact cap is open.

## Where it meets the current game

| Area | Today | Why it matters |
| --- | --- | --- |
| Recipe | [`BuildRecipe`](../../shared/src/loadout.rs) is core + passive + **four** unique skill IDs; the array index is the physical Q/W/E/R binding ([equipped skills](../equipped-skills.md)) | The four-slot shape is a contract, not just a UI choice |
| Slots | `SkillSlot` is a four-variant enum, `SkillSlot::ALL: [Self; 4]` ([shared/src/lib.rs](../../shared/src/lib.rs)); catalogue validation checks `abilities.len() == SkillSlot::ALL.len()` ([catalog.rs](../../shared/src/catalog.rs)) | More slots means a slot index type, not more enum variants |
| Unlocks in a match | Skill roles unlock at levels 1/2/4/6 and rank up with level points | Extra slots need their own in-match unlock and point budget |
| Rating | Competitive rating exists only for eligible bot-free PvP ([public MVP](../public-mvp.md)); career progression and history are saved for approved games | "Rating" must be defined before anything can be gated on it |
| Protocol | Join carries the loadout; the shared enums are Strict (protocol 11) | A longer recipe is a protocol change for client and server |
| Authoring | Class Workshop composes recipes; the creator catalogue maps skills to model requirements | More skills per hero means more Studio briefs per class |

## Open questions

1. **Which rating unlocks a slot?**
   - A per-character mastery value (games, wins and objectives played with that hero).
   - The account's competitive rating.
   - A season level.

   Per-character mastery fits "the character grows" best and does not lock bot-only
   players out.
2. **Active or known?**
   - **A, more active slots:** five to ten skills bound at once in a match.
   - **B, a larger known pool:** the hero learns more skills over time, but a match
     still binds a fixed number (four to six), chosen in the draft.

   B adds variety without making high-rating players strictly stronger. A raises
   power and input complexity.
3. **Fairness.** In ranked, should everyone get the same active-slot count, with
   unlocks widening only choice? Or does matchmaking by rating make the gap
   acceptable? Unranked and practice can be more generous.
4. **Input.**
   - Desktop: Q/W/E/R plus D/F, then number keys.
   - Gamepad: chords or a second page.
   - Phone: a second ring or page beside the radial; the edge HUD is already tight.

   Decide the maximum that each control scheme can really play.
5. **Economy inside a match.** Unlock levels and rank points for slots five and up,
   and mana and cooldown budgets. Also a limit on ultimate-role skills: four
   ultimates are valid today. Bot upgrade priority must follow.
6. **Server authority.** The server must check the unlocked slot count from saved
   progression at admission, the same way it checks loadouts today. Offline
   practice could allow any count.
7. **Compatibility.** Older clients must not join with a longer recipe they cannot
   render, and recipes of four skills stay valid.

## TODO when scheduled

- [ ] **S0 — Decide** answers 1–3 and the target cap (6? 8? 10?), with one
      desktop and one phone control mock-up.
- [ ] **S1 — Data model.** Turn the fixed four-slot recipe into a bounded slot
      list with a schema version. Keep `SkillSlot` Q/W/E/R as the first four
      bindings, and keep every four-skill recipe and saved build loading.
- [ ] **S2 — Progression.** Store the chosen rating per account and character,
      compute it server-side, expose it in the career/profile UI and on
      app.omoba.io, and define the unlock thresholds.
- [ ] **S3 — Match rules.** In-match unlock levels, skill points, cooldown and mana
      budgets for extra slots, the bot priority order, and ultimate limits.
      Balance them with the headless harness before any public test.
- [ ] **S4 — Input and HUD.** An ability bar for more than four slots on desktop,
      gamepad and phone, plus aim previews and upgrade badges for the new slots.
- [ ] **S5 — Protocol and admission.** Make the protocol change, have the server
      check the slot count against progression, and keep the stale-client
      fallback.
- [ ] **S6 — Workshop, draft and web.** Choose extra skills in the draft and Class
      Workshop, and show the unlocked slots and their progress on the player
      portal.
- [ ] **S7 — Prototype gate.** First try six slots in offline practice only,
      behind an opt-in flag, before any ranked or public change.
