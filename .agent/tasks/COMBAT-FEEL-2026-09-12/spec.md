# Combat presentation and minion roles

Frozen: 2026-09-12. Base: 6d46076. Scope includes the user's follow-up request for extensible cosmetic profiles.

## Acceptance criteria

- AC1: Class-specific projectiles replace generic spheres: arrows, arcane bolts, holy bolts and a visible warrior crescent. Gameplay trajectory, damage and targeting remain server-owned.
- AC2: Each existing three-unit lane wave contains two melee minions and one caster. Melee closes to strike; casters have lower durability and launch real ranged projectiles. Team/role silhouettes and release animations are distinct. Existing lane routing and wave/reward cadence remain intact.
- AC3: Positive confirmed damage produces bounded, deduplicated combat events with typed source/target identities and actual HP removed. Desktop/mobile and 3D/2D clients show readable floating damage and impacts; resets, healing and missing projectiles cannot fabricate damage.
- AC4: A versioned cosmetic registry supports class/action defaults and avatar/sprite overrides, packaged projectile models or animated sprites, and avatar animation-clip aliases. Invalid/missing profiles/assets have safe built-in fallback. Cosmetic fields cannot alter simulation. Document an actionable contributor example.
- AC5: Focused and regression Rust tests, real UDP combat verification, native desktop and touch-preview visual checks pass on the final code. The native touch preview does not substitute for testing a physical mobile device. Document measured limits and tuning assumptions without claiming competitive balance.
- AC6: Workspace version/changelog/features/session notes are updated, evidence maps every criterion to fresh results, and all task changes are committed without touching unrelated cinematic files.

## Verification

Run formatter, appropriate cargo workspace tests and native client/server build using existing cached dependencies. Verify mixed waves, projectile hit timing, damage event identity/deduplication and compatibility through tests and a local UDP harness. Capture actual running client scenes in desktop and touch preview, including combat feedback. Review assets/config fallback and lifecycle resource bounds. No new production dependency, remote cosmetic download, deployment change or gameplay entitlement system is included.
