import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("identity", ROOT / "scripts/verify_android_identity.py")
identity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identity)


class AndroidIdentityTests(unittest.TestCase):
    def test_separate_install_id_resolves_existing_native_activity(self):
        identity.verify("package: name='io.github.takbirzaman.localconnect' versionCode='2'\n"
                        "launchable-activity: name='com.example.localconnect.MainActivity' label='LocalConnect'\n")

    def test_rejects_old_id_missing_activity_and_wrong_class(self):
        for badging in [
            "package: name='com.example.localconnect'\nlaunchable-activity: name='com.example.localconnect.MainActivity'",
            "package: name='io.github.takbirzaman.localconnect'",
            "package: name='io.github.takbirzaman.localconnect'\nlaunchable-activity: name='io.github.takbirzaman.localconnect.MainActivity'",
            "",
        ]:
            with self.subTest(badging=badging), self.assertRaises(ValueError):
                identity.verify(badging)
