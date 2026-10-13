# Release and device acceptance handoff

Automated builds do not establish device behavior. Windows/Linux release builds
passed in run 38384325209; Android CI must still run the SDK fix in PR #1.
The previously verified Android package is debug signed. The new release-candidate
workflow is source preparation only: no signed production candidate has been run.

## Finish CI

1. In repository Actions, select Validate LocalConnect.
2. Run on `codex/fix-android-ci-sdk`; wait for all three jobs and inspect artifacts.
3. Review/merge PR #1 after success. A ready-for-review event is also configured.
4. Keep the whole Windows/Linux bundle; do not distribute just the executable.

## Prepare Android signing privately

Generate your key outside the repository, preserving an encrypted/offline backup.
The following prompts for passwords instead of exposing them in command arguments:

```sh
keytool -genkeypair -keystore localconnect-release.jks -alias localconnect -keyalg RSA -keysize 3072 -validity 10000
```

Configure these repository Actions secrets through GitHub's settings, never chat:

- `LOCALCONNECT_KEYSTORE_BASE64`: base64 encoding of the keystore, without newlines.
- `LOCALCONNECT_STORE_PASSWORD`: keystore password.
- `LOCALCONNECT_KEY_ALIAS`: alias selected above.
- `LOCALCONNECT_KEY_PASSWORD`: private-key password (often the same with PKCS12).

Use a private local editor to prepare the base64 value; do not commit or share it.
Run Android release candidate only on reviewed, trusted source. The workflow
exposes signing secrets only to the build step, writes a mode-0600 temporary
keystore, removes it afterward and uploads only the APK. It fails with missing
secret names instead of silently debug-signing. It does not publish a GitHub or
store release. Save the successful commit SHA, APK SHA-256 and signer fingerprint.
Do not lose the production key: replacement keys may prevent installed-app updates.

A production-signed APK may not update a debug-signed installation. Export files
before any uninstall; uninstall erases private identity/trust/recovery state.

## Device acceptance record

Record date, commit SHA, package hash, OS/firmware, display scaling/rotation,
network type, outcome and logs for each test. Mark failures/skips explicitly.

| Check | Procedure | Required result | Status |
|---|---|---|---|
| Windows capture/control | Two paired devices; request view/control; deny, then approve on Windows | No input before approval; frame and pointer/key mapping correct at 100/150/200% scaling | Not run |
| Windows Stop/disconnect | Hold a harmless modifier in an approved test session; Stop or disconnect | Session closes; modifier/buttons released; fresh approval required | Not run |
| Android projection | Start from visible activity; deny/approve fresh OS consent; rotate 0/90/180/270; revoke OS sharing | Denial actionable; mapping correct; revocation stops input/capture; no token reuse | Not run |
| Android Accessibility | Leave disabled, then manually enable; test taps/swipes/editable focused text; disable during session | Accurate capability/error; approved session only; revocation stops further injection | Not run |
| Android SAF | Local document import/export; cancellation; unreadable/read-only provider; low storage | No assumed URI path; bounded copy; verified private file survives failed export | Not run |
| Android lifecycle | Deny notifications; lock/Doze; swipe away/force-stop; notification Stop; OEM battery settings | Actionable restrictions; locks/sockets/capture cleaned; restart/resume; no silent capture restart | Not run |
| Wayland GNOME/KDE | Grant/deny portal consent; test capture/input; revoke; stop PipeWire/portal | No X11 fallback claim; unavailable grants reported; Stop/input release | Not run |
| Physical LAN/WLAN | Two distinct identities; discovery/manual address; deny pairing; compare full fingerprints; pair/forget | Untrusted discovery never grants access; forgotten peer cannot transfer/control | Not run |
| Recovery | Interrupt Wi-Fi, change DHCP IP, restart both apps; resume original source | Missing/invalid chunks requested; original final SHA-256 matches | Not run |
| Filesystem errors | Dedicated disposable test volume: fill it/deny writes; corrupt partial; remove/change source | No durable ACK after failed write/commit; originals/destination protected; actionable failure | Not run |
| Process/power failure | Dedicated disposable device/VM: terminate at write/sync/commit/ACK/finalization; then power-loss test | Reconcile/reverify; no false completion; resume or explicit failure | Not run |
| Responsiveness | Saturate bulk transfer while viewing/controlling; measure RSS and heartbeat deadlines | Bounded buffers; control remains responsive; stale frames dropped | Not run |

For independent hashes: Linux `sha256sum source received`; Windows
`Get-FileHash source -Algorithm SHA256` and the same for received. On Android,
export the completed file through SAF, then hash that exported copy on desktop.
Use a disposable isolated device/volume for destructive failure tests; do not
fill or power-cut your normal system. Local >4 GiB/RSS, recovery, trust and X11
checks already passed as recorded in VALIDATION.md; they do not replace this table.

## Release decision

Resolve every failure or document unsupported capabilities. Retain PNG 4 fps /
single-display limitations in release notes. Publish only verified packages with
checksum/signing information. Windows code signing and distribution choices still
need your publisher certificate/tooling; Linux packaging needs target distro checks.
