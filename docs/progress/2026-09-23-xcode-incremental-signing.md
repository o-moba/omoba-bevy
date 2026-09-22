# Incremental Xcode device signing repair

Reproduced the reported iPad installation failure without installing: the first
Debug device build passed strict signature verification; the unchanged second
build succeeded but its executable had no signature. The Rust phase used copy2,
restoring the cached unsigned binary with its old modification time and allowing
Xcode to skip CodeSign.

Use copyfile to retain a fresh output modification time. Xcode continues to own
signing; no certificate, team, provisioning or project settings were changed.
The regression fails before the fix and passes afterwards. All 41 iOS tooling
tests pass. Two consecutive signed Debug device builds now pass strict signature
verification with matching executable/dSYM UUIDs. No device installation or
TestFlight upload was performed by this task.
