# Production release status

LocalConnect currently has working test builds, not an established commercial
release. The owner reports real-device testing. Preserve build hashes and the
device/OS test matrix before treating that report as release acceptance.

## Publisher identity gate

The Android release-candidate workflow requires the four existing signing
secrets described in RELEASE_HANDOFF.md. It additionally requires repository
Actions **variable** `LOCALCONNECT_SIGNER_SHA256`: the full SHA-256 fingerprint
of the intended release certificate, not the APK file hash. This fingerprint
is public; the keystore and passwords must remain private.

Read your release certificate locally with:

```sh
keytool -list -v -keystore /private/path/localconnect-release.jks -alias localconnect
```

Enter the password at the prompt. Copy the certificate SHA256 fingerprint to
the repository variable. Preserve an encrypted backup of the key offline.
The workflow verifies the APK signature using apksigner, rejects a mismatched
publisher, an Android Debug certificate, a debuggable application, or a wrong
application ID. The output `.apk.release.json` ties the candidate to its source
commit, package version, APK hash and certificate fingerprint. The metadata is
not itself a signed attestation or Google certification.

Run the release workflow only from reviewed trusted source. It currently
expects `com.example.localconnect` to preserve existing test installations.
Choose the final product application ID before the first paid release; changing
it creates a separate Android app and requires a tested migration plan.

## Protect source distributions

The source packager excludes common environment, keystore, private-key and
database files and credential directories. Test fixtures verify these
exclusions. This is a packaging safeguard, not a complete secret scanner:
credentials embedded in ordinary source files still need a source review.
Never place real signing keys or runtime user data in the repository.

## Owner decisions still required

1. Release identity: final Android application ID, stable signing key and backup;
   Windows publisher signing certificate if distributing a signed Windows app.
2. Commercial model: paid downloads, paid support, or a licensed edition; choose
   price, license scope, upgrade entitlement and refund/support terms before
   implementing purchases. No billing or entitlement enforcement currently exists.
3. Dependency distribution review: retain Flutter and Rust third-party notices
   and review the actual shipped dependencies before choosing your product license.
4. Distribution: direct downloads or app stores; publish stable signed artifacts
   and checksums, with a verified update/migration procedure and rollback plan.
5. Customer support: a public support address or issue channel, response policy,
   private vulnerability reporting channel and clear privacy documentation.

These decisions are not inferred from permission to edit code. Do not collect
payments or advertise an approved production release until the release gates
and customer terms have been completed.

## Release acceptance

- CI for the exact source revision passes. Existing default CI APKs are still
  debug-signed and must not be presented as production builds.
- The production APK passes the publisher-identity verifier with the owner key.
- Test upgrades from the previous signed release without losing files, trust
  records or recovery state. A different debug signer cannot establish this.
- Record the owner's physical-device results per OS/build, including consent,
  Stop, interrupted transfers and reconnect. Repeat failures after fixes.
- Document current PNG streaming (up to 4 fps), single-display scope, Android
  foreground restrictions and platform capability limits in sales material.
- Google Play Protect review is separate from build validation and store
  publication. On 2026-10-14, the APK with SHA-256
  `1e2951955f7bcf8ca695d43077366efbda99d0e799bd416a245a4bc227bdf81d`
  was uploaded to VirusTotal (0/67 vendors flagged it at that time). Google
  confirmed appeal submission and requested email verification by the owner.
  Neither the scan nor the submission establishes Google approval. This review
  concerns the existing debug-signed test APK, not a future production binary.
- Test release verification itself against a real signed APK with SDK tools;
  parser fixture tests do not establish that end-to-end signing has run.

The application remains local-network based. A future checkout website does
not imply permission to add telemetry, cloud relays or remote access servers
to the app. Decide an offline-friendly entitlement model before implementing it.
