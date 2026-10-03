# iOS export-compliance metadata

Reviewed 2026-10-03 for OMOBA's current game functionality and pinned dependencies.
The common `Info.plist` declares `ITSAppUsesNonExemptEncryption` as Boolean `false`.
This is a technical self-assessment of exempt use, not a statement that the app
contains no cryptography and not an Apple-issued classification or approval.

## Reviewed use

- `passport/Cargo.toml` and pinned Ekza SDK `927fc0d` use reqwest/rustls for HTTPS
  connections supporting game account operations and avatar/weapon downloads.
  This is standard TLS implemented outside Apple's OS; do not describe it as
  exclusively OS-provided encryption.
- `client/src/career_identity.rs`, `career_devices.rs` and
  `net/public_transport.rs` use ed25519-dalek for identity and command signatures.
  Signatures authenticate requests; they do not encrypt gameplay payloads.
- SHA-256 verifies asset integrity. The SDK's local SHA-256 implementation is an
  implementation of a published standard, not a proprietary cipher.
- The product's primary purpose is playing the game. The reviewed functionality
  does not offer a VPN, general-purpose encrypted storage, a cryptographic API,
  or end-to-end encrypted messaging as a separate product feature.

Apple permits the false value for either no encryption or only exempt encryption,
including incorporated libraries. BIS distinguishes authentication, signatures,
and data integrity from cryptography for data confidentiality, and explicitly
lists games among entertainment-related exclusions. This is the basis for the
current game-use declaration, rather than assuming that all standard algorithms
or all HTTPS implementations are automatically exempt.

## Build behavior and existing uploads

`build_device.app_info()` loads the shared plist; the Xcode build phase writes
that result as its generated plist. The archive preparer copies the input app's
plist while updating its build number. Tests cover all three paths.

The setting takes effect in newly packaged builds starting with 0.34.2. It cannot
modify the already-uploaded 0.34.1 (16). Complete that build's outstanding questions
in App Store Connect, if requested. When the algorithm-type question appears,
the reviewed code uses standard algorithms outside/in addition to Apple's OS;
that question is distinct from whether their usage is exempt.

No `ITSEncryptionExportComplianceCode` is invented. If Apple requires documentation,
use its reviewed code after approval. This metadata does not finish processing,
assign testers, submit an external beta for review, or publish an App Store update.

## Reassessment triggers

Review the declaration before adding proprietary encryption, encrypted storage,
VPN/tunneling, end-to-end messaging, a general-purpose wallet/crypto interface,
or changing the SDK's crypto capabilities or the product's primary purpose.
Review destination-specific requirements when changing distribution territories;
Apple separately identifies French documentation for certain non-OS standard
implementations. Do not assume this US-use assessment answers every destination
question or override an Apple request for documentation.

## Primary sources

- [Apple: ITSAppUsesNonExemptEncryption](https://developer.apple.com/documentation/bundleresources/information-property-list/itsappusesnonexemptencryption)
- [Apple: Complying with encryption export regulations](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations)
- [BIS: Cryptography for data confidentiality](https://www.bis.gov/learn-support/encryption-controls/cryptography-for-data-confidentiality)
- [BIS: Primary functions under 5A002.a](https://www.bis.gov/learn-support/encryption-controls/5a002-a.1-a.5)
- [Apple: Documentation by algorithm type and French distribution](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption/)
