# iPad-only phone layout preview

The owner requested a cheap way to inspect iPhone layout on an iPad and explicitly ruled out a broad engine refactor.

The implementation changes the native iOS UIWindow to a centred, unscaled 852×393-point rectangle. The existing winit window-resize and window-local touch paths continue to drive all Bevy layout and cameras. A separate black UIKit window supplies a reachable Return to iPad control outside the game's viewport while the game keeps its key-window status. The bridge accepts only iPad idiom, landscape and sufficient original window space; it validates geometry and restores on return, backgrounding, rotation or unexpected changes. Preview is not persisted.

The Rust integration adds one Graphics setting, main-thread native polling and gesture cancellation on transitions. Existing rendering, cameras, HUD and per-screen coordinate calculations are unchanged. English, Russian and Chinese labels are included.

Profile reference: [Apple iPhone 16 specifications](https://www.apple.com/iphone-16/specs/) list 2556×1179 pixels; this layout profile uses 852×393 logical points in landscape. The preview uses the game's existing safe-area policy and iPad display density, so it is not hardware, cutout or pixel-density emulation.

Verification is recorded in `.agent/tasks/UI-IPAD-PHONE-PREVIEW-2026-09-29/`. Native UIKit simulator checks exercise window size, touch conversion, key-window retention, return and fail-safe restoration plus iPhone rejection. These lightweight checks avoid rebuilding the entire Rust iOS game. Full-game physical iPad installation remains a separate device check.

Final root verification on main (`3cc2553`): 804 client tests passed, one existing migration report ignored; locked client build, production Clippy with warnings denied, formatting and diff checks passed. All four Swift bridges typechecked against the device SDK. Root rebuilt and reran the native simulator harness: 16 iPad assertions and two iPhone rejection assertions passed. An actual Simulator tap reached window-local point (70,323); the independent return button restored full size. The Bevy desktop client also captured 14 screens at 852×393 with phone controls. Those captures predate only the final Chinese wording correction; rendering and English text are identical.

The first full suite found unsupported Chinese characters in the new labels. Rewording fixed them without regenerating fonts. Native harness screenshots and representative real Bevy phone-size captures are saved in [omoba-ui](../../../omoba-ui/captures/2026-09-29-ipad-phone-preview/README.md), with their different evidence scopes clearly labelled. Temporary smoke apps, the small Swift cache and the merged implementation worktree were removed; the iPad simulator was returned to its original stopped state.
