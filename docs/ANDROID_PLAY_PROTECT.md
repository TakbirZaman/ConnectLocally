# Android installation and Play Protect

The reported screenshot says “App blocked to protect your device” and mentions sensitive data. This matches Google's documented sensitive-permission Internet-sideload protection; it does not establish an Android 17 runtime failure. LocalConnect's full Android package declares an Accessibility service for user-approved phone gestures/text input. It does not request SMS or notification-listener access.

## Optional View Only package

`LocalConnect-Android-ViewOnly.apk` supports secure files, pairing, approved screen capture/viewing, and controlling a capable PC. It **cannot accept remote taps, swipes or text input on the phone**. Its manifest removes the Accessibility service entirely. Kotlin reports that capability as unsupported, rejects attempts to authorize/inject phone input, and Flutter hides the unavailable settings action.

Build it with:

```sh
python3 scripts/build_android_view_only.py
python3 scripts/verify_view_only_apk.py dist/LocalConnect-Android-ViewOnly.apk "$ANDROID_HOME/build-tools/35.0.0/aapt2"
```

The wrapper temporarily edits the generated runner manifest, builds, and restores the full-control manifest even on failure. It does not alter Google's scanner, ask users to disable protection, disguise the app as an accessibility tool, or guarantee installation approval. The full-control source and APK remain available.

## Full-control distribution

Remote phone input requires the user's explicit Accessibility enablement plus LocalConnect session approval. Removing this capability is a real functionality tradeoff, not a way to keep hidden input privileges. For a full-control app incorrectly classified, review Google's developer guidance and submit a Play Protect appeal with your stable signed APK and accurate disclosures. Google decides the result. Production signing and Play distribution require the owner's credentials/key; no key is included.

- Guidance: https://developers.google.com/android/play-protect/warning-dev-guidance
- Appeal: https://support.google.com/googleplay/android-developer/answer/2992033

Debug builds may have different signing certificates between CI runs. Export received files before uninstalling if Android rejects an update; uninstall removes private identity, trust, files and recovery state. Full and View Only builds share an application ID and replace one another.
