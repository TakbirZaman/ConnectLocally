# LocalConnect 0.2 foundation

Commercial release preparation: read [Production release](docs/PRODUCTION_RELEASE.md)
for publisher verification, signing setup, and the remaining owner decisions.
Current downloadable debug APKs are test builds.

Assumptions: both devices are on a trusted local LAN/WLAN, users share a fresh six-digit pairing code with the intended device, and receive folders use a filesystem supporting hard links. No cloud signaling, relays, STUN or TURN are used by the application. Build tools download SDKs/dependencies separately from application operation.

“Zero-loss” means resumable transfer with cryptographic verification. It does not mean uninterrupted connections or permanent Android background execution.

This source includes the secure transfer milestone and remote-session adapters. Read VALIDATION.md for actual checks and REMAINING.md for work that still needs a platform or device. The supplied 0.2.0 ARM64 debug APK is a full current Flutter/Gradle/Kotlin/NDK build. Historical Milestone 1 APKs do not contain these changes.

## 1. Scaffolding commands

Use Flutter **3.29.3**, Dart **3.7.2**, Rust **1.88.0**, Python 3, and the host platform's compiler. Source runners and generated bridge files are included; no `flutter create` is needed to use this archive.

```sh
rustup toolchain install 1.88.0 --component rustfmt,clippy
flutter --version
flutter pub get --enforce-lockfile
python3 scripts/install_android.py
```

If scaffolding an empty directory before applying these source files:

```sh
flutter create --org com.example --project-name localconnect --platforms windows,linux,android .
```

Do not regenerate runners over the provided native window-close hooks. Android overlays are applied by `scripts/install_android.py`.

Linux Debian/Ubuntu development prerequisites:

```sh
sudo apt-get update
sudo apt-get install clang cmake ninja-build pkg-config libgtk-3-dev libxcb1-dev libxkbcommon-dev libdbus-1-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev xvfb xauth
```

Wayland runtime additionally requires a RemoteDesktop-capable xdg-desktop-portal backend, PipeWire, GStreamer `pipewiresrc`, `pngenc`, conversion/scale/rate plugins. Install the portal backend matching your compositor; X11 support does not provide Wayland input.

Windows: Visual Studio 2022 with Desktop development with C++, Windows SDK, Flutter Windows tooling, and Rust MSVC toolchain.

Android: JDK **17 including javac**, Android SDK platform **35**, build tools **35.0.0**, NDK **27.0.12077973**, CMake **3.22.1**. App minimum SDK **26**, target SDK **35**, AGP **8.7.3**, Kotlin **2.1.0**. Plugins may also install SDK/build-tools 34 for their own builds.

```sh
sdkmanager 'platforms;android-35' 'build-tools;35.0.0' 'ndk;27.0.12077973' 'cmake;3.22.1'
sdkmanager --licenses
flutter doctor -v
```

Direct dependencies are exact-pinned in `pubspec.yaml` and `rust/Cargo.toml`; transitive versions are in both lockfiles. FRB runtime and generator are **2.11.1**, exclusively v2.

Project tree:

```text
lib/                       Flutter Home, Riverpod state, remote viewer
lib/src/rust/              Generated FRB v2 bindings and typed models
rust/src/api.rs            Flutter commands and event stream
rust/src/engine.rs         Discovery, connections, transfer orchestration
rust/src/identity.rs        Persistent identity and TLS configurations
rust/src/protocol.rs        Wire framing and manifest validation
rust/src/store.rs           SQLite recovery/settings/trust
rust/src/transfer.rs        Chunk durability and final publication
rust/src/errors.rs          Structured failures and retry policy
rust/src/remote_session.rs  Consent, peer binding, rates, geometry
rust/src/remote_engine.rs   Independent control and screen TLS channels
rust/src/desktop.rs         Windows/X11 capture and input
rust/src/wayland.rs         Portal/PipeWire capture and portal input
rust/src/lifecycle.rs       Desktop host shutdown export
rust/src/android_jni.rs     Generation/session-bound Android cleanup
rust/migrations/            001 transfers/trust; 002 settings
rust/tests/                 TLS/transfer/remote integration tests
android-native/             Kotlin overlays, manifest additions, service XML
android/, linux/, windows/  Native Flutter runners
rust_builder/              Vendored Cargokit native build integration
scripts/                   Android overlay installer and packagers
.github/workflows/          Proposed build/test CI
```

Build on the target host:

```sh
flutter build linux --release
python3 scripts/package.py --linux
flutter build windows --release
python scripts/package.py --windows
flutter build apk --debug --target-platform android-arm64
```

Linux/Windows package commands use existing release bundles; they do not invent a successful build. Distribute the entire bundle. Linux requires compatible GTK/GStreamer runtime libraries.

Release Android signing uses your private `LOCALCONNECT_KEYSTORE`, `LOCALCONNECT_STORE_PASSWORD`, `LOCALCONNECT_KEY_ALIAS`, `LOCALCONNECT_KEY_PASSWORD` environment variables. No release key is included and release builds do not silently use debug signing. Preserve a development signing key to install updates without clearing app state.

## 2. Bridge definition

`rust/src/api.rs` is the actual bridge API. Commands initialize/shutdown the engine, probe/pair/approve/forget peers, connect, send/resume/cancel/relocate/clean transfers, configure saved limits, request/approve/stop remote sessions, and send geometry-tagged input. Android frame publication passes private filenames and metadata, not file-transfer chunks.

Typed models include `Peer`, `Event`, `TransferLimits`, `TransferProgress`, `FailureInfo`, `Geometry`, and `RemoteView`. Async Rust failures cross the v2 bridge as command errors; transfer history has structured recovery errors. `events()` uses a bounded broadcast channel and replays discovery, connection, and active-session snapshots after lag. Flutter restores transfer progress from SQLite.

```sh
cargo install flutter_rust_bridge_codegen --version 2.11.1 --locked
cargo install cargo-expand --version 1.0.113 --locked
flutter_rust_bridge_codegen generate
```

`lib/main.dart` initializes `RustLib`, calls generated `core` commands, and subscribes to events. File data stays in Rust, except Android SAF import/export uses native streaming to/from private staging. Screenshot decoding uses bounded PNG buffers in Flutter. This foundation does not claim universal bridge zero-copy behavior.

## 3. Rust core: networking and transfer

Discovery advertises `_localconnect._tcp.local.` with persistent certificate-derived device ID, name, wire version and port. It caches arrivals/removals/interface address changes, ignores itself, filters local IP ranges, and treats TXT metadata as untrusted. Manual numeric IP/port works when multicast is unavailable. IPv6 link-local manual addresses need a numeric scope; unscoped link-local discovery addresses are excluded.

IPv4 and a separate IPv6-only listener use the same port when IPv6 is available. Discovery may suggest updated endpoints; each alternative connection must prove the previously approved certificate fingerprint before it is used. Endpoint hints never grant trust.

Pairing bootstrap TLS verifies certificate-key possession but initially accepts an unknown self-signed certificate. In Devices, choose **Show pairing code** on one device; select it or enter its IP/port on the other and enter all six digits. Codes are random, single-use, expire after 120 seconds, and allow at most three attempts per displayed code. Generating another code has a 15-second cooldown. SPAKE2 (Ed25519 group) authenticates the short code; HMAC key confirmation binds both certificate identities and the TLS exporter to this connection. The pairing protocol does not transmit the raw code. Keep the displayed code private, including during screen sharing. Closing the code invalidates unfinished approval. Both devices must use this updated build.

Access is **session-only by default**, ending when the LocalConnect engine stops or the app process ends. This is an engine session, not necessarily one socket or one file transfer. Enable **Remember** on both devices for durable reconnect trust; each device controls its own persistence. Remote screen/input authorization remains separate. Advanced identity details show the certificate fingerprint, which is a cryptographic hash and needs no biometric scanner. The older explicit fingerprint API remains for compatible clients; discovery never grants trust. All file/control/video operations require an authenticated approved peer identity. This verifier is scoped to LocalConnect and must not be used for general HTTPS. Forgetting a peer revokes its transfers, control connection, and remote sessions.

Identity is persistent. Linux private state directory is mode 0700 and key mode 0600. Windows identity secrets use Credential Manager through `keyring`, migrating a private plaintext key once. Android uses app-private storage plus Android Keystore AES-GCM wrapping; Rust reads a transient private plaintext key during initialization, then Kotlin seals and removes it. Crash recovery may find that private transient file and reseal it. Trust fingerprints are public metadata stored in private application SQLite; Android backups are disabled.

Transport uses rustls/Tokio TLS, ALPN `localconnect/2`, wire version **2**, and length-prefixed typed framing. Control JSON is capped at 64 KiB; manifests at 16 MiB; raw chunks at 4 MiB; PNG frames at 8 MiB. Types and lengths are checked before allocation. Unpaired initial frames retain a 64 KiB cap. Video sequence, PNG signature/IHDR bounds, codec and session binding are validated.

Each bulk transfer, heartbeat/control connection, remote input connection and screen connection is independently authenticated. Heartbeats are framed ping/pong about every 15 seconds with response deadlines, backoff and jitter. They detect failures and may maintain idle connections; they cannot prevent Doze, OS process death or OEM restrictions.

File manifest format remains version **1** for recovery compatibility. It contains transfer UUID, safe name, u64 size, 4 MiB chunk size, implicitly validated chunk count, SHA-256 hashes for each chunk and whole file, and source identity metadata. Empty files work; u64 offsets support files above 4 GiB. Manifest hash count has a bounded policy; default receive limits are 64 GiB per file and 128 GiB incomplete reservations, configurable and saved in SQLite. Reservations are logical limits, not free-space guarantees.

Sender/receiver have one active outgoing/incoming transfer gate. Hashing also holds the outgoing gate. In-flight window is 1–8, default 4. Chunks are read/written sequentially inside each window, with socket backpressure and bounded buffers. Filesystem/SQLite work uses blocking workers. UI progress is throttled, reporting verified bytes, throughput and status.

Receiver ordering:

1. Validate identity, index, offset, exact length and chunk SHA-256.
2. Write only into the registered partial file and sync file data.
3. Commit verified chunk recovery state with SQLite FULL synchronization.
4. Send ACK.

No ACK is sent before data and recovery state are durable. This implementation syncs every accepted chunk; batching is not enabled. Duplicate verified chunks are idempotent and revalidated. Reconciliation rehashes persisted chunks against the partial file and re-requests missing/invalid chunks. Sender rehashes the complete source before resuming and rejects changed content/identity/path. Retries are bounded; permission, disk-full, quota and changed-source failures require explicit recovery.

Finalization verifies the whole-file hash and atomically publishes without overwrite using a same-filesystem hard link, then removes the partial file. Unix directory changes are synced. Existing destinations are rejected. Filesystems without hard-link publication support fail safely; choose a supported dedicated receive folder. Cancellation retains recovery data. Cleanup removes a partial/history entry after active transfers stop, retaining original sources and completed files. Transfer IDs cannot be rebound to different paths or peers. Recovery filenames and traversal characters are rejected.

## 4. Android native integration

The actual Kotlin files are in `android-native/`, copied into the runner by the installer. They are implementations, not the previous disabled reference stubs, but the latest Kotlin/OS behavior still needs the checks in VALIDATION.md.

`MainActivity.kt` owns platform channels, consent results and SAF streaming. `TransferService.kt` starts from visible user interaction, reports real foreground startup success, holds a multicast lock while listening, and holds a timed partial wake lock for active operations. A visible Stop notification shuts down operations. `START_NOT_STICKY` is intentional; there is no boot or silent capture restart.

`ProjectionService.kt` consumes a fresh MediaProjection consent result, registers its stop callback before creating one virtual display, captures via ImageReader, rotates/downsizes/encodes PNG on a worker, resizes the same display, and releases reader/display/projection/wake resources on Stop/destruction. Capture permission denial is actionable. Foreground type is `mediaProjection`; transfer/listening type is `connectedDevice`.

`RemoteAccessibilityService.kt` is enabled only by the user in system settings. It accepts only the approved session and current geometry: taps, swipes lasting 50–500 ms, and `ACTION_SET_TEXT` on an editable focused field. It does not provide desktop keyboard/mouse semantics on Android. Pending gestures are bounded to one latest action. Already executing OS gestures may take up to 500 ms to finish after revocation.

Generation-bound JNI shutdown prevents a delayed old-service callback from stopping a restarted engine. Session-bound JNI cleanup targets the approved UUID. The JNI and desktop C export attributes fix ABI symbols; raw JNI pointers are never dereferenced. Other hand-written Rust denies unsafe code. Generated FRB and pinned OS/FFI dependencies contain their own native boundaries.

SAF documents are not assumed to be filesystem paths. Import streams through a 1 MiB native buffer into persistent app-private staging, then Rust hashes/transfers it. Export streams a completed private verified file into a user-selected document. The chooser requests local-only documents. SAF providers may not support fsync or atomic replacement; failed/cancelled export can leave an incomplete provider document, while the verified private source remains intact. Stop cancels native copy work. Interrupted import cache housekeeping remains a documented limitation.

API 35 permissions include INTERNET, network/Wi-Fi state, CHANGE_WIFI_MULTICAST_STATE, notifications (runtime on API 33+), WAKE_LOCK and appropriate foreground-service permissions/types. No blanket location request is needed for the raw mDNS/manual TCP workflow selected here. Android 16 local-network restrictions are opt-in; Android 17 requires ACCESS_LOCAL_NETWORK when targeting SDK 37 or higher. This target-35 build does not request that newer permission. Revalidate and implement its runtime consent flow before upgrading the target. See https://developer.android.com/privacy-and-security/local-network-permission. Multicast may still be blocked by AP isolation/OEM behavior; use manual IP.

Xiaomi/Oppo/Vivo battery helpers resolve best-effort intents, catch failures and fall back to standard settings. Activity names vary by firmware. No helper bypasses user or OS consent.

## 5. Flutter UI

`main.dart`, `providers.dart`, `remote_page.dart` implement discovery, manual connection, fingerprint confirmation on both devices, trust management, file picking, persisted progress/recovery, retry/cancel/cleanup, saved limits, screen-view/control requests, source approval dialogs, active-session indicators and Stop actions.

Stop bypasses the busy guard. Startup, probe and pairing callbacks check operation epochs; startup is deduplicated until its previous future resolves. Remote input tags the geometry of the displayed frame, so stale rotation events are rejected. PNG decoding allows one read in flight and retains only latest pending metadata. Desktop pointer movement is throttled; input queues/rates are bounded. Loss of focus releases held input; source disconnect cleanup also releases it.

## 6. Validation and remaining work

```sh
cargo fmt --manifest-path rust/Cargo.toml --check
cargo clippy --manifest-path rust/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --locked
python3 tests/run_x11.py
cargo build --manifest-path rust/Cargo.toml --locked --lib
flutter analyze
flutter test --dart-define=LOCALCONNECT_NATIVE_LIB=/absolute/project/rust/target/debug/liblocalconnect_core.so
```

Windows/Linux/Android repository CI builds and packaging passed in PR run 38398824783. A duplicate push run exposed an X11 test-server reset race, which is being fixed before merge. See VALIDATION.md. See that report for recorded runs and remaining acceptance checks.

| Capability | Windows | Linux X11 | Linux Wayland | Android |
|---|---|---|---|---|
| Secure resumable file transfer | Implemented | Implemented | Implemented | Implemented with private staging/SAF |
| View another supported source | Implemented | Implemented | Implemented | Implemented |
| Capture own screen | GDI/software, primary monitor | Software, primary/single monitor | Portal + PipeWire, one selected monitor | Fresh MediaProjection consent |
| Inject input | Enigo; ordinary desktop only | Enigo/X11 | RemoteDesktop portal grants required | Accessibility taps/short swipes/focused text |
| OS/runtime verification | Full Windows release CI build; device input/capture untested | Isolated Xvfb streaming/input tested | Compiles; real compositor untested | Full ARM64 APK build verified; device behavior untested |

PNG streaming targets 4 fps at at most 1920×1080; it is a responsive-control foundation, not a high-frame-rate codec implementation. Windows secure desktops/UAC/elevated applications, multi-monitor switching, arbitrary Android key injection, silent Accessibility enablement, reused projection tokens and permanent background execution are unsupported. Wayland backend/plugin limitations are reported during permission/setup; no XWayland fallback is advertised.

## Current Android debug package

The supplied APK targets ARM64 only, minimum API 26 / target API 35, version
0.2.0 (2). Flutter/Rust architecture matching, signature, native exports, ZIP
alignment and 64-bit ELF 16 KiB alignment were verified. This does not establish
installed-app or 16 KiB-device behavior. Build another target explicitly when
needed; do not package orphan Rust libraries without a matching Flutter engine.

```sh
python3 scripts/verify_android_apk.py build/app/outputs/flutter-apk/app-debug.apk
```

The verification script checks ZIP CRC, ABI matching and 64-bit ELF alignment;
use SDK `apksigner verify` separately for signatures. A new debug signing key was
generated for this build. It may not update earlier installations. Export files
you need before any uninstall; uninstall removes private identity, trust and
recovery state. Release signing requires your own protected key.

