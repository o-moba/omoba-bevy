---
name: omoba-class-navigator
description: Trace Omoba classes and abilities to their authored stats, authoritative Rust combat logic, presentation, and regression tests. Use for questions or changes about class kits, skill behavior, role balance, or where the code for a hero lives in omoba-bevy.
---

# Omoba class navigator

Read the [hero code map](../../../docs/hero-code-map.md) from this repository. For a named class, follow its catalog, runtime, and regression links. For a named ability, resolve the stable ability ID in the linked catalog first.

Follow the dispatch for the actual equipped skill when tracing Workshop builds. Treat `skills.skillfx` as client presentation and `common` as the shared combat authority for online and offline play.

Keep the map's affected row and source links current when adding a class or moving its implementation.
