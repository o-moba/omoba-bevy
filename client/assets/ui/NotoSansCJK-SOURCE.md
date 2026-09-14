# Noto Sans CJK SC Regular

Unmodified official Noto CJK font, release **Sans2.004**, font version 2.004.

- Project: https://github.com/notofonts/noto-cjk
- Release: https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004
- Font: https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf
- License: https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/LICENSE
- Local license: `NotoSansCJK-OFL.txt` (SIL Open Font License 1.1, unmodified).
- Embedded copyright: © 2014–2021 Adobe (http://www.adobe.com/).
- Downloaded: 2026-09-14.
- Bytes: 16,437,364.
- SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

The client keeps Inter for Latin/Cyrillic UI. Text containing CJK, Kana or
Hangul selects this bundled font because Bevy 0.18 `TextFont` exposes one
font handle, without a per-span fallback list. The font is packaged for
offline use; no operating-system font or runtime download is required.
This is not a promise of coverage for every Unicode script/codepoint.

The downloaded font's OpenType format-12 cmap was checked for nonzero glyphs
for `QA小明中文漢字かなカナ한글Дмитрий`; the UI regression also loads both real
font assets and verifies font changes when an existing name changes script.
Native screenshot verification remains a separate render check.
