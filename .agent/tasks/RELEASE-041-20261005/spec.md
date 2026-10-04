# Frozen release acceptance criteria

AC1: Commit and merge tested compatibility feature into origin/main after required CI succeeds.
AC2: Deploy immutable Linux server release, preserve rollback, verify lobby and worker compatibility plus signed admission/reconnect.
AC3: Archive signed iOS 0.41.0 build 20 from clean merged source, verify signature, assets, dSYM and encryption declaration.
AC4: Upload to App Store Connect and report actual delivery/processing status; do not equate archive success with TestFlight availability.

Constraints: do not interrupt active matches, preserve service configuration, no new dependencies/secrets/CI changes. Disk cache deletion requires pending explicit approval.
