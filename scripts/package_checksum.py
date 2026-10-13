#!/usr/bin/env python3
"""Write/verify a SHA-256 sidecar without loading the package into memory.

Checksums detect corruption; obtain the checksum through a trusted channel.
They do not authenticate a publisher or replace platform signing.
"""
import argparse
import hashlib
import hmac
from pathlib import Path


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def checksum(path: Path, verify: bool = False) -> Path:
    if not path.is_file() or path.is_symlink():
        raise ValueError('Package must be a regular file, not a symlink')
    if any(char in path.name for char in '\r\n\\'):
        raise ValueError('Package filename cannot contain newlines or backslashes')
    sidecar = path.with_name(path.name + '.sha256')
    actual = digest(path)
    if verify:
        line = sidecar.read_text(encoding='utf-8')
        expected, separator, name = line.rstrip('\n').partition('  ')
        if (separator != '  ' or name != path.name or len(expected) != 64
                or any(char not in '0123456789abcdef' for char in expected)):
            raise ValueError('Malformed checksum or mismatched package filename')
        if not hmac.compare_digest(actual, expected):
            raise ValueError('SHA-256 mismatch; do not install this package')
    else:
        with sidecar.open('x', encoding='utf-8', newline='\n') as output:
            output.write(f'{actual}  {path.name}\n')
    return sidecar


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    try:
        result = checksum(args.package, args.verify)
    except (OSError, ValueError) as error:
        parser.exit(1, f'Checksum failed: {error}\n')
    print(f'{"Verified" if args.verify else "Created"}: {result}')


if __name__ == '__main__':
    main()
