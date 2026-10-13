#!/usr/bin/env python3
"""Check the packaged view-only manifest, not Google approval or malware status."""
import argparse
import subprocess
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("apk")
parser.add_argument("aapt2")
args = parser.parse_args()
manifest = subprocess.check_output(
    [args.aapt2, "dump", "xmltree", "--file", "AndroidManifest.xml", args.apk],
    text=True,
)
for forbidden in ("BIND_ACCESSIBILITY_SERVICE", "android.accessibilityservice.AccessibilityService",
                  "RemoteAccessibilityService", "READ_SMS", "RECEIVE_SMS",
                  "BIND_NOTIFICATION_LISTENER_SERVICE"):
    if forbidden in manifest:
        raise SystemExit("View-only APK declares forbidden capability: " + forbidden)
for required in ("TransferService", "ProjectionService", "MainActivity"):
    if required not in manifest:
        raise SystemExit("View-only APK missing required component: " + required)
print("Packaged manifest: phone input service absent; files and capture services retained.")
