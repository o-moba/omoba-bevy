# Verification repairs

- Initial full gate found that the all-role runtime fixture accessed the private match pool path. Added a cfg(test)-only directory accessor so the test can remove its unique temporary fixture without changing production visibility or behavior. Rerun required.

- Strict Clippy flagged single-character push_str and a helper below a test module. Fixed both. Fresh review also rejects unknown future handshake versions even when both manifests carry the same value, and clears the previous verification on teardown.

- Full client tests require exact locale key parity. Added Russian and Simplified Chinese translations for every new compatibility string, including the corrected protocol message. Visual scope remains English only.

- Chinese font-subset coverage rejected five newly introduced glyphs. Rephrased those strings using the shipped vocabulary (version match/network version); no font asset rebuild or dependencies required.

- CLI fixture matrix passed; the first live-process probe raced native process startup. The smoke runner now waits for the actual listening log with a deadline before probing. No production change.

- Wire enum inventory caught the new pre-game enum and local diagnostic enum. Classified the former as strict with its independent handshake-version policy pin, and the latter as local/operator JSON. This keeps the protocol evolution guard active.

- Native English 852×393 capture passed layout bounds but exposed low-contrast error text over the scene. Switched to the text-danger token and a small panel behind the explanation, shortened the unavailable message, and scheduled one repeat of that affected capture. Fixed a registry-edit syntax error caught by rustfmt before compilation.

- Native logs showed that the menu owns another automatic retry loop, independent of the session reconnect loop. Added a confirmed-mismatch guard there and a regression that still permits unavailable-server retries. The full gate had passed before this last focused fix; final client test and lint checks cover it.
