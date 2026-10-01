#!/usr/bin/env python3
"""Build/serve a curated local developer catalog consumed through Ekza SDK.

This is an operator-reviewed local content pack, not a public Studio publication.
Source archive and the game's shipped avatar manifest are never modified.
"""
from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import threading
import time
from urllib.parse import parse_qs, urlparse
import uuid

from ekza_build_rendition import build
from ekza_publish import glb_document
from stage_avatars import sniff_image_extension
from validate_candidate_assets import embedded_conflicts

ROOT = Path(__file__).resolve().parents[1]
PROFILE = 'humanoid-glb-v1'
SELECTION = ROOT / 'assets-src/sdk-avatar-pack/selection.json'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def prepare(archive, output, selection=SELECTION):
    selected = json.loads(selection.read_text())
    if len(selected['items']) != 20 or len({i['source_id'] for i in selected['items']}) != 20:
        raise ValueError('The reviewed selection must contain 20 unique avatars')
    # Refuse to overwrite a prior pack or an unrelated directory.
    output.mkdir(parents=True, exist_ok=False)
    files = output / 'files'
    files.mkdir()
    entries, evidence = [], []
    shipped = json.loads((ROOT / 'client/assets/avatars/manifest.json').read_text())
    shipped_urls = {item['source_url'] for item in shipped['avatars']}
    for chosen in selected['items']:
        source = (archive / chosen['local_dir'] / 'model.vrm').resolve()
        source.relative_to(archive.resolve())
        if source.stat().st_size > 50 * 1024 * 1024:
            raise ValueError('Source exceeds profile limit')
        raw = source.read_bytes()
        if digest(raw) != chosen['source_sha256'] or chosen['source_url'] in shipped_urls:
            raise ValueError(f"Changed or already shipped source: {chosen['name']}")
        document = glb_document(raw)
        meta = document['extensions']['VRM']['meta']
        if chosen['license'] != 'CC0' or meta.get('licenseName') != 'CC0' or any(
            meta.get(key) != 'Allow' for key in ('violentUssageName', 'commercialUssageName')
        ) or embedded_conflicts(document):
            raise ValueError(f"Conflicting embedded permissions: {chosen['name']}")
        model, report = build(raw)
        if model is None:
            raise ValueError(f"{chosen['name']}: {report}")
        sha = digest(model)
        model_path = files / f'{sha}.glb'
        model_path.write_bytes(model)
        # The output of the real consumer-owned rendition builder is inspected.
        inspect = subprocess.run(['assimp', 'info', str(model_path)], capture_output=True, text=True, check=True)
        (output / f'assimp-{sha[:12]}.txt').write_text(inspect.stdout + inspect.stderr)
        thumb = source.parent / chosen['thumbnail']
        portrait = thumb.read_bytes()
        if digest(portrait) != chosen['thumbnail_sha256']:
            raise ValueError('Source thumbnail changed')
        thumb_name = f'{digest(portrait)}.{sniff_image_extension(thumb)}'
        (files / thumb_name).write_bytes(portrait)
        # A stable developer-catalog identity, never an invented production ID.
        avatar_id = 'ekza:avatar:' + str(uuid.uuid5(uuid.NAMESPACE_URL, 'omoba-local-osa-pack:' + chosen['source_id']))
        slug = 'ekza-' + digest((avatar_id + '\n' + sha).encode())
        entries.append(dict(
            id=avatar_id, name=chosen['name'], description='Local OSA SDK Twenty developer collection',
            access='free', creator={'name': chosen['author']},
            license={'text': 'CC0', 'attribution': f"{chosen['author']} · {chosen['collection']}"},
            thumbnailPath=f'/files/{thumb_name}',
            origin={'kind': 'local-developer-pack', 'sourceUrl': chosen['source_url']},
            renditions=[dict(platform='desktop', profile=PROFILE, format='glb', sha256=sha,
                             sizeBytes=len(model), assetPath=f'/files/{sha}.glb')],
            projectSupport=[dict(projectId='omoba', platform='desktop', profile=PROFILE, status='approved')],
        ))
        evidence.append(dict(**chosen, avatar_id=avatar_id, slug=slug, sha256=sha,
                             size_bytes=len(model), builder_report=report, embedded_permissions=meta))
        print(f"Prepared {len(entries)}/20: {chosen['name']}", flush=True)
    write_json(output / 'catalog.json', dict(schema='ekza.avatar.catalog.v2', count=20, items=entries))
    write_json(output / 'pack.json', dict(schema='omoba.local-sdk-pack.v1', public_studio=False,
                                       selection_sha256=digest(selection.read_bytes()), items=evidence))
    return evidence


def catalog_handler(pack, requests=None, excluded_ids=()):
    catalog = json.loads((pack / 'catalog.json').read_text())
    catalog['items'] = [i for i in catalog['items'] if i['id'] not in excluded_ids]
    # Allowlist exact files referenced by this catalog; no directory serving.
    allowed = {}
    for item in catalog['items']:
        for path in [item['thumbnailPath'], *[r['assetPath'] for r in item['renditions']]]:
            file = (pack / path.lstrip('/')).resolve()
            file.relative_to((pack / 'files').resolve())
            allowed[path] = file

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            parsed = urlparse(self.path)
            if requests is not None:
                requests.append(self.path)
            if parsed.path == '/v2/avatars':
                query = parse_qs(parsed.query)
                expected = {'project': ['omoba'], 'platform': ['desktop'], 'profile': [PROFILE]}
                items = []
                if all(k in expected and v == expected[k] for k, v in query.items()):
                    base = f'http://127.0.0.1:{self.server.server_port}'
                    for item in catalog['items']:
                        item = json.loads(json.dumps(item))
                        item['thumbnailUrl'] = base + item.pop('thumbnailPath')
                        for rendition in item['renditions']:
                            rendition['downloadUrl'] = base + rendition.pop('assetPath')
                        items.append(item)
                body = json.dumps(dict(schema='ekza.avatar.catalog.v2', count=len(items), items=items)).encode()
                mime = 'application/json'
            elif parsed.path in allowed:
                body = allowed[parsed.path].read_bytes()
                mime = 'model/gltf-binary' if parsed.path.endswith('.glb') else ('image/png' if parsed.path.endswith('.png') else 'image/jpeg')
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            self.wfile.write(body)
    return Handler


def start_catalog(pack, port=0, requests=None, excluded_ids=()):
    server = ThreadingHTTPServer(('127.0.0.1', port), catalog_handler(pack, requests, excluded_ids))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread


def launch(args):
    from capture_showcase import base_env, free_address, stop
    pack = args.pack.resolve()
    # Keep this collection's cache/settings separate from the public account.
    config = pack / 'player'
    config.mkdir(exist_ok=True)
    server, thread = start_catalog(pack)
    env = base_env(dict(size=(1280, 720), profile='desktop'), args.assets.resolve(), config, pack)
    host, port = free_address()
    env.update(OMOBA_REGISTRY_URL=f'http://127.0.0.1:{server.server_port}',
               SERVER_ADDR=f'{host}:{port}', GAME_SERVER_ADDR=f'{host}:{port}',
               OMOBA_MATCH_MODE='dev', OMOBA_LANGUAGE='en')
    children = []
    try:
        with (pack / 'play-server.log').open('w') as log:
            children.append(subprocess.Popen([str(args.server_bin.resolve())], env=env, stdout=log, stderr=subprocess.STDOUT))
        deadline = time.monotonic() + 15
        while 'is listening' not in (pack / 'play-server.log').read_text(errors='replace'):
            if children[0].poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Game server failed to start; see play-server.log')
            time.sleep(.05)
        print('Local SDK collection ready. Open Avatars → Studio, choose a character, then play.', flush=True)
        children.append(subprocess.Popen([str(args.client_bin.resolve())], env=env))
        children[-1].wait()
    finally:
        stop(children)
        server.shutdown()
        server.server_close()
        thread.join()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='command', required=True)
    build_p = sub.add_parser('build')
    build_p.add_argument('--archive', type=Path, required=True)
    build_p.add_argument('--output', type=Path, required=True)
    play = sub.add_parser('launch')
    play.add_argument('--pack', type=Path, required=True)
    play.add_argument('--client-bin', type=Path, required=True)
    play.add_argument('--server-bin', type=Path, required=True)
    play.add_argument('--assets', type=Path, default=ROOT / 'client/assets')
    args = p.parse_args()
    if args.command == 'build':
        prepare(args.archive, args.output)
    else:
        launch(args)


if __name__ == '__main__':
    main()
