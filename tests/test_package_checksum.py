import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'package_checksum', Path(__file__).resolve().parents[1] / 'scripts/package_checksum.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ChecksumTests(unittest.TestCase):
    def test_roundtrip_and_corruption(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder) / 'package with spaces.zip'
            package.write_bytes(b'abc')
            sidecar = module.checksum(package)
            self.assertEqual(sidecar.read_text(),
                'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  package with spaces.zip\n')
            module.checksum(package, verify=True)
            package.write_bytes(b'corrupted')
            with self.assertRaisesRegex(ValueError, 'mismatch'):
                module.checksum(package, verify=True)

    def test_empty_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder) / 'empty.apk'
            package.touch()
            module.checksum(package)
            module.checksum(package, verify=True)
            with self.assertRaises(FileExistsError):
                module.checksum(package)

    def test_wrong_name_and_malformed_digest(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder) / 'package.zip'
            package.write_bytes(b'abc')
            sidecar = module.checksum(package)
            for content in ['0' * 64 + '  other.zip\n', 'invalid  package.zip\n']:
                sidecar.write_text(content)
                with self.assertRaisesRegex(ValueError, 'Malformed'):
                    module.checksum(package, verify=True)


if __name__ == '__main__':
    unittest.main()
