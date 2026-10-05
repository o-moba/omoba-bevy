# Verdant Dragon

Original low-poly raid boss, authored for Open Moba contributors; CC-BY-4.0.
Regenerate `.blend` and `client/assets/bosses/verdant-dragon.glb` using Blender
with `scripts/build_verdant_dragon.py`. No external geometry/textures.

One skinned mesh, nine bones, Idle and Walk clips. Wings, tail, neck and feet
move without simulation root motion. Blender +Y facing exports to glTF -Z; the
boss adapter rotates it to the server's +Z convention.

The old `king-mutatio` network boss ID and gameplay stats remain compatible;
its presentation name is localized as Verdant Dragon. The former model remains
in the reviewed historical asset inventory but is not loaded for this boss.
