# Verdant Confluence — Art Direction

## Creative premise

An ancient forest observatory has become a contested sanctuary. Two ivory-stone faction bases face one another across a turquoise river, with worn ceremonial roads threading through sculpted jade forests. The original working concept is **Verdant Concord: ruins of a living observatory**; the deliverable directory and arena title are Verdant Confluence.

The target is a cohesive premium stylized environment, readable from gameplay height and attractive in close views. Broad forms establish silhouettes; bevels, layered construction, selective ornament, and composed planting provide detail. Geometry must move beyond recognizable unmodified primitives. Saturated color and emission belong to landmarks rather than every prop.

## Palette and material hierarchy

| Role | Palette | Treatment |
| --- | --- | --- |
| Forest shadows | Deep teal `#163C39` | Dark interiors and understory |
| Main foliage | Jade `#387B57`, sage `#80A76D` | Layered canopy masses with restrained variation |
| Architecture | Limestone `#CEC5A3`, weathered stone `#777F73` | Rough surfaces, softened edges, heavier foundations |
| Ornament | Aged brass `#9F7A42` | Narrow rings, collars, inlays, and astronomical details |
| Terrain | Moss `#4F6542`, earth `#665640` | Broad low-contrast areas around gameplay routes |
| Roads | Worn stone `#ABA185` | Pale, interrupted paving and irregular shoulders |
| Water | Deep teal `#176F78`, shallows `#43AFB4` | Visible banks and occasional light foam accents |
| Factions | Emerald `#58DCA1`, azure `#599EF2` | Matching structure families with distinct crystals and banners |

These are authoring targets, not claims of calibrated render output. Materials should remain useful under ordinary game lighting, with renderer-specific presentation effects documented separately.

## Modular asset brief

### Architecture

- Hero sanctuary: 12–16 units tall, stepped foundation, central faceted crystal, three or four surrounding ribs, open upper silhouette, and an inscribed ring. Produce equivalent green and blue variants.
- Defense tower: 7–9 units tall, broad octagonal foot, tapered masonry, brass collar, and elevated crystal crown. Its silhouette must differ clearly from a tree.
- Gateway: carved paired pillars with inward-facing ornamental horns and an open traversal gap.
- Supporting kit: ruined wall, broken column, obelisk, pennant, and lantern pedestal.
- Bridge: pale stone deck, substantial abutments, and low parapets; the central crossing must support the full 12-unit route width.

### Nature

- Three broadleaf tree variants, approximately 7–11 units tall: tapered or bent trunks, exposed roots, and asymmetric layered canopies.
- Two slender conifer variants with stepped foliage and distinct branching silhouettes.
- Stratified rock formations, mossy boulders, rooted stump, and fallen log.
- Fern, broadleaf plant, grass fan, flowering shrub, and river reeds.
- Reusable composed clusters of rocks with understory and trees with roots and shrubs.

Use common materials, ground-level origins, named asset roots, and shared mesh data for placed copies. Variation should come from a limited designed kit, scale bands, and rotation, with deterministic placement.

## Arena composition

Preserve the current MapLayout footprint, the two base centers and 46-unit square pads, all three 12-unit lane centerlines, the 18-unit diagonal river, three neutral-camp anchors, and two boss anchors. One Blender unit equals one game unit. Record the Blender-to-Bevy axis mapping in the handoff.

- Maintain quiet, continuously legible road surfaces. Concentrate visual complexity beyond their shoulders.
- Compose forests as thick islands with dark interiors, medium-height canopies, and low vegetation at edges. Keep camp clearings and objective approach routes open.
- Distinguish the boss locations using a broken observatory motif and a rooted standing-stone motif.
- Shape riverbanks with rock shelves, reeds, shallow margins, and grouped foam details. River crossings need visible structural support.
- Give each base paved negative space around the sanctuary, patterned masonry, corner planting, and a small number of ceremonial accents.
- Use taller trees and cliff masses to frame the boundary while preserving major landmark visibility from the main camera.
- Place detail in groups of unequal sizes, with gaps between groups. Avoid uniform random scatter and repeated grid spacing.

## Lighting and cameras

Use warm afternoon sunlight, cool sky fill, readable soft shadows, and subtle atmospheric depth. Keep bloom restrained and ground detail visible.

Required presentation views:

1. An orthographic or restrained-perspective three-quarter overview showing the complete arena composition.
2. A gameplay-height river/forest view demonstrating route readability and local asset quality.
3. A sanctuary detail showing materials, edge treatment, and ornamental hierarchy.

A near-top-down layout view is useful for checking uninterrupted lanes, preserved objective locations, and clearances.

## Visual review criteria

- The arena has recognizable forest-observatory identity at thumbnail scale.
- Both faction sanctuaries read immediately, and defense structures remain distinct from vegetation.
- All three routes and the diagonal river are readable throughout the scene.
- No major hovering objects, unsupported bridges, intrusive tree/rock placement, or repetitive scatter patterns are visible.
- Tree silhouettes and architecture show deliberate modeling rather than plain primitive assemblies.
- At least one close view demonstrates purposeful prop composition and consistent material hierarchy.
- Reusable assets, environment instances, presentation lights, and cameras remain organized separately.
- The saved Blender file, GLB exports, inventory, and handoff describe the actual deliverable and distinguish game-ready material content from presentation-only effects.

## Scope boundary

This delivery creates and assembles original art in Blender. Runtime integration, gameplay changes, collision implementation, LOD tuning, and in-game lighting/shader work belong to the next stage. Existing layout contracts remain authoritative.
