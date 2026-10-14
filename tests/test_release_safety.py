import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/verify_release_apk.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseIdentityTests(unittest.TestCase):
    signer = 'ab' * 32
    badge = "package: name='com.example.localconnect' versionCode='2' versionName='0.2.0'\n"

    def signing(self, digest=None, name='CN=LocalConnect Publisher'):
        return f'Signer #1 certificate DN: {name}\nSigner #1 certificate SHA-256 digest: {digest or self.signer}\n'

    def check(self, signing=None, badge=None, expected=None):
        return release.inspect(signing or self.signing(), badge or self.badge,
                               expected or self.signer, 'com.example.localconnect')

    def test_approved_identity(self):
        result = self.check(expected=':'.join(['AB'] * 32))
        self.assertEqual(result['signer_sha256'], self.signer)
        self.assertEqual(result['version_code'], 2)

    def test_wrong_signer(self):
        with self.assertRaises(ValueError): self.check(expected='cd' * 32)

    def test_incomplete_fingerprint(self):
        with self.assertRaises(ValueError): self.check(expected='abcd')

    def test_debug_certificate(self):
        with self.assertRaises(ValueError): self.check(signing=self.signing(name='CN=Android Debug,O=Android,C=US'))

    def test_debuggable_application(self):
        with self.assertRaises(ValueError): self.check(badge=self.badge + 'application-debuggable\n')

    def test_wrong_application(self):
        with self.assertRaises(ValueError): self.check(badge=self.badge.replace('com.example.localconnect', 'other.app'))

    def test_missing_signer(self):
        with self.assertRaises(ValueError): self.check(signing='Verifies\n')

    def test_additional_signer(self):
        with self.assertRaises(ValueError): self.check(signing=self.signing() + self.signing().replace('#1', '#2'))


class SourceArchiveTests(unittest.TestCase):
    def test_sensitive_files_never_enter_source_archive(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'scripts').mkdir()
            shutil.copyfile(ROOT/'scripts/package.py', root/'scripts/package.py')
            files = ['lib/main.dart', 'README.md', 'pubspec.lock', 'assets/logo.png',
                     '.env', '.env.production', 'config/.ENV.local', 'signing/owner.p12',
                     'signing/owner.PFX', 'identity.key.sealed', 'identity.key',
                     'private.pem', 'state.sqlite-wal', '.ssh/id_rsa', '.aws/credentials',
                     'secrets/password.txt', 'build/test.txt', 'target/test.txt']
            for name in files:
                path = root/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('fixture only')
            subprocess.run([sys.executable, str(root/'scripts/package.py'), '--source'], check=True, capture_output=True)
            with zipfile.ZipFile(root/'dist/LocalConnect-Source.zip') as archive:
                names = set(archive.namelist())
            self.assertEqual(names, {'localconnect/scripts/package.py', 'localconnect/lib/main.dart',
                                     'localconnect/README.md', 'localconnect/pubspec.lock',
                                     'localconnect/assets/logo.png'})


if __name__ == '__main__':
    unittest.main()
