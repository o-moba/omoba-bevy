# Skill icon atlas

Generated for Open Moba with OpenAI imagegen on 2026-09-16. No input/reference
images or third-party game art were supplied. The original generated PNG is
preserved pixel-for-pixel as the first four rows. Runtime UV rectangles select
one cell; `client/src/skill_icons.rs` maps stable gameplay ability IDs to art.
Replacing this presentation atlas never changes ability stats or network IDs.

Layout: four columns, five rows, read left to right. Rows are Warrior (shield
impact, rally fist, sword strike, crossed axes), Mage (arc bolt, mana crystal,
ice lance, fireball), Ranger (quick arrow, healing bandage, piercing arrow,
longshot bow), Cleric (smite star, healing hands, mana chalice, guardian wings),
Warden (claw swipe, bark-covered arm, spear at a target rune, spirit beast).
Cells use dark navy backgrounds, centered fantasy objects, no text, designed
for small circular touch buttons. Atlas dimensions are 1254 × 1568 pixels;
runtime cell rectangles support fractional pixel coordinates.

The fifth (Warden) row was added on 2026-09-24 with Higgsfield `gpt_image_2_5`,
using a downscaled copy of the first four rows as the only style reference.
Four 1024 × 1024 outputs were resized to 314 × 314 and placed below the untouched
original rows; their alpha is the mean alpha of the sixteen original cells so
the new icons keep the same translucent navy surround. Job IDs are in `manifest.json`.
