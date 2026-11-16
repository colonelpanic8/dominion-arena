import tempfile
import unittest
from pathlib import Path
from finalize_web import finalize


class FinalizeWebTest(unittest.TestCase):
    def test_versions_final_script_content_and_keeps_preload_aligned(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'assets').mkdir()
            script = root / 'assets/app-dxh.js'
            script.write_text('load("first.wasm")')
            index = root / 'index.html'
            index.write_text('<link href="/./assets/app-dxh.js"><script src="/./assets/app-dxh.js"></script>')
            finalize(root)
            first = index.read_text()
            self.assertEqual(first.count('?v='), 2)
            self.assertEqual(first.split('href="')[1].split('"')[0], first.split('src="')[1].split('"')[0])
            finalize(root)
            self.assertEqual(index.read_text(), first)
            script.write_text('load("replacement.wasm")')
            finalize(root)
            self.assertNotEqual(index.read_text(), first)
            self.assertEqual(script.read_text(), 'load("replacement.wasm")')


if __name__ == '__main__':
    unittest.main()
