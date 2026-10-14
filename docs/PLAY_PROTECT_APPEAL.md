# Play Protect review preparation

Status: prepared; **not submitted or approved**. Checked 2026-10-14.

This concerns the reported sideloaded-APK Play Protect block, not Play Store
publication. Google decides the classification. A new icon or a passing test
suite does not establish policy approval.

Official form: https://support.google.com/googleplay/android-developer/contact/protectappeals

The form currently requires contact email, application package name, SHA-256 of
the APK uploaded to VirusTotal, and supporting information. Developer name is
also available. Do not treat the form's hidden confirmation text as evidence of
submission. Retain the actual receipt after submission and verify the email if
Google requests it.

## Submission checklist

- Use the exact full-control APK being appealed. Its package is
  `com.example.localconnect`; do not submit the View Only hash for this app's
  Accessibility behavior.
- Upload that APK to VirusTotal and retain its SHA-256 report URL. An APK upload
  shares the binary with that external service. Do not upload a keystore,
  signing password, user files, or app-private identity databases.
- Enter an owner-controlled contact email. No contact email was provided in
  this conversation, so none is guessed from the LinkedIn profile.
- Copy the supporting text below, replacing the artifact placeholders only
  after verifying the actual APK. Record build commit and APK hash together.
- The current CI produces debug-signed test APKs. Preserve the reviewed binary;
  another build may have a different hash and signing certificate. For stable
  distribution, configure an owner-controlled production key using the existing
  release-candidate workflow. Never commit private signing material.
- Only claim the actual implementation and supplied evidence. Do not claim
  Google certification, a malware-free verdict, or complete policy compliance
  without the corresponding review.

## Supporting information draft

LocalConnect is a user-operated local-network file transfer and remote-screen
application developed by Takbir Zaman Bhuiyan. I request review of the Play
Protect installation block shown for this APK. The reported warning mentions
access to sensitive data.

Package: com.example.localconnect
APK SHA-256: [exact submitted binary hash]
Source/build revision: [exact checked commit]
VirusTotal report: [report for the same binary]
Source: https://github.com/TakbirZaman/ConnectLocally

The full-control variant declares Android Accessibility solely for taps,
swipes, and supported text entry sent by a paired LAN device during a separately
approved remote-control session. The user chooses whether to enable the service
in Android settings. An in-app disclosure explains gestures, focused-field
access, local screen sharing, optional use, and how to stop access before opening
those settings. The app does not declare itself an accessibility tool for users
with disabilities.

Focused editable-window access is used to apply text-entry actions, not to send
the accessibility tree to the developer. Screen images use separate Android
MediaProjection consent. They can contain private information visible on-screen,
which the disclosure explains. Files, frames and remote-input commands travel
directly between authenticated local peers using TLS; there are no cloud relay,
analytics or advertising services. The optional creator link opens LinkedIn in
an external handler only when the user taps it.

Remote access requires explicit approval on the controlled device. Active
sessions expose Stop controls and Android foreground notifications. The app
does not enable Accessibility automatically or restart screen capture using old
consent. The manifest contains no SMS or notification-listener permissions.
An optional View Only variant removes the Accessibility service and phone input.

The owner reports real-device testing. Automated validation and its limitations
are recorded in VALIDATION.md and CI; no independent security certification is
claimed. Please review the classification of the submitted binary in light of
this disclosed, consent-based use of Android APIs.

## References

- https://developers.google.com/android/play-protect/warning-dev-guidance
- https://support.google.com/googleplay/android-developer/contact/protectappeals

Play Store publication is a separate process, including developer-account and
signing setup and the applicable privacy, Data safety and Accessibility
declarations. A Play Protect appeal does not publish the app on Google Play.
