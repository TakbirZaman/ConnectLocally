# Android beta installation identity

New builds use `io.github.takbirzaman.localconnect`. Earlier supplied APKs used
`com.example.localconnect`. Android treats these as separate applications:
installing the new ID does not upgrade, overwrite or migrate the old installation.
Both may appear as LocalConnect in the launcher. Check App info/package identity
before stopping or uninstalling either one.

1. In the old app, finish or cancel pending transfers and Stop active sessions.
2. Export every completed file you want to keep to a chosen SAF document location.
   Verify exports are readable; use SHA-256 comparison for important files.
3. Stop/force-stop the old app so both apps do not compete for listening ports.
4. Install the new app. It creates a new private identity and empty transfer history.
   Pair again with each peer. Forget the old device entry on your other devices.
5. Grant notifications and fresh capture consent as needed. Accessibility enablement
   does not carry over: enable it only for the intended new app after disclosure.
6. Re-select source files through SAF and start new transfers. Old partial transfers,
   keys, trust records, URI grants and recovery databases are not imported.
7. Keep the old app until exports and new-app operation are verified. Uninstalling
   it permanently removes its app-private files and recovery state.

To return to the old test app, Stop the new app first and launch the old installation.
New-app state is not synchronized back. This is a manual transition, not an automatic
migration or a tested upgrade path. Validate it on physical devices before release.

Full and View Only variants share the new application ID and replace each other only
when their signing certificates are compatible. Export files before uninstalling
for a signer mismatch. Default CI debug keys are not stable production identity.

The Google appeal submitted on 2026-10-14 concerns the old ID and exact APK hash
recorded in PRODUCTION_RELEASE.md. It does not approve the new application ID,
new binary or release signing certificate. Review the actual signed candidate
separately when available.
