#!/usr/bin/env python3
"""Reject debug/wrong-publisher APKs; record exact release identity. Requires SDK tools."""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path


def fingerprint(value):
    normalized = value.replace(':', '').strip().lower()
    if not re.fullmatch(r'[0-9a-f]{64}', normalized):
        raise ValueError('Expected signer must be a complete SHA-256 certificate fingerprint')
    return normalized


def inspect(signing, badging, expected_signer, expected_package):
    expected = fingerprint(expected_signer)
    signers = re.findall(r'^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F:]+)$', signing, re.M)
    if len(signers) != 1 or fingerprint(signers[0]) != expected:
        raise ValueError('APK signer does not match the single approved publisher certificate')
    if re.search(r'^Signer #\d+ certificate DN:.*(?:Android Debug|CN=androiddebugkey)', signing, re.M | re.I):
        raise ValueError('Android debug certificate cannot be used for a production candidate')
    if re.search(r'^application-debuggable(?:\s|$)', badging, re.M):
        raise ValueError('Debuggable APK cannot be used for a production candidate')
    package = re.search(r"^package: name='([^']+)' versionCode='(\d+)' versionName='([^']*)'", badging, re.M)
    if package is None or package[1] != expected_package:
        raise ValueError('Unexpected or missing Android application identity')
    return {'package': package[1], 'version_code': int(package[2]),
            'version_name': package[3], 'signer_sha256': expected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('apk', type=Path)
    parser.add_argument('--apksigner', required=True)
    parser.add_argument('--aapt2', required=True)
    parser.add_argument('--expected-signer', required=True)
    parser.add_argument('--expected-package', required=True)
    parser.add_argument('--commit', required=True)
    args = parser.parse_args()
    fingerprint(args.expected_signer)
    if not re.fullmatch(r'[0-9a-f]{40}', args.commit):
        parser.error('commit must be the complete checked source SHA')
    # check=True means failed signature verification can never produce metadata.
    signing = subprocess.run([args.apksigner, 'verify', '--verbose', '--print-certs', str(args.apk)],
                             check=True, capture_output=True, text=True).stdout
    badging = subprocess.run([args.aapt2, 'dump', 'badging', str(args.apk)],
                             check=True, capture_output=True, text=True).stdout
    data = inspect(signing, badging, args.expected_signer, args.expected_package)
    with args.apk.open('rb') as source:
        data['apk_sha256'] = hashlib.file_digest(source, 'sha256').hexdigest()
    data.update(source_commit=args.commit, size_bytes=args.apk.stat().st_size,
                schema_version=1, google_approval='not_established_by_this_check')
    output = Path(str(args.apk) + '.release.json')
    with output.open('x') as target:
        json.dump(data, target, indent=2)
        target.write('\n')
    print('Verified release identity:', output)


if __name__ == '__main__':
    main()
