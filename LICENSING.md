# Licensing Open Moba

Open Moba is an open-source MOBA and reusable engine project. You may study, modify, redistribute and commercially operate licensed versions, subject to the applicable standard licenses. We want improvements to remain useful to players and developers, including people running independent games and servers.

## License map

| Material | License / scope |
| --- | --- |
| Original source in `server/` | [AGPL-3.0-only](LICENSES/AGPL-3.0-only.txt) |
| Original source in `client/`, `shared/`, `passport/`, `skills/`, `harness/`, `arena-sync/`; original scripts, mobile scaffolding and configuration elsewhere | [MPL-2.0](LICENSES/MPL-2.0.txt) |
| Original standalone project documentation, including this guide and the mission/contribution/brand policy text | [CC-BY-4.0](LICENSES/CC-BY-4.0.txt) |
| Original Verdant Confluence `.blend`/`.glb` visual assets and rendered stills in `art/verdant-confluence/` and `client/assets/verdant/` | [CC-BY-4.0](LICENSES/CC-BY-4.0.txt) |
| Sprite, presentation2D and world2D art already dedicated to CC0 | Their existing directory `LICENSE.md` declarations; CC0 is preserved |
| Imported models, animations, font and other third-party works | Their own notices; see [ATTRIBUTION.md](ATTRIBUTION.md) and the relevant manifests |
| Dependencies, the separately maintained Ekza SDK, runtime downloads and user-supplied avatars | Their own licenses/permissions; this repository does not grant rights on their authors' behalf |
| Names and logos used as identifiers of the official project | Separate [brand policy](TRADEMARKS.md); no automatic trademark license |

Source/configuration/metadata files in art directories remain MPL-2.0; the visual asset grant does not change their source license. Comments, docstrings and code examples inside source files use the license of their containing source file. Asset-specific notices take precedence over the defaults above. Included third-party excerpts, license texts, standards and notices retain their existing terms. Do not overwrite a specific license notice with a directory default.

For attribution of original documentation or Verdant assets, use **Open Moba contributors**, link to https://github.com/o-moba/omoba-bevy, identify CC-BY-4.0 and indicate modifications. Preserve any supplied individual creator credits. This credit does not assert that every generated element is copyrightable in every jurisdiction; licenses grant only rights their licensors hold.

## Why these licenses

AGPL protects reciprocity for the authoritative server. A modified version made available for remote network interaction must prominently offer its users the corresponding source of that version (AGPL section 13). Distributing server binaries also carries source obligations (section 6). The obligation is to provide source under the license, not to send the upstream project a pull request, pay royalties or request permission to compete.

MPL protects the source files of the client and reusable engine components when copies are distributed. Modified covered files, including new files containing covered code, stay under MPL. Separately authored additions may have other licenses. MPL allows different terms for executable distribution while preserving recipients' rights to covered source (sections 3.1–3.3), which suits desktop and mobile distribution. This is not a guarantee of approval by any app store.

The server is **not** offered as `MPL-2.0 OR AGPL-3.0-only`. Client and server are separate programs communicating over the protocol. Do not copy AGPL-only server code into an MPL client and assume a directory name changes its license. Shared MPL files do not carry an Exhibit B incompatibility notice; MPL section 3.3 allows combining eligible files with AGPL software while meeting the applicable terms. Keep the original MPL notices and make the shared source available accordingly.

These licenses do not require someone to share our mission, prevent an independently written competitor, or guarantee development funding. We express our direction through [MISSION.md](MISSION.md), project decisions and the official service. Those aspirations do not add non-commercial, field-of-use, pay-to-fork or anti-competition restrictions to the licenses.

## Avatars belong to their creators and rights holders

Connecting a wallet, owning a token or using an avatar in a match does not assign copyright to Open Moba or automatically place the model under our code/content licenses. Token ownership alone does not establish the right to copy, adapt or redistribute its associated artwork. Imported content needs an actual license or permission for the intended use; people retain their existing rights.

Player-provided models, private files, account data and runtime downloads are outside the repository's default grants. Any future upload or hosting permission must be stated separately and limited to the service's needs. This document is not consent to upload, publish or license anyone's likeness or avatar.

## Contributing and distributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for contributions under the applicable license without copyright assignment. See [SOURCE.md](SOURCE.md) for source and notice delivery for binaries and modified servers. A license file or Cargo metadata field alone does not satisfy every distribution obligation.

This policy grants rights in material the project is authorized to license. It cannot cure missing third-party permissions, change a dependency's license, or withdraw valid earlier grants. The standard license texts govern if this explanatory guide differs from them. Before a public distribution, resolve the rights and source obligations of the actual dependency and asset set being shipped.

Primary references: [AGPL](https://opensource.org/license/agpl-3.0), [MPL text](https://www.mozilla.org/en-US/MPL/2.0/), [MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/), [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
