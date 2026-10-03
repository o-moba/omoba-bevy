# iPhone preview framing and Android 0.34.2

The user reported head/lateral cropping on iPhone and an offline/reconnecting status while an older iPad build is online. Android universal packaging remains part of this task.

Implemented measured, padded preview framing using the existing glTF bounds traversal. It respects normalized size and off-center origins and remains steady during turntable rotation. No match camera, combat collider or model-scale preference changes.

Beta service is active; a protocol-4 UDP probe from this Mac received a matching transport challenge and a lobby bootstrap snapshot. An initial diagnostic probe mistakenly used protocol 2 and timed out; that was a diagnostic error, not evidence of a server outage. Client transport and protocol were unchanged between 0.34.0 and 0.34.1. The iPhone is unavailable to devicectl; the iPad is connected. No production changes performed and no phone-side root cause claimed. User network comparison is pending.

Current verification and artifact results are recorded in `.agent/tasks/IPHONE-FRAMING-CONNECTION-20261003/`.
