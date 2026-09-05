#!/usr/bin/env python3
"""Independent, dependency-free validation of the saved Verdant art deliverable.

Usage: python3 scripts/validate_art.py [--output report.json] [--skip-assimp]
Validates files already on disk; never launches Blender or changes source files.
"""
import argparse
from collections import Counter
import datetime
import gzip
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
EXPECTED = {
    'sanctuary_green', 'sanctuary_blue', 'watchtower_green', 'watchtower_blue',
    'banner_green', 'banner_blue', 'bridge', 'ruined_arch', 'lantern', 'ruin_wall',
    'tree_jade_canopy', 'tree_sage_elder', 'tree_windswept_oak', 'tree_cypress_spire',
    'tree_river_pine', 'rock_stratified_outcrop', 'boulder_moss_tall',
    'boulder_moss_flat', 'fern_cluster', 'grass_fan', 'flowering_shrub',
    'river_reeds', 'fallen_log', 'rooted_stump',
}
COMPONENTS = {5120: ('b', 1), 5121: ('B', 1), 5122: ('h', 2),
              5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}
WIDTHS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4,
          'MAT2': 4, 'MAT3': 9, 'MAT4': 16}
IDENTITY = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def finite(values):
    return all(isinstance(v, (float, int)) and math.isfinite(v) for v in values)


def matmul(a, b):
    return tuple(sum(a[k * 4 + row] * b[col * 4 + k] for k in range(4))
                 for col in range(4) for row in range(4))


def transform(m, p):
    return tuple(sum(m[k * 4 + row] * p[k] for k in range(3)) + m[12 + row]
                 for row in range(3))


def node_matrix(node):
    if 'matrix' in node:
        require(not any(k in node for k in ('translation', 'rotation', 'scale')),
                'Node mixes matrix and TRS')
        m = node['matrix']
        require(len(m) == 16 and finite(m), 'Invalid node matrix')
        require(all(abs(m[i]) < 1e-6 for i in (3, 7, 11)) and abs(m[15] - 1) < 1e-6,
                'Node matrix is not affine')
        return m
    t, q, s = node.get('translation', [0, 0, 0]), node.get('rotation', [0, 0, 0, 1]), node.get('scale', [1, 1, 1])
    require(len(t) == 3 and len(q) == 4 and len(s) == 3 and finite(t + q + s), 'Invalid node TRS')
    require(abs(sum(v * v for v in q) - 1) < 1e-4, 'Non-unit node quaternion')
    require(all(abs(v) > 1e-12 for v in s), 'Singular node scale')
    x, y, z, w = q
    r = (1-2*y*y-2*z*z, 2*x*y+2*z*w, 2*x*z-2*y*w, 0,
         2*x*y-2*z*w, 1-2*x*x-2*z*z, 2*y*z+2*x*w, 0,
         2*x*z+2*y*w, 2*y*z-2*x*w, 1-2*x*x-2*y*y, 0,
         t[0], t[1], t[2], 1)
    return tuple(r[i] * s[i // 4] if i < 12 else r[i] for i in range(16))


class GLB:
    def __init__(self, path):
        data = path.read_bytes()
        require(len(data) >= 20, 'Truncated GLB')
        magic, version, length = struct.unpack_from('<4sII', data)
        require(magic == b'glTF' and version == 2 and length == len(data), 'Invalid GLB header/length')
        chunks, offset = [], 12
        while offset < len(data):
            require(offset + 8 <= len(data), 'Truncated chunk header')
            size, kind = struct.unpack_from('<II', data, offset)
            offset += 8
            require(size % 4 == 0 and offset + size <= len(data), 'Invalid chunk length/alignment')
            chunks.append((kind, data[offset:offset + size]))
            offset += size
        require(len(chunks) == 2 and chunks[0][0] == 0x4E4F534A and chunks[1][0] == 0x004E4942,
                'Expected one JSON chunk followed by one BIN chunk')
        self.doc = json.loads(chunks[0][1])
        self.binary = chunks[1][1]
        self.cache = {}
        d = self.doc
        require(d.get('asset', {}).get('version') == '2.0', 'Invalid glTF asset version')
        require(len(d.get('buffers', [])) == 1, 'Expected one embedded buffer')
        require('uri' not in d['buffers'][0], 'External buffer URI')
        require(0 <= len(self.binary) - d['buffers'][0]['byteLength'] <= 3, 'BIN length mismatch')
        for v in d.get('bufferViews', []):
            require(v.get('buffer', 0) == 0 and v.get('byteOffset', 0) >= 0 and v['byteLength'] > 0,
                    'Invalid buffer view')
            require(v.get('byteOffset', 0) + v['byteLength'] <= d['buffers'][0]['byteLength'], 'Buffer view outside buffer')
        for img in d.get('images', []):
            require('uri' not in img and 'bufferView' in img, 'Image is not embedded')
            require(0 <= img['bufferView'] < len(d.get('bufferViews', [])), 'Invalid embedded image view')
        require(not d.get('cameras'), 'Camera included in game export')
        require('KHR_lights_punctual' not in d.get('extensions', {}), 'Light included in game export')
        for i in range(len(d.get('accessors', []))):
            self.accessor(i)
        triangles = 0
        for mesh in d.get('meshes', []):
            for p in mesh['primitives']:
                require(p.get('mode', 4) == 4, 'Non-triangle mesh primitive')
                require('POSITION' in p.get('attributes', {}), 'Primitive has no positions')
                positions = self.accessor(p['attributes']['POSITION'])
                require(positions and len(positions[0]) == 3, 'Invalid position accessor')
                for attr, ai in p['attributes'].items():
                    require(len(self.accessor(ai)) == len(positions), 'Mismatched ' + attr + ' attribute count')
                indices = self.indices(p, len(positions))
                require(len(indices) % 3 == 0, 'Triangle index count not divisible by three')
                require(all(0 <= i < len(positions) for i in indices), 'Triangle index out of range')
                require('material' in p and 0 <= p['material'] < len(d.get('materials', [])), 'Missing/invalid primitive material')
                triangles += len(indices) // 3
        require(triangles > 0, 'GLB contains no triangles')
        nodes = d.get('nodes', [])
        parents = Counter(c for n in nodes for c in n.get('children', []))
        require(all(0 <= c < len(nodes) and count == 1 for c, count in parents.items()), 'Invalid/multiple node parents')
        for n in nodes:
            node_matrix(n)
            require('camera' not in n and 'KHR_lights_punctual' not in n.get('extensions', {}), 'Presentation node in export')
            if 'mesh' in n:
                require(0 <= n['mesh'] < len(d['meshes']), 'Invalid node mesh')
        require(d.get('scenes') and 0 <= d.get('scene', 0) < len(d['scenes']), 'Missing/invalid default scene')
        self.world_nodes()  # Detect cycles and invalid default-scene roots.
        self.summary = {'bytes': len(data), 'meshes': len(d.get('meshes', [])),
                        'nodes': len(nodes), 'materials': len(d.get('materials', [])),
                        'accessors': len(d.get('accessors', [])), 'unique_mesh_triangles': triangles}

    def accessor(self, index):
        if index in self.cache:
            return self.cache[index]
        require(isinstance(index, int) and 0 <= index < len(self.doc.get('accessors', [])), 'Invalid accessor index')
        a = self.doc['accessors'][index]
        require('sparse' not in a, 'Sparse accessors unsupported by this export contract')
        require(a.get('componentType') in COMPONENTS and a.get('type') in WIDTHS, 'Unknown accessor type')
        require(a.get('count', 0) > 0 and 'bufferView' in a, 'Empty/unbacked accessor')
        require(0 <= a['bufferView'] < len(self.doc['bufferViews']), 'Invalid accessor view')
        v = self.doc['bufferViews'][a['bufferView']]
        fmt, size = COMPONENTS[a['componentType']]
        width = WIDTHS[a['type']]
        stride = v.get('byteStride', width * size)
        offset = a.get('byteOffset', 0)
        require(offset >= 0 and offset % size == 0 and stride >= width * size and stride % size == 0,
                'Invalid accessor stride/alignment')
        require(offset + (a['count'] - 1) * stride + width * size <= v['byteLength'], 'Accessor outside view')
        start = v.get('byteOffset', 0) + offset
        unpack = struct.Struct('<' + fmt * width).unpack_from
        values = [unpack(self.binary, start + i * stride) for i in range(a['count'])]
        if a['componentType'] == 5126:
            require(all(finite(row) for row in values), 'Non-finite FLOAT accessor')
        for key in ('min', 'max'):
            if key in a:
                require(len(a[key]) == width and finite(a[key]), 'Invalid accessor bounds')
        self.cache[index] = values
        return values

    def indices(self, primitive, count):
        if 'indices' not in primitive:
            return list(range(count))
        a = self.doc['accessors'][primitive['indices']]
        require(a['type'] == 'SCALAR' and a['componentType'] in (5121, 5123, 5125), 'Invalid index accessor')
        return [i[0] for i in self.accessor(primitive['indices'])]

    def world_nodes(self):
        result, seen = [], set()
        nodes = self.doc.get('nodes', [])
        def visit(index, parent, stack):
            require(isinstance(index, int) and 0 <= index < len(nodes), 'Invalid scene node')
            require(index not in stack and index not in seen, 'Node cycle or repeated scene traversal')
            seen.add(index)
            node = nodes[index]
            world = matmul(parent, node_matrix(node))
            result.append((node, world))
            for child in node.get('children', []):
                visit(child, world, stack | {index})
        for index in self.doc['scenes'][self.doc.get('scene', 0)].get('nodes', []):
            visit(index, IDENTITY, set())
        require(result, 'Empty default scene')
        return result

    def geometry_blender(self):
        triangles, points = [], []
        for node, world in self.world_nodes():
            if 'mesh' not in node:
                continue
            for p in self.doc['meshes'][node['mesh']]['primitives']:
                xyz = [transform(world, v) for v in self.accessor(p['attributes']['POSITION'])]
                verts = [(v[0], -v[2], v[1]) for v in xyz]
                points.extend(verts)
                ids = self.indices(p, len(verts))
                triangles.extend(tuple(verts[j] for j in ids[i:i+3]) for i in range(0, len(ids), 3))
        return points, triangles

    def node_points_blender(self, node, world):
        """Actual instantiated mesh positions, including all ancestor transforms."""
        points = []
        if 'mesh' in node:
            for primitive in self.doc['meshes'][node['mesh']]['primitives']:
                for point in self.accessor(primitive['attributes']['POSITION']):
                    x, y, z = transform(world, point)
                    points.append((x, -z, y))
        return points


def assembled_river_geometry(glb, expected_width=18.0, tolerance=1e-4):
    """Measure saved world geometry, independently of manifest width claims.

    The channel's perpendicular coordinate is (Blender Y - X) / sqrt(2).
    Tapered island-tip vertices may be inside the corridor. The main surface and
    shallows must nevertheless reach both exact banks, and all upper bank-edge
    vertices must lie on their corresponding +/- half-width datum. Deep bank
    buttresses intentionally flare outward below the visible channel wall.
    """
    half = expected_width / 2
    groups = {'channel': [], 'shallows': [], 'banks': [], 'meadows': []}
    for node, world in glb.world_nodes():
        name = node.get('name', '')
        group = ('channel' if name == 'River / turquoise channel' else
                 'shallows' if name.startswith('River / jade shallows') else
                 'banks' if name.startswith('Landscape / eroded bank ') else
                 'meadows' if name.startswith('Landscape / meadow ') else None)
        if group:
            groups[group].append((name, glb.node_points_blender(node, world)))
    result = {'expected_width_m': expected_width, 'tolerance_m': tolerance,
              'status': 'FAIL', 'errors': [], 'objects': {}}
    def check(condition, message):
        if not condition:
            result['errors'].append(message)
    check(len(groups['channel']) == 1, 'Expected one named turquoise channel mesh')
    check(len(groups['shallows']) == 2, 'Expected two named jade shallows meshes')
    check(len(groups['banks']) == 2, 'Expected two named eroded bank meshes')
    check(len(groups['meadows']) == 2, 'Expected two named meadow meshes')
    for group, entries in groups.items():
        combined = []
        for name, points in entries:
            check(bool(points), 'Named river geometry has no mesh vertices: ' + name)
            if not points:
                continue
            signed = [(point[1] - point[0]) / math.sqrt(2) for point in points]
            combined.extend(signed)
            result['objects'][name] = {'vertices': len(points),
                                       'perpendicular_min_m': min(signed),
                                       'perpendicular_max_m': max(signed)}
            if group in ('channel', 'shallows'):
                check(all(abs(t) <= half + tolerance for t in signed),
                      name + ': water vertices extend outside the declared river corridor')
            elif group == 'banks':
                # Upper edge and vertical exposed wall should meet the water
                # edge; only the deeper buttress is permitted to flare outward.
                wall = [t for point, t in zip(points, signed) if point[2] >= -1.6001]
                side = -1 if name.endswith(' -1') else 1
                check(bool(wall), name + ': no upper bank wall geometry')
                deviation = max((abs(t - side * half) for t in wall), default=math.inf)
                result['objects'][name]['upper_wall_max_deviation_m'] = deviation
                check(deviation <= tolerance, name + ': actual upper bank edge is not on the exact river-width datum')
                check(all(t * side >= half - tolerance for t in signed),
                      name + ': bank geometry intrudes into the river corridor')
            elif group == 'meadows':
                side = -1 if name.endswith(' -1') else 1
                near = min(t * side for t in signed)
                check(near >= half - tolerance, name + ': meadow geometry intrudes into the river corridor')
                check(abs(near - half) <= tolerance, name + ': meadow does not meet its river-bank datum')
        if group in ('channel', 'shallows') and combined:
            check(abs(min(combined) + half) <= tolerance and abs(max(combined) - half) <= tolerance,
                  group + ': actual world-space width differs from ' + str(expected_width) + ' m')
    result['status'] = 'PASS' if not result['errors'] else 'FAIL'
    return result


def segment_distance(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    denominator = dx * dx + dy * dy
    t = max(0, min(1, ((p[0]-a[0])*dx + (p[1]-a[1])*dy) / denominator)) if denominator else 0
    return math.hypot(p[0]-a[0]-t*dx, p[1]-a[1]-t*dy)


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1]) - (b[1]-a[1])*(c[0]-a[0])


def edge_distance(a, b, c, d):
    if cross(a,b,c)*cross(a,b,d) < 0 and cross(c,d,a)*cross(c,d,b) < 0:
        return 0
    return min(segment_distance(a,c,d), segment_distance(b,c,d),
               segment_distance(c,a,b), segment_distance(d,a,b))


def in_triangle(p, t):
    if abs(cross(*t)) < 1e-9:
        return False
    signs = [cross(t[i], t[(i+1)%3], p) for i in range(3)]
    return min(signs) >= -1e-9 or max(signs) <= 1e-9


def triangle_segment_distance(t, a, b):
    if in_triangle(a, t) or in_triangle(b, t):
        return 0
    return min(edge_distance(t[i], t[(i+1)%3], a, b) for i in range(3))


def expected_layout():
    h = 225 / math.sqrt(2) / 2
    half, home, away = h + 29, [-h, h], [h, -h]
    edge = half - 12
    lanes = [[home, away], [home, [-edge,home[1]], [-edge,-edge], [edge,-edge], [away[0],-edge], away],
             [home, [home[0],edge], [-edge,edge], [edge,edge], [edge,away[1]], away]]
    def sample(lane, f):
        lengths = [math.dist(a,b) for a,b in zip(lane,lane[1:])]
        distance = sum(lengths)*f
        for a,b,length in zip(lane,lane[1:],lengths):
            if distance <= length:
                return [a[i]+(b[i]-a[i])*distance/length for i in range(2)]
            distance -= length
        return lane[-1]
    return {'half_extent':half, 'base_centers_blender':[home,away], 'base_pad_size':46,
            'base_pad_height':.7, 'ramp_length':6, 'lane_width':12, 'lanes_blender':lanes,
            'river_width':18, 'river_endpoints_blender':[[-edge,-edge],[edge,edge]],
            'camps_blender':[[-half*2*.34,-half*2*.22],[half*2*.34,half*2*.22],[-half*2*.22,half*2*.34]],
            'bosses_blender':[[half*2*.22,half*2*.34],[-half*2*.22,-half*2*.34]],
            'towers_blender':[sample(l,t) for l in lanes for t in (.3,.7)]}


def compare_numbers(a, b):
    if isinstance(b, list):
        return isinstance(a, list) and len(a) == len(b) and all(compare_numbers(x,y) for x,y in zip(a,b))
    return isinstance(a, (int,float)) and math.isfinite(a) and abs(a-b) < 1e-5


def clearances(manifest, geometry, warnings):
    layout = manifest['layout']
    lanes = [(a,b) for line in layout['lanes_blender'] for a,b in zip(line,line[1:])]
    result = {'checked_placements':0, 'failures':[], 'tight_bounds':[]}
    for p in manifest['placements']:
        if p.get('role') not in ('forest_tree','forest_rock'):
            continue
        result['checked_placements'] += 1
        xy, angle, scale = p['position_blender'], p['rotation_z'], p['scale']
        points, triangles = geometry[p['asset']]
        radius = max(math.hypot(v[0],v[1]) for v in points) * scale
        ca,sa = math.cos(angle),math.sin(angle)
        def place(v):
            return (xy[0]+scale*(ca*v[0]-sa*v[1]), xy[1]+scale*(sa*v[0]+ca*v[1]))
        tests = [('lane', a,b,layout['lane_width']/2) for a,b in lanes]
        tests += [('river',layout['river_endpoints_blender'][0],layout['river_endpoints_blender'][1],layout['river_width']/2)]
        tests += [('camp',c,c,12) for c in layout['camps_blender']]
        tests += [('boss',c,c,12) for c in layout['bosses_blender']]
        tests += [('tower',c,c,5) for c in layout['towers_blender']]
        nearby = [(name,a,b,r) for name,a,b,r in tests if segment_distance(xy,a,b) < r+radius]
        world_triangles = [tuple(place(v) for v in t) for t in triangles] if nearby else []
        for name,a,b,r in nearby:
            actual = min(triangle_segment_distance(t,a,b) for t in world_triangles)
            if actual < r - .01:
                result['failures'].append({'placement':p['name'], 'feature':name,
                                           'penetration_m':round(r-actual,4)})
            else:
                result['tight_bounds'].append({'placement':p['name'],'feature':name,
                                               'actual_clearance_m':round(actual-r,4)})
        # Base pad + walk-up ramp is the square used by the documented exclusion.
        base_half = layout['base_pad_size']/2 + layout['ramp_length']
        for center in layout['base_centers_blender']:
            if max(abs(xy[i]-center[i]) for i in range(2)) >= base_half+radius:
                continue
            placed = [place(v) for v in points]
            if any(max(abs(v[i]-center[i]) for i in range(2)) < base_half-.01 for v in placed):
                result['failures'].append({'placement':p['name'],'feature':'base_pad_and_ramp'})
            else:
                result['tight_bounds'].append({'placement':p['name'],'feature':'base_pad_and_ramp'})
        if max(abs(xy[i]) for i in range(2)) + radius > layout['half_extent'] - 3:
            warnings.append('Forest bounds close to terrain edge: ' + p['name'])
    require(result['checked_placements'] > 0, 'No forest placements to validate')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--skip-assimp', action='store_true')
    args = parser.parse_args()
    report = {'checked_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'root':str(ROOT), 'status':'FAIL', 'errors':[], 'warnings':[], 'files':{}}
    try:
        manifest = json.loads((ROOT/'manifest.json').read_text())
        expected = expected_layout()
        for key,value in expected.items():
            require(compare_numbers(manifest['layout'].get(key),value), 'Layout formula mismatch: ' + key)
        report['layout_formulas'] = 'PASS'
        require(manifest.get('units') == '1 Blender meter = 1 game unit', 'Unexpected units')
        assets = manifest['assets']
        require(len(assets) == 24 and {a['id'] for a in assets} == EXPECTED, 'Expected exactly 24 named reusable assets')
        source = ROOT/manifest['source']
        require(source.name == 'verdant-confluence.blend' and source.stat().st_size > 1000, 'Missing/empty source .blend')
        with source.open('rb') as handle:
            header = handle.read(12)
        source_encoding = 'uncompressed'
        if header[:2] == b'\x1f\x8b':
            with gzip.open(source,'rb') as handle:
                header = handle.read(12)
            source_encoding = 'gzip'
        elif header[:4] == b'\x28\xb5\x2f\xfd':
            source_encoding = 'zstandard'
            zstd = shutil.which('zstd')
            if zstd:
                unpacked = subprocess.run([zstd,'-d','-c',str(source)],capture_output=True,timeout=180)
                require(unpacked.returncode == 0, 'Source .blend Zstandard decompression failed')
                header = unpacked.stdout[:12]
            else:
                report['warnings'].append('Recognized Zstandard .blend container; zstd CLI unavailable. '
                                          'Full source readability must be verified by Blender load/render evidence.')
        require(header.startswith(b'BLENDER') or (source_encoding == 'zstandard' and header[:4] == b'\x28\xb5\x2f\xfd'),
                'Invalid/unrecognized .blend header')
        report['source_blend'] = {'bytes':source.stat().st_size,'encoding':source_encoding,
                                  'header_hex':header.hex(), 'blender_header_verified':header.startswith(b'BLENDER')}
        paths = [a['path'] for a in assets] + [manifest['assembled_glb']]
        require(manifest['assembled_glb'] == 'exports/verdant-confluence.glb', 'Unexpected assembled export name')
        require(len(set(paths)) == 25, 'Duplicate or missing GLB paths')
        actual_paths = {str(p.relative_to(ROOT)) for folder in ('library','exports') for p in (ROOT/folder).glob('*.glb')}
        require(actual_paths == set(paths), 'GLB files do not match manifest')
        geometry = {}
        assimp = shutil.which('assimp') if not args.skip_assimp else None
        if assimp is None:
            report['warnings'].append('Independent assimp import was skipped or executable is unavailable')
        for rel in paths:
            try:
                path = ROOT/rel
                require(path.resolve().is_relative_to(ROOT), 'Export path escapes art root')
                glb = GLB(path)
                report['files'][rel] = glb.summary
                if rel == manifest['assembled_glb']:
                    report['actual_river_geometry'] = assembled_river_geometry(glb, manifest['layout']['river_width'])
                    report['errors'].extend('Assembled river: ' + error
                                            for error in report['actual_river_geometry']['errors'])
                if rel.startswith('library/'):
                    key = path.stem
                    asset = next(a for a in assets if a['id'] == key)
                    require(rel == 'library/'+key+'.glb', 'Asset name/path mismatch')
                    points, triangles = glb.geometry_blender()
                    require(points, 'No instantiated library geometry')
                    actual = {'min':[min(v[i] for v in points) for i in range(3)],
                              'max':[max(v[i] for v in points) for i in range(3)]}
                    for bound in ('min','max'):
                        require(compare_numbers(actual[bound],asset['bounds_blender'][bound]), 'Recorded bounds mismatch: '+key)
                    dimensions = [actual['max'][i]-actual['min'][i] for i in range(3)]
                    require(compare_numbers(dimensions,asset['dimensions']), 'Recorded dimensions mismatch: '+key)
                    if key == 'bridge':
                        # The authored ground datum is at the bank/deck approach;
                        # abutments and arch fascia intentionally descend below it.
                        require(actual['min'][2] < 0 < actual['max'][2], 'Bridge does not straddle its ground datum')
                        report['files'][rel]['pivot_convention'] = 'ground approach datum; subgrade abutments'
                    else:
                        require(abs(actual['min'][2]) < .15, 'Asset ground pivot too far from geometry: '+key)
                    require(len(triangles) == asset['triangles'], 'Recorded triangle count mismatch: '+key)
                    geometry[key] = (points,triangles)
                if assimp:
                    check = subprocess.run([assimp,'info',str(path)],capture_output=True,text=True,timeout=180)
                    require(check.returncode == 0, 'assimp import failed: '+check.stderr[-1000:]+check.stdout[-1000:])
                    report['files'][rel]['assimp'] = 'PASS'
            except (ValueError,KeyError,TypeError,IndexError,OSError,struct.error,subprocess.TimeoutExpired) as error:
                report['errors'].append(rel+': '+str(error))
        report['placements_by_role'] = dict(Counter(p.get('role') for p in manifest['placements']))
        require(len({p['name'] for p in manifest['placements']}) == len(manifest['placements']), 'Duplicate placement names')
        for p in manifest['placements']:
            require(p['asset'] in EXPECTED and finite(p['position_blender']) and len(p['position_blender']) == 3,
                    'Invalid placement asset/position')
            require(finite([p['rotation_z'],p['scale']]) and p['scale'] > 0, 'Invalid placement transform')
        if set(geometry) == EXPECTED:
            report['clearances'] = clearances(manifest,geometry,report['warnings'])
            if report['clearances']['tight_bounds']:
                report['warnings'].append(str(len(report['clearances']['tight_bounds'])) +
                                          ' conservative radius clearances are tight; actual triangles were tested')
            report['errors'].extend('Forest intersects '+f['feature']+': '+f['placement']
                                    for f in report['clearances']['failures'])
        else:
            report['errors'].append('Clearance validation incomplete because some library geometry is invalid')
    except (ValueError,KeyError,TypeError,IndexError,OSError,subprocess.TimeoutExpired) as error:
        report['errors'].append(str(error))
    report['status'] = 'PASS' if not report['errors'] else 'FAIL'
    output = json.dumps(report,indent=2)+'\n'
    if args.output:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(output)
    print(output,end='')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
