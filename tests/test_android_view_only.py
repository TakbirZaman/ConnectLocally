import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("view_only", ROOT / "scripts/build_android_view_only.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class ViewOnlyManifestTests(unittest.TestCase):
    def test_removes_input_without_removing_files_capture_or_backup_policy(self):
        source = ROOT / "android/app/src/main/AndroidManifest.xml"
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "AndroidManifest.xml"
            target.write_bytes(source.read_bytes())
            original = ET.parse(target).getroot()
            original_permissions = {p.get(module.ANDROID + "name") for p in original.findall("uses-permission")}
            module.remove_phone_input(target)
            module.remove_phone_input(target)
            updated = ET.parse(target).getroot()
            app = updated.find("application")
            services = {s.get(module.ANDROID + "name") for s in app.findall("service")}
            self.assertNotIn(".RemoteAccessibilityService", services)
            self.assertIn(".TransferService", services)
            self.assertIn(".ProjectionService", services)
            self.assertEqual(app.get(module.ANDROID + "allowBackup"), "false")
            self.assertEqual(app.get(module.ANDROID + "label"), "LocalConnect View Only")
            self.assertEqual(original_permissions, {p.get(module.ANDROID + "name") for p in updated.findall("uses-permission")})
            self.assertNotIn("BIND_ACCESSIBILITY_SERVICE", target.read_text())
            self.assertNotIn("android.accessibilityservice.AccessibilityService", target.read_text())
