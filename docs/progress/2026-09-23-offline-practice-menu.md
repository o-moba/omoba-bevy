# Offline practice and reachable Game menu

Added a separate socket-free character playground, rather than relabeling the server-backed Bot Practice. It reuses the normal snapshot, model, movement and combat presentation paths; the deliberately small simulation shares ability/utility definitions and has no career/purchase result path. All shipped classes and avatars are usable, all four slots unlock at level six, targets recover, and leaving restores the online address.

The main/settings modal has a fixed × header and navigation footer; content scrolls independently. Touch scrolling is tested against actual Bevy layout at mobile dimensions/DPI, including cancellation of button taps after a drag. Modal UI scale stays one even from the desktop Home screen.

Verification and raw native captures: `.agent/tasks/TASK-OFFLINE-DEMO-MENU-2026-09-23/`. Native desktop mobile-profile captures and synthetic touch tests do not claim physical iPad verification. No Apple build is uploaded by this change. Existing Xcode signing and Makefile edits remain user-owned.
