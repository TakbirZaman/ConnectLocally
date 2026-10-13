#!/usr/bin/env python3
"""Build the optional Android view-only APK; retain files and screen viewing."""
import pathlib
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET

ANDROID = "{http://schemas.android.com/apk/res/android}"
SERVICE = ".RemoteAccessibilityService"

def remove_phone_input(manifest_path):
    ET.register_namespace("android", ANDROID[1:-1])
    tree = ET.parse(manifest_path)
    application = tree.getroot().find("application")
    if application is None:
        raise ValueError("Android application missing")
    for service in list(application.findall("service")):
        if service.get(ANDROID + "name") in (
            SERVICE, "com.example.localconnect.RemoteAccessibilityService"
        ):
            application.remove(service)
    application.set(ANDROID + "label", "LocalConnect View Only")
    tree.write(manifest_path, encoding="utf-8", xml_declaration=True)

def main():
    root = pathlib.Path(__file__).resolve().parents[1]
    subprocess.run([sys.executable, str(root / "scripts/install_android.py")], cwd=root, check=True)
    manifest = root / "android/app/src/main/AndroidManifest.xml"
    original = manifest.read_bytes()
    try:
        remove_phone_input(manifest)
        subprocess.run(["flutter", "build", "apk", "--debug", "--target-platform", "android-arm64"], cwd=root, check=True)
        source = root / "build/app/outputs/flutter-apk/app-debug.apk"
        destination = root / "dist/LocalConnect-Android-ViewOnly.apk"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        print(destination)
    finally:
        # The full-control source runner remains the default for subsequent builds.
        manifest.write_bytes(original)

if __name__ == "__main__":
    main()
