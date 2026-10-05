import unittest
from stage_verdant import ART, read_glb, flatten_sanctuary, articulate_nexus_rings

class NexusRingsTest(unittest.TestCase):
    def test_original_faces_materials_and_three_pivots_survive_split(self):
        for team in ('green', 'blue'):
            doc, binary = read_glb(ART / 'library' / f'sanctuary_{team}.glb')
            flatten_sanctuary(doc, binary)
            def face_count():
                return sum(doc['accessors'][p['indices']]['count'] for mesh in doc['meshes'] for p in mesh['primitives'])
            before = face_count()
            articulate_nexus_rings(doc, binary)
            self.assertEqual(before, face_count())
            rings = [n for n in doc['nodes'] if n.get('name', '').startswith('NexusOrbit-')]
            self.assertEqual(len(rings), 3)
            for ring in rings:
                self.assertAlmostEqual(ring['translation'][1], 7.33, places=4)
                self.assertAlmostEqual(ring['translation'][0], 0, places=4)
                self.assertAlmostEqual(ring['translation'][2], 0, places=4)
                for primitive in doc['meshes'][ring['mesh']]['primitives']:
                    position = doc['accessors'][primitive['attributes']['POSITION']]
                    self.assertAlmostEqual(position['min'][1], -position['max'][1], places=4)

if __name__ == '__main__':
    unittest.main()
