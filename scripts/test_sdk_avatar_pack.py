"""Catalog serving boundaries; SDK/model validation is covered by native proof."""
import json
from pathlib import Path
import tempfile
import unittest
from urllib.error import HTTPError
from urllib.request import urlopen

from sdk_avatar_pack import start_catalog


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.pack = Path(self.temporary.name)
        (self.pack / 'files').mkdir()
        (self.pack / 'files/a.glb').write_bytes(b'local-model')
        (self.pack / 'files/a.png').write_bytes(b'local-preview')
        self.item = dict(id='local-one', name='One', access='free', thumbnailPath='/files/a.png',
                         renditions=[dict(assetPath='/files/a.glb')])
        (self.pack / 'catalog.json').write_text(json.dumps(dict(items=[self.item])))
        (self.pack / 'private.txt').write_text('must not be served')
        self.server, self.thread = start_catalog(self.pack)
        self.base = f'http://127.0.0.1:{self.server.server_port}'

    def tearDown(self):
        self.server.shutdown(); self.server.server_close(); self.thread.join()
        self.temporary.cleanup()

    def read(self, path):
        with urlopen(self.base + path, timeout=2) as response:
            return response.read()

    def test_exact_profile_and_loopback_download_urls(self):
        catalog = json.loads(self.read('/v2/avatars?project=omoba&platform=desktop&profile=humanoid-glb-v1'))
        self.assertEqual(catalog['count'], 1)
        item = catalog['items'][0]
        self.assertEqual(item['thumbnailUrl'], self.base + '/files/a.png')
        self.assertEqual(item['renditions'][0]['downloadUrl'], self.base + '/files/a.glb')
        self.assertNotIn('assetPath', item['renditions'][0])
        self.assertEqual(self.read('/files/a.glb'), b'local-model')
        for query in ('project=other', 'profile=handheld-glb-v1', 'project=omoba&project=other'):
            self.assertEqual(json.loads(self.read('/v2/avatars?' + query))['items'], [])

    def test_only_allowlisted_files_are_served(self):
        for path in ('/private.txt', '/files/../private.txt', '/files/%2e%2e/private.txt', '/catalog.json'):
            with self.assertRaises(HTTPError) as caught:
                self.read(path)
            self.assertEqual(caught.exception.code, 404)

    def test_withdrawn_item_and_its_files_are_not_served(self):
        server, thread = start_catalog(self.pack, excluded_ids=['local-one'])
        try:
            with urlopen(f'http://127.0.0.1:{server.server_port}/v2/avatars', timeout=2) as r:
                self.assertEqual(json.load(r)['count'], 0)
            with self.assertRaises(HTTPError):
                urlopen(f'http://127.0.0.1:{server.server_port}/files/a.glb', timeout=2)
        finally:
            server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__':
    unittest.main()
