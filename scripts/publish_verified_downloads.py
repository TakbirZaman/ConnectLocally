#!/usr/bin/env python3
"""Publish only the three immutable, hash-pinned beta binaries already tested in CI."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = "TakbirZaman/ConnectLocally"
TAG = "v0.2.1-beta.2"
TARGET = "9a40720bb93163fe4912ff055faf0f914d49fddb"
FILES = [
    {
        "run": 39804962188, "source": "3c33fd5a7630b3e398cbca49029e2d7345693b76",
        "artifact": "linux-x64", "input": "LocalConnect-Linux-x64.tar.gz",
        "output": "LocalConnect-Linux-x64-0.2.1.tar.gz",
        "sha256": "ab105696be666e9b6d87da73ef2041b2066d6c75695f77515cc42e1f86548cec",
    },
    {
        "run": 39804962188, "source": "3c33fd5a7630b3e398cbca49029e2d7345693b76",
        "artifact": "windows-x64", "input": "LocalConnect-Windows-x64-Setup.exe",
        "output": "LocalConnect-Windows-x64-Setup-0.2.1.exe",
        "sha256": "7bc98e295f10fc46cc2abb9ae8c7e7641f7b10da5b81407d6986da7cfd1bebfe",
    },
    {
        "run": 38627084934, "source": "8e8cbff353e3a6d8654f1f230d26c8541a9b5088",
        "artifact": "android-arm64-release-candidate-8e8cbff353e3a6d8654f1f230d26c8541a9b5088",
        "input": "app-release.apk", "output": "LocalConnect-Android-arm64-Signed-0.2.0.apk",
        "sha256": "94afe2f0dec74709f3b8ff7410b520772b83b57cbbc39415737051e6b46087a9",
    },
]
NOTES = """Beta packages for testing; these are not a certified production release.

- Windows 0.2.1: install once, remembered receive folder, Open received files. The installer is not publisher-signed.
- Linux 0.2.1: x64 tar.gz built and tested in Ubuntu 24.04 CI. Requires compatible GTK 3/runtime libraries; screen capture additionally needs the documented X11 or Wayland dependencies. Extract and run LocalConnect/localconnect.\n- Android 0.2.0: ARM64, owner release-signed, application ID io.github.takbirzaman.localconnect.
- Versions differ intentionally: the Windows usability update does not change the LAN protocol. Android signing keys are not uploaded.
- Google approval is not established. Follow Android's fresh capture/Accessibility consent prompts.
- PNG streaming is limited to up to 4 fps and one desktop display.

Windows CI: https://github.com/TakbirZaman/ConnectLocally/actions/runs/39804962188
Android CI: https://github.com/TakbirZaman/ConnectLocally/actions/runs/38627084934

SHA256SUMS.txt contains exact hashes. Close the old Windows app before installing; launch from Start afterward.
"""


def gh(*args, check=True):
    return subprocess.run(["gh", *args], check=check, capture_output=True, text=True)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    with tempfile.TemporaryDirectory(prefix="localconnect-downloads-") as temporary:
        root = Path(temporary)
        publish = root / "publish"
        publish.mkdir()
        for item in FILES:
            run = json.loads(gh("api", f"repos/{REPO}/actions/runs/{item['run']}").stdout)
            if run["conclusion"] != "success" or run["head_sha"] != item["source"]:
                raise RuntimeError("CI result/source mismatch")
            stage = root / str(item["run"])
            gh("run", "download", str(item["run"]), "--repo", REPO,
               "--name", item["artifact"], "--dir", str(stage))
            candidates = list(stage.rglob(item["input"]))
            if len(candidates) != 1 or candidates[0].is_symlink():
                raise RuntimeError("Expected exactly one regular binary")
            source = candidates[0]
            if digest(source) != item["sha256"]:
                raise RuntimeError("Pinned binary checksum mismatch")
            if item["input"].endswith(".apk"):
                metadata = json.loads(Path(str(source) + ".release.json").read_text())
                expected = {
                    "apk_sha256": item["sha256"], "source_commit": item["source"],
                    "package": "io.github.takbirzaman.localconnect",
                    "signer_sha256": "bb3d3046a89857ddafa41ffc47dd349755cbead93dc58d7b5ea38162f9c7b72c",
                }
                if any(metadata.get(key) != value for key, value in expected.items()):
                    raise RuntimeError("Android publisher metadata mismatch")
            shutil.copyfile(source, publish / item["output"])
        checksums = publish / "SHA256SUMS.txt"
        checksums.write_text("".join(
            f"{item['sha256']}  {item['output']}\n" for item in FILES
        ))
        notes = root / "notes.md"
        notes.write_text(NOTES)
        # Never overwrite an existing release asset. A rerun must match every byte.
        releases = json.loads(gh("api", "--paginate", "--slurp",
                               f"repos/{REPO}/releases?per_page=100").stdout)
        existing = [release for page in releases for release in page if release["tag_name"] == TAG]
        assets = sorted(publish.iterdir())
        if existing:
            if existing[0]["draft"] or not existing[0]["prerelease"]:
                raise RuntimeError("Existing tag is not the expected public beta")
            current = root / "existing"
            gh("release", "download", TAG, "--repo", REPO, "--dir", str(current))
            for asset in assets:
                if not (current / asset.name).is_file() or digest(current / asset.name) != digest(asset):
                    raise RuntimeError("Existing release differs; manual review required")
            print("Existing public beta assets match verified files.")
        else:
            gh("release", "create", TAG, *[str(asset) for asset in assets],
               "--repo", REPO, "--target", TARGET, "--prerelease",
               "--title", "LocalConnect beta — Windows, Linux and Android",
               "--notes-file", str(notes))
            print("Published verified Windows and Android beta downloads.")


if __name__ == "__main__":
    main()
