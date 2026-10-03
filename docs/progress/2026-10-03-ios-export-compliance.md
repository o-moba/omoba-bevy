# Persist iOS encryption declaration — 2026-10-03

The common iOS plist previously omitted an export-compliance declaration, so each
new upload could require the questionnaire again. Version 0.34.2 includes Boolean
`ITSAppUsesNonExemptEncryption = false` for the reviewed game-only use. This is an
exempt-use declaration, not a claim of no crypto or Apple approval. The rationale,
standard non-OS TLS/signature dependencies, and reassessment triggers are recorded
in `mobile/ios/EXPORT-COMPLIANCE.md`.

The device builder and Xcode both read the common plist; the archive preparer
preserves it from the source app. Regression checks exercise these paths with
Apple signing/network operations mocked. No new app binary was built or uploaded,
and existing archive 0.34.1 (16) remains unchanged and needs its own questionnaire
answer if still pending. No account, credential or production setting changed.

Verification: all 45 iOS packaging tests passed; plist validation, locked Cargo
metadata and whitespace checks passed. Changes remain uncommitted. Raw results in
`.agent/tasks/IOS-EXPORT-COMPLIANCE-20261003/`.
