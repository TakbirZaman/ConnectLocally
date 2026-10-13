#!/usr/bin/env python3
"""Check ZIP CRC, matching ABIs and 64-bit ELF alignment; not signing/runtime."""
import argparse
import hashlib
import struct
import zipfile
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('apk', type=Path)
args = parser.parse_args()
with zipfile.ZipFile(args.apk) as archive:
    bad = archive.testzip()
    if bad is not None:
        raise SystemExit(f'Corrupt ZIP entry: {bad}')
    names = set(archive.namelist())
    flutter = {name.split('/')[1] for name in names if name.startswith('lib/') and name.endswith('/libflutter.so')}
    rust = {name.split('/')[1] for name in names if name.startswith('lib/') and name.endswith('/liblocalconnect_core.so')}
    if not flutter or flutter != rust:
        raise SystemExit(f'ABI mismatch: Flutter={sorted(flutter)}, Rust={sorted(rust)}')
    if 'AndroidManifest.xml' not in names:
        raise SystemExit('Android manifest missing')
    for name in sorted(names):
        if not name.endswith('.so') or not name.startswith(('lib/arm64-v8a/', 'lib/x86_64/')):
            continue
        with archive.open(name) as library:
            header = library.read(64)
            if len(header) != 64 or header[:6] != b'\x7fELF\x02\x01':
                raise SystemExit(f'Invalid 64-bit little-endian ELF: {name}')
            offset = struct.unpack_from('<Q', header, 32)[0]
            size, count = struct.unpack_from('<HH', header, 54)
            if size < 56 or size > 1024 or count > 1024:
                raise SystemExit(f'Invalid ELF program headers: {name}')
            library.seek(offset)
            loads = 0
            for _ in range(count):
                record = library.read(size)
                if len(record) != size:
                    raise SystemExit(f'Truncated ELF: {name}')
                kind, _, file_offset, address, _, _, _, alignment = struct.unpack_from('<IIQQQQQQ', record)
                if kind == 1:
                    loads += 1
                    if alignment < 16384 or file_offset % 16384 != address % 16384:
                        raise SystemExit(f'16 KiB ELF alignment failed: {name}')
            if loads == 0:
                raise SystemExit(f'ELF load segments missing: {name}')
print(f'ZIP CRC and matching native ABIs passed: {", ".join(sorted(flutter))}')
print('64-bit ELF 16 KiB alignment passed; device/runtime compatibility still requires testing.')
with args.apk.open('rb') as source:
    print(f'SHA-256: {hashlib.file_digest(source, "sha256").hexdigest()}')
