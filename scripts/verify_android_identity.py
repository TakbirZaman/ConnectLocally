#!/usr/bin/env python3
"""Verify installed package and launch activity in the built APK, not just Gradle."""
import re
import subprocess
import sys

APPLICATION_ID = "io.github.takbirzaman.localconnect"
ACTIVITY = "com.example.localconnect.MainActivity"


def verify(badging):
    package = re.search(r"^package: name='([^']+)'", badging, re.M)
    activity = re.search(r"^launchable-activity: name='([^']+)'", badging, re.M)
    if package is None or package[1] != APPLICATION_ID:
        raise ValueError("Built APK has an unexpected application ID")
    if activity is None or activity[1] != ACTIVITY:
        raise ValueError("Built APK cannot resolve the expected Kotlin launch activity")


def main():
    if len(sys.argv) != 3:
        raise SystemExit("Usage: verify_android_identity.py APK AAPT2")
    result = subprocess.run([sys.argv[2], "dump", "badging", sys.argv[1]],
                            check=True, capture_output=True, text=True)
    verify(result.stdout)
    print("Verified APK install identity and launch activity:", APPLICATION_ID)


if __name__ == "__main__":
    main()
