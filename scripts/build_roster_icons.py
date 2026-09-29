#!/usr/bin/env python3
"""Reproducible original geometric glyphs for the nine standard skill families.
Developer-only Pillow utility; the runtime loads the committed PNG.
"""
from pathlib import Path
from PIL import Image, ImageDraw
import math

ROOT = Path(__file__).resolve().parents[1]
SIZE = 128
PALETTES = ['#fb923c','#e8c2fa','#5de2ef','#ae82ef','#ffb067','#bdb7ff','#63cfff','#84ebbd','#b7e8ff']
image = Image.new('RGB', (SIZE*4,SIZE*9), '#101725')
for row, color in enumerate(PALETTES):
    for col in range(4):
        tile = Image.new('RGB',(SIZE,SIZE),'#101725'); d=ImageDraw.Draw(tile)
        d.rounded_rectangle((5,5,122,122),radius=20,fill='#1b2539',outline=color,width=3)
        cx,cy=64,62
        # Each family has a distinct central motif; slot geometry is consistent.
        if row==0: # anvil and heated fault
            d.polygon([(30,46),(95,46),(83,60),(70,60),(78,84),(42,84),(51,60),(38,60)],fill=color)
        elif row==1: # crossed facets
            d.polygon([(64,24),(77,56),(64,95),(51,56)],outline=color,width=5)
            d.line([(32,70),(90,42)],fill=color,width=5)
        elif row==2: # angular fist / thunder
            d.line([(72,25),(43,59),(70,59),(49,94)],fill=color,width=9)
        elif row==3: # torn crescent
            d.ellipse((32,28,94,92),fill=color);d.ellipse((48,21,101,76),fill='#1b2539')
            d.line([(38,88),(48,60)],fill=color,width=4)
        elif row==4: # ember petals
            for k in range(3):
                a=k*math.tau/3; x=cx+20*math.cos(a);y=cy+20*math.sin(a)
                d.ellipse((x-13,y-13,x+13,y+13),outline=color,width=5)
        elif row==5: # orb and orbit
            d.ellipse((49,47,79,77),fill=color);d.ellipse((23,39,105,83),outline=color,width=3)
            d.ellipse((42,22,86,102),outline=color,width=3)
        elif row==6: # split rift arrow
            d.line([(30,87),(77,40)],fill=color,width=6);d.polygon([(60,30),(98,27),(95,65),(85,45)],fill=color)
        elif row==7: # linked rings
            d.rounded_rectangle((28,35,68,69),radius=14,outline=color,width=5)
            d.rounded_rectangle((60,57,100,91),radius=14,outline=color,width=5)
        else: # ice shield
            d.polygon([(64,27),(96,39),(89,76),(64,98),(39,76),(32,39)],outline=color,width=5)
            d.line([(64,37),(64,83)],fill=color,width=4);d.line([(45,59),(83,59)],fill=color,width=4)
        # Q trajectory, W protection, E displacement, R radial impact.
        if col==0:d.line([(21,105),(96,105),(89,98),(96,105),(89,112)],fill='white',width=3)
        elif col==1:d.arc((16,14,111,111),195,345,fill='white',width=4)
        elif col==2:
            for x in [23,94]:d.line([(x,41),(x+9,62),(x,83)],fill='white',width=3)
        else:
            for k in range(8):
                a=k*math.tau/8;d.line([(64+44*math.cos(a),62+44*math.sin(a)),(64+51*math.cos(a),62+51*math.sin(a))],fill='white',width=3)
        image.paste(tile,(col*SIZE,row*SIZE))
image.save(ROOT/'client/assets/ui/skills/roster-skills.png')
