#!/usr/bin/env python3
"""Create labeled native-QA clipless VRM0/VRM1 GLBs; never change a source skin."""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def prepare(source, destination):
    original = source.read_bytes()
    length = struct.unpack_from('<I', original, 12)[0]
    document = json.loads(original[20:20+length])
    remainder = original[20+length:]
    output = []
    destination.mkdir(parents=True, exist_ok=True)
    for version in [0, 1]:
        doc = json.loads(json.dumps(document))
        doc.pop('animations', None)
        doc.setdefault('asset', {}).setdefault('extras', {})['omoba_motion_qa'] = 'Clipless fixture, actual shipped CC0 mesh; no Studio ownership claim'
        if version == 1:
            old = doc['extensions'].pop('VRM')
            bones = {}
            for entry in old['humanoid']['humanBones']:
                name = entry['bone']
                if name.endswith('ThumbProximal'):
                    name = name.replace('ThumbProximal', 'ThumbMetacarpal')
                elif name.endswith('ThumbIntermediate'):
                    name = name.replace('ThumbIntermediate', 'ThumbProximal')
                bones[name] = {'node': entry['node']}
            doc['extensions']['VRMC_vrm'] = {'specVersion': '1.0', 'humanoid': {'humanBones': bones},
                'meta': {'name': 'Omoba CC0 clipless motion fixture', 'version': '1', 'authors': ['ToxSam'],
                    'licenseUrl': 'https://creativecommons.org/publicdomain/zero/1.0/'}}
            for i, node in enumerate(doc['nodes']):
                # Duplicate names deliberately prove node-index rather than name binding.
                node['name'] = 'arbitrary_joint' if i % 2 else f'renamed_{i}'
            for field in ['extensionsUsed', 'extensionsRequired']:
                if field in doc:
                    doc[field] = ['VRMC_vrm' if e == 'VRM' else e for e in doc[field]]
        encoded = json.dumps(doc, separators=(',', ':')).encode()
        encoded += b' ' * (-len(encoded) % 4)
        blob = struct.pack('<4sII', b'glTF', 2, 20+len(encoded)+len(remainder)) + struct.pack('<I4s', len(encoded), b'JSON') + encoded + remainder
        path = destination / f'clipless-vrm{version}.glb'
        path.write_bytes(blob)
        output.append({'path': str(path), 'sha256': hashlib.sha256(blob).hexdigest(), 'bytes': len(blob), 'animations': 0, 'version': version})
    assert source.read_bytes() == original
    report = {'source': str(source), 'source_sha256': hashlib.sha256(original).hexdigest(), 'source_unchanged': True, 'fixtures': output}
    (destination / 'fixture-manifest.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1]/'client/assets/avatars/agnes.glb')
    args = parser.parse_args()
    print(json.dumps(prepare(args.source, args.destination)))
