"""Original Adventurer skill emblems, painted from project-authored geometry.

Four distinct silhouettes (thrust, feint, rear strike, heavy thrust). No copied
game art, fonts, sampled imagery or generated placeholders. Rebuild with the
existing developer Pillow installation; not a production dependency.
"""
import hashlib
import json
import math
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter
from build_skill_icons import mask

ROOT = Path(__file__).resolve().parents[1]
SIZE = 512
COLORS = [(95, 223, 239), (181, 103, 244), (244, 64, 105), (255, 187, 60)]


def blade(image, center, angle, scale=1, alpha=255):
    draw = ImageDraw.Draw(image)
    def points(coords):
        c, s = math.cos(angle), math.sin(angle)
        return [(center[0] + scale * (x*c-y*s), center[1] + scale * (x*s+y*c)) for x,y in coords]
    draw.polygon(points([(-19, 95), (19, 95), (23, 17), (-23, 17)]), fill=(104, 42, 49, alpha))
    for y in range(32, 91, 15):
        draw.line(points([(-18, y), (18, y-8)]), fill=(228, 151, 66, alpha), width=6)
    draw.polygon(points([(-56, 24), (-48, -1), (48, -1), (56, 24), (14, 16), (-14, 16)]), fill=(229, 168, 76, alpha))
    draw.polygon(points([(-32, -6), (-27, -124), (0, -200), (27, -124), (32, -6)]), fill=(199, 232, 235, alpha))
    draw.polygon(points([(0, -12), (0, -192), (27, -124), (32, -6)]), fill=(58, 112, 130, alpha))
    draw.line(points([(-30, -6), (-26, -122), (0, -200)]), fill=(250, 255, 239, alpha), width=5)


tiles = []
for index, color in enumerate(COLORS):
    tile = Image.new('RGBA', (SIZE, SIZE), (7, 22, 28, 255))
    glow = Image.new('RGBA', tile.size)
    g = ImageDraw.Draw(glow)
    g.ellipse((105, 85, 430, 410), fill=(*color, 90))
    tile.alpha_composite(glow.filter(ImageFilter.GaussianBlur(50)))
    d = ImageDraw.Draw(tile)
    if index == 0:
        for n in range(3):
            d.line((110+n*32, 350, 245+n*26, 100), fill=(*color, 115+n*35), width=10-n*2)
        blade(tile, (261, 300), .58)
    elif index == 1:
        # Two withdrawing silhouettes and a hook arc communicate misdirection.
        blade(tile, (185, 275), -.55, .8, 105)
        d.arc((112, 105, 422, 368), 188, 335, fill=(*color, 240), width=17)
        d.polygon([(405, 163), (394, 221), (363, 175)], fill=(*color, 255))
        blade(tile, (294, 310), .47, .84)
    elif index == 2:
        d.ellipse((188, 76, 301, 189), fill=(58, 91, 110, 255))
        d.polygon([(181, 181), (320, 181), (358, 399), (145, 399)], fill=(41, 70, 85, 255))
        for y in range(205, 341, 34):
            d.line((250, y, 250, y+20), fill=(100, 139, 153, 255), width=9)
        d.ellipse((205, 231, 295, 321), outline=(*color, 255), width=13)
        blade(tile, (162, 350), .86, .79)
    else:
        for a in range(0, 360, 45):
            rad=math.radians(a)
            d.line((280+55*math.cos(rad), 208+55*math.sin(rad), 280+125*math.cos(rad), 208+125*math.sin(rad)), fill=(*color, 255), width=13)
        blade(tile, (256, 337), .25, 1.05)
    tiles.append(tile)
source = Image.new('RGBA', (1024, 1024))
for i, tile in enumerate(tiles):
    source.paste(tile, ((i%2)*SIZE, (i//2)*SIZE))
source_path = ROOT/'assets-src/weapons/adventurer-icons.png'
source.save(source_path, optimize=True)
atlas = Image.new('RGBA', (1024, 256))
for i, tile in enumerate(tiles):
    tile = tile.resize((256,256), Image.Resampling.LANCZOS)
    tile.putalpha(mask(256))
    atlas.paste(tile, (i*256, 0))
path = ROOT/'client/assets/ui/skills/dagger-skills.png'
atlas.save(path, optimize=True)
manifest = ROOT/'client/assets/ui/skills/manifest.json'
doc = json.loads(manifest.read_text())
doc['atlases'] = [a for a in doc['atlases'] if a['path'] != 'ui/skills/dagger-skills.png']
doc['atlases'].append(dict(path='ui/skills/dagger-skills.png', columns=4, rows=1,
    classes=['adventurer'], width=1024,height=256,sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
doc['sources'] = [s for s in doc['sources'] if s['class_id'] != 'adventurer']
doc['sources'].append(dict(class_id='adventurer', file='adventurer-icons.png', dimensions=[1024,1024],
    sha256=hashlib.sha256(source_path.read_bytes()).hexdigest(), generator='scripts/build_dagger_icons.py',
    provenance='Original Open Moba vector-painted silhouettes, no third-party game artwork',
    license='CC-BY-4.0', abilities=['dagger_deadly_blow','dagger_bluff','dagger_backstab','dagger_lethal_blow']))
manifest.write_text(json.dumps(doc,indent=2)+'\n')
