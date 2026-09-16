# Skill icon atlas

Generated for Open Moba with OpenAI imagegen on 2026-09-16. No input/reference
images or third-party game art were supplied. The original generated PNG is
preserved without cropping or other raster edits. Runtime UV rectangles select
one cell; `client/src/skill_icons.rs` maps stable gameplay ability IDs to art.
Replacing this presentation atlas never changes ability stats or network IDs.

Layout: four columns, four rows, read left to right. Rows are Warrior (shield
impact, rally fist, sword strike, crossed axes), Mage (arc bolt, mana crystal,
ice lance, fireball), Ranger (quick arrow, healing bandage, piercing arrow,
longshot bow), Cleric (smite star, healing hands, mana chalice, guardian wings).
Cells use dark navy backgrounds, centered fantasy objects, no text, designed
for small circular touch buttons. Atlas dimensions are 1254 × 1254 pixels;
runtime cell rectangles support fractional pixel coordinates.
