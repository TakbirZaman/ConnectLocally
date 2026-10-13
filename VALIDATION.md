# Validation evidence — 2026-10-13

This report separates executed checks from source implementation. The latest
source uses Rust 1.88.0, Flutter 3.29.3/Dart 3.7.2, FRB 2.11.1, and committed
Cargo.lock/pubspec.lock. Tests were executed on Linux x86_64.

## Current revision: checks actually executed

- Full default-feature `cargo test --locked`: **26 passed**, one live X11 test
  intentionally ignored in the normal suite (17 unit, 8 network, 1 remote security).
- Bridge-only/headless Rust suite: **26 passed**; the live X11 test is ignored.
- Full default-feature `cargo clippy --locked --all-targets -- -D warnings`:
  **passed**. Generated FRB code retains its documented generated-code lint allowance.
- `cargo build --locked --lib`: **passed**, producing the current Linux native library.
- `python3 tests/run_x11.py`: **passed** on an authenticated isolated Xvfb display.
  It verifies explicit approval, TLS PNG streaming, physical pointer movement,
  held Shift state, and release of Shift on Stop. It never controls a user desktop.
- Current Windows GNU Rust cross-check: **passed**, `cargo check --locked
  --target x86_64-pc-windows-gnu --lib`. This is not a Windows MSVC app build.
- Opt-in large-file test: **passed**, actual TLS transfer of **4,294,967,313 bytes**,
  final streamed SHA-256 verified. Sender and receiver share one test process;
  sampled RSS baseline **20,271,104 bytes**, peak **62,337,024 bytes**, increase
  **42,065,920 bytes**. Runtime **32.50 seconds** includes transfer/final hash.
  This used optimized SHA-256 in an otherwise debug test build, loopback and local
  disk. It is a single workload measurement, not an Android/Windows benchmark or
  a guarantee for all configurations. The initial unoptimized run was cancelled
  before completion; it is not a passed test.
- Strict clippy for the new large-file test: **passed** after one format lint fix.
- Dart dependency resolution with `pub get --enforce-lockfile`: **passed**.
- Dart formatting: **passed**.
- `flutter analyze --no-pub`: **passed**, no issues found after fixing three
  flow-control style findings in the latest UI changes.
- `flutter test --no-pub --dart-define=LOCALCONNECT_NATIVE_LIB=...`: **5 passed**,
  including actual native FRB initialization/unpaired-send rejection/shutdown,
  three state regressions and the Stop widget regression.
- `flutter build linux --debug --no-pub`: **passed** for the current complete
  application, including native Rust/Cargokit and the C++ runner. Cargokit also
  needed a fresh temporary target cache and a verified missing Dart dependency.
- The bundled native library exports `localconnect_shutdown`; dynamic dependency
  inspection found the required GStreamer/XCB libraries and no missing entries.
- Current Linux bundle startup smoke: **passed**, eight seconds on an isolated
  authenticated X11 TCP display with no unhandled exception. An Atk accessibility
  embedding warning occurred. This test performs no UI actions and terminates
  the process; it does not establish graceful window-close resource cleanup.
- Android overlay installation into the actual Flutter runner: **passed**.
  This establishes installation of source files, not Kotlin compilation.
  Android bitmap conversion and display-resize failure cleanup compiled in the
  current full APK build below. Device behavior remains untested.

The Rust tests cover oversized/invalid frame rejection before allocation,
canonical transfer IDs, unsafe and recovery-reserved names, source mutation,
empty files, collisions, >4 GiB metadata, corrupted and duplicate chunks,
reconciliation after restart, refusal to acknowledge failed SQLite commits,
trusted TLS pairing and end-to-end verified transfer, unpaired bulk and remote
rejection, independent control during stalled bulk, interrupted-transfer recovery,
policy persistence, and stale native lifecycle generation rejection.
Network tests use real rustls, SQLite and temporary files over loopback.
They do not establish physical WLAN, mDNS multicast or power-loss behavior.

## Build environment observations

The restored workspace target cache produced invalid/empty object metadata.
Validation was repeated successfully in a fresh temporary Cargo target directory
with dev/test debug information disabled; this is a build-cache workaround, not
an application code change. The reproducible equivalent is:

```sh
export CARGO_TARGET_DIR=/tmp/localconnect-validation-target
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
cargo test --locked --manifest-path rust/Cargo.toml
cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings
python3 tests/run_x11.py
cargo build --locked --manifest-path rust/Cargo.toml --lib
flutter analyze --no-pub
flutter test --no-pub --dart-define=LOCALCONNECT_NATIVE_LIB=/tmp/localconnect-validation-target/debug/liblocalconnect_core.so
```

Use an environment without DISPLAY/WAYLAND_DISPLAY for the bridge capability
assertions. Use the synthetic X11 script for actual capture/input assertions.

## Previous checkpoints, not latest-build evidence

Earlier source checkpoints passed Windows GNU Rust cross-check, Android ARM64
Rust cross-check, Linux Flutter debug build/startup smoke, Flutter state/Stop
widget tests and actual FRB library loading. Those results precede the latest
lifecycle/JNI/UI changes and are not current complete platform builds.
The previously delivered Android APK was a Milestone 1 build. It does **not**
contain the latest Android projection/accessibility UI and services. Native-only
APK refresh is invalid when Kotlin, manifest, Dart or bridge contracts change.
The new full Gradle/Flutter/NDK build below supersedes that historical package.
It is a debug test build, not a certified release.
The included CI workflow has not been executed in an external repository.

## Required real-device acceptance checks

1. Two private-LAN devices: deny pairing, compare complete fingerprints, approve
   both sides, forget trust, verify all new bulk/input requests fail after removal.
2. Empty/small/multi-chunk/>4 GiB files: independently compare SHA-256 after
   completion and Android SAF export. Measure RSS as file size grows.
3. Wi-Fi loss, DHCP changes and process force-kill/restart: resume only missing or
   corrupt chunks; mutate/remove the source and verify actionable failure.
4. Fault injection at data sync, SQLite commit, ACK and final publication; real
   ENOSPC/read-only filesystem tests. Never ACK non-durable recovery state.
5. Saturated transfer: heartbeats and input remain responsive; check configured
   window, connection limits, slow peers and bounded PNG queues.
6. Windows: full MSVC/Flutter build, DPI/rotation, controlled-session approval,
   held-input release, UAC/secure desktop restrictions, Credential Manager.
7. Real X11: scaling and primary-display behavior. Wayland: GNOME/KDE portal
   grant/denial, PipeWire node loss, input revocation, Stop and compositor shutdown.
8. Android API 26/33/34/35+: fresh MediaProjection consent, rotation, projection
   revocation, manually enabled/disabled Accessibility, notification denial, SAF
   provider failure, foreground-service startup restrictions, Doze and OEM settings.
   Check wake/multicast lock release and no automatic capture restart.
9. Stop/disconnect during startup, capture and transfers; check sockets, temporary
   frames, held input, projection resources and generation-bound JNI callbacks.
10. Build/sign/install current apps on each target. Use a protected stable signing
    key; do not confuse successful compilation with device behavior.

No physical Android/Windows/Wayland tests, real power loss, or full-disk test has
been run. The large-file/RSS result above is the current executed local workload.

## Opt-in large-file acceptance command

Requires at least 5 GiB free disk space. The source fixture is sparse, but the
receiver writes all bytes durably. This is intentionally excluded from normal CI.

```sh
cargo test --locked --manifest-path rust/Cargo.toml \
  --config 'profile.test.package.sha2.opt-level=3' \
  --test large_transfer -- --ignored --nocapture
```

## Current Android full build and package verification

- `flutter build apk --debug --target-platform android-arm64 --no-pub`: **passed**
  with JDK 17.0.20 including javac, Gradle 8.10.2, AGP 8.7.3, Kotlin 2.1.0, SDK 35,
  NDK 27.0.12077973 and CMake 3.22.1. Current Dart, Kotlin, resources, manifest
  and Rust compiled; this is not a native-only refresh.
- ARM64, i686 and x86_64 Rust builds passed during build validation. The final
  APK deliberately contains only requested ARM64 Flutter/Rust native libraries.
- APK inspection exposed and fixed extra debug ABIs without Flutter engines.
  Cargokit now follows the requested platform list, removes old output ABIs, and
  the app's abiFilters prevent stale AGP intermediates entering the package.
- 64-bit Rust ELF initially had 4 KiB PT_LOAD alignment. Cargokit now passes
  16 KiB max/common-page-size linker arguments for Android ARM64/x86_64.
- Final `scripts/verify_android_apk.py`: **passed**, ZIP CRC, matching ARM64 ABI,
  and 16 KiB ELF alignment for all three packaged native libraries.
- `apksigner verify --verbose`: **passed**, v2 signature; `zipalign -c -P 16 4`:
  **passed**. These are packaging checks, not a 16 KiB-device runtime test.
- Packaged library exports FRB and generation/session-bound JNI cleanup symbols.
  Manifest identifies version 0.2.0 (2), min SDK 26, target SDK 35 and registered
  transfer/projection/accessibility services with accessibility binding permission.
- APK bytes: **92,941,000**. SHA-256:
  `9f84fca027ecbbd6af9b9851ba0cee40014609ff782d7b013a7a47bf075ff4e1`.
- A new debug key was generated; no release key is included.

Initial attempts failed on missing javac, stale environment proxy configuration,
missing manually restored NDK package metadata, and the packaging/alignment
issues above. They were corrected and the final complete build/inspection passed.
No Android device installation, projection consent, accessibility behavior,
SAF/OEM/lifecycle tests, or production signing is claimed.
