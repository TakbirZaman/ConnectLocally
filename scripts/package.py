#!/usr/bin/env python3
"""Package source or an existing host-native Flutter bundle; never builds/signs."""
import argparse, tarfile, zipfile
from pathlib import Path
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source',action='store_true');parser.add_argument('--linux',action='store_true');parser.add_argument('--windows',action='store_true')
parser.add_argument('--output',type=Path)
args=parser.parse_args()
if sum([args.source,args.linux,args.windows])!=1:parser.error('Choose one package type')
output=args.output or root/'dist'/('LocalConnect-Source.zip' if args.source else 'LocalConnect-Linux-x64.tar.gz' if args.linux else 'LocalConnect-Windows-x64.zip')
output=output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
if args.source:
    excludes={'build','target','.dart_tool','.gradle','.plugin_symlinks','.symlinks','.git','dist','ephemeral'}
    secrets={'local.properties','key.properties','identity.der','identity.key','identity.key.sealed','state.sqlite','state.sqlite-shm','state.sqlite-wal'}
    suffixes={'.keystore','.jks','.so','.dll','.exe','.a','.apk','.zip','.tar','.gz','.log','.pyc','.p12','.pfx','.pem','.key','.db','.sqlite','.sqlite3'}
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as archive:
        for file in sorted(root.rglob('*')):
            relative=file.relative_to(root)
            if file.is_symlink() or not file.is_file() or excludes.intersection(relative.parts) or '__pycache__' in relative.parts:continue
            if file.name in secrets or file.suffix.lower() in suffixes:continue
            if any(part.lower() in {'.aws','.ssh','.gnupg','secrets'} for part in relative.parts):continue
            if file.name.lower()=='.env' or file.name.lower().startswith('.env.'):continue
            if file.name.lower().endswith(('.sqlite-wal','.sqlite-shm','.sqlite-journal','.key.sealed')):continue
            archive.write(file,'localconnect/'+relative.as_posix())
    print(output)
elif args.linux:
    bundle=root/'build/linux/x64/release/bundle'
    if not (bundle/'localconnect').is_file():parser.error('Run flutter build linux --release on Linux first')
    with tarfile.open(output,'w:gz') as archive:archive.add(bundle,arcname='LocalConnect')
    print(output)
else:
    bundle=root/'build/windows/x64/runner/Release'
    if not (bundle/'localconnect.exe').is_file():parser.error('Run flutter build windows --release on Windows first')
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED) as archive:
        for file in sorted(bundle.rglob('*')):
            if file.is_file():archive.write(file,'LocalConnect/'+file.relative_to(bundle).as_posix())
    print(output)

