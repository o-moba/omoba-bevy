# iPad-only phone layout preview

The owner requested a cheap way to inspect iPhone layout on an iPad and explicitly ruled out a broad engine refactor.

The implementation changes the native iOS UIWindow to a centred, unscaled 852×393-point rectangle. The existing winit window-resize and window-local touch paths continue to drive all Bevy layout and cameras. A separate black UIKit window supplies a reachable Return to iPad control outside the game's viewport while the game keeps its key-window status. The bridge accepts only iPad idiom, landscape and sufficient original window space; it validates geometry and restores on return, backgrounding, rotation or unexpected changes. Preview is not persisted.

The Rust integration adds one Graphics setting, main-thread native polling and gesture cancellation on transitions. Existing rendering, cameras, HUD and per-screen coordinate calculations are unchanged. English, Russian and Chinese labels are included.

Profile reference: [Apple iPhone 16 specifications](https://www.apple.com/iphone-16/specs/) list 2556×1179 pixels; this layout profile uses 852×393 logical points in landscape. The preview uses the game's existing safe-area policy and iPad display density, so it is not hardware, cutout or pixel-density emulation.

Verification is recorded in `.agent/tasks/UI-IPAD-PHONE-PREVIEW-2026-09-29/`. Native UIKit simulator checks exercise window size, touch conversion, key-window retention, return and fail-safe restoration plus iPhone rejection. These lightweight checks avoid rebuilding the entire Rust iOS game. Full-game physical iPad installation remains a separate device check.
