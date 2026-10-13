#!/usr/bin/env python3
"""Refresh ONLY Rust libraries in an existing debug APK. Not a Gradle build.

Use only when Dart, generated FRB bindings, Kotlin, resources and dependencies
are unchanged. A new signing key cannot update an installed APK signed by the
old key. Keep the existing debug keystore if installation/state must be retained.
Prerequisites: pinned Rust 1.88.0, matching Android Rust targets, NDK 27.0.12077973,
build-tools 35.0.0, JDK, and an explicit test keystore. Never use for releases.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile


def run(args, env=None):
    subprocess.run([str(x) for x in args], env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['base', 'output', 'ndk', 'build-tools', 'keystore']:
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--alias', default='androiddebugkey')
    parser.add_argument('--password-env', default='LOCALCONNECT_DEBUG_KEY_PASSWORD')
    parser.add_argument('--cargo', default='cargo')
    args = parser.parse_args()
    if not os.environ.get(args.password_env):
        parser.error('Set the specified keystore password environment variable')
    if args.base.resolve() == args.output.resolve():
        parser.error('Base and output must differ')
    root = Path(__file__).resolve().parents[1]
    tool = args.ndk.resolve() / 'toolchains/llvm/prebuilt/linux-x86_64/bin'
    if '27.0.12077973' not in (args.ndk / 'source.properties').read_text():
        parser.error('Expected NDK 27.0.12077973')
    targets = {'arm64-v8a': 'aarch64-linux-android',
               'x86': 'i686-linux-android', 'x86_64': 'x86_64-linux-android',
               'armeabi-v7a': 'armv7-linux-androideabi'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='localconnect-apk-') as temporary:
        work = Path(temporary)
        workaround = work / 'gcc'
        workaround.mkdir()
        (workaround / 'libgcc.a').write_text('INPUT(-lunwind)')
        replacements = {}
        with zipfile.ZipFile(args.base) as original:
            for name in original.namelist():
                if not name.endswith('/liblocalconnect_core.so'):
                    continue
                abi = name.split('/')[1]
                target = targets[abi]
                clang_target = 'armv7a-linux-androideabi' if abi == 'armeabi-v7a' else target
                compiler = tool / (clang_target + '26-clang')
                env = os.environ.copy()
                env.update(CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR=str(work / 'target'),
                           RUSTFLAGS='-L ' + str(workaround))
                env.pop('CARGO_ENCODED_RUSTFLAGS', None)
                env['CARGO_TARGET_' + target.upper().replace('-', '_') + '_LINKER'] = str(compiler)
                env['CC_' + target] = str(compiler)
                env['AR_' + target] = str(tool / 'llvm-ar')
                run([args.cargo, '+1.88.0', 'build', '--manifest-path', root / 'rust/Cargo.toml',
                     '--lib', '--locked', '--target', target], env)
                library = work / 'target' / target / 'debug/liblocalconnect_core.so'
                symbols = subprocess.check_output([tool / 'llvm-nm', '-D', library], text=True)
                if 'frb_pde_ffi_dispatcher_primary' not in symbols:
                    raise RuntimeError('Native library is missing FRB exports')
                run([tool / 'llvm-strip', '--strip-unneeded', library])
                replacements[name] = library
            if not replacements:
                raise RuntimeError('Base APK contains no LocalConnect Rust library')
            unsigned = work / 'unsigned.apk'
            with zipfile.ZipFile(unsigned, 'w') as updated:
                for info in original.infolist():
                    # Discard old JAR signatures; APK v2 signing block is not a ZIP entry.
                    if info.filename.startswith('META-INF/') and (
                        info.filename.upper().endswith(('.SF', '.RSA', '.DSA', '.EC'))
                        or info.filename.upper() == 'META-INF/MANIFEST.MF'):
                        continue
                    if info.filename in replacements:
                        updated.write(replacements[info.filename], info.filename, compress_type=info.compress_type)
                    else:
                        with original.open(info) as src, updated.open(info, 'w') as dst:
                            shutil.copyfileobj(src, dst, 1024 * 1024)
        aligned = work / 'aligned.apk'
        run([args.build_tools / 'zipalign', '-P', '16', '-f', '4', unsigned, aligned])
        run([args.build_tools / 'apksigner', 'sign', '--ks', args.keystore,
             '--ks-key-alias', args.alias, '--ks-pass', 'env:' + args.password_env,
             '--key-pass', 'env:' + args.password_env, '--out', args.output, aligned])
        run([args.build_tools / 'apksigner', 'verify', '--verbose', args.output])
        run([args.build_tools / 'zipalign', '-c', '-P', '16', '4', args.output])
        print('Refreshed debug APK:', args.output)


if __name__ == '__main__':
    main()
