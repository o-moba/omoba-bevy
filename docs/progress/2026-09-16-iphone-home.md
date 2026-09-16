# Physical iPhone home-playtest kit — 2026-09-16

Fresh client and Mac server builds use canonical gameplay revision `9c9fedf`,
version `0.20.0-rc.3`, locked offline dependencies, Rust 1.93.1 and Xcode 26.2.
The client targets physical arm64 iOS 15+, rather than Simulator. Source hashes
were recorded before and checked after compilation.

`mobile/ios/install_device.py` checks signature, profile expiration, enrolled
physical iPhone availability and Developer Mode. It updates the application and
launches only after successful installation; there is no automatic uninstall.
`--check` performs no install or launch. Multiple eligible phones require explicit
selection. `home_server.py` wraps the existing owned-process launcher, prints
current LAN addresses, forces native bot practice and removes inherited career/QA
settings so the local session needs no external database.

The local, ignored `builds/iphone-home-2026-09-16/` kit contains the signed app/IPA,
fresh Mac arm64 server, both standalone launchers and a Russian START-HERE guide.
Existing signing credentials were reused without changing them. The selected
profile contains the paired iPhone and expires on 2027-09-04 UTC. All zipped app
bytes match the signed bundle. Packaged assets passed the candidate gate.

Twenty packaging/installer tests passed. The actual standalone host was started
on a free port, verified in practice mode with inherited database/release settings
deliberately overridden, interrupted through Ctrl+C and checked for port release.
The read-only device preflight reported the paired iPhone unavailable, with
Developer Mode enabled. No install or physical-device gameplay is claimed.

No gameplay change, new production dependency, public deployment or background
server was introduced. Build caches are separate from retained ready-to-run files.
Detailed evidence is in `.agent/tasks/IPHONE-HOME-2026-09-16/`.
