# Remaining work — current source

## Implemented source

The former Milestone 1 gaps are implemented: configurable bounded chunk window,
structured errors/progress, persisted storage limits, discovery/session snapshots,
authenticated IP relocation, IPv6 listener/manual connections, history cleanup,
Windows credential storage and Android Keystore wrapping.

Approved remote sessions, independent authenticated input/video connections,
latest-frame bounded PNG streaming, Flutter rendering/input, Windows/X11 adapters,
Wayland portal/PipeWire adapter, Android MediaProjection/Accessibility services,
rotation mapping, Stop notification, lock cleanup and generation-bound JNI
shutdown are implemented in source. Linux Rust compilation and isolated X11
integration have passed. Implementation does not establish every platform's
runtime correctness.

## Remaining release blockers

- Install and validate the current Android app on real devices. A full current
  ARM64 APK build, signing/package checks and ELF alignment checks now passed.
- Windows runtime validation: the full Windows release build now passed in
  repository CI, alongside Rust/Flutter tests and ZIP packaging.
- Interactive Linux real-desktop validation (the current debug bundle builds).
- Real GNOME/KDE Wayland portal/PipeWire consent, input and cleanup tests.
- Android projection/accessibility consent, rotation, revocation, foreground
  lifecycle, SAF, Doze/OEM, permission-denial and 16 KiB page-size device checks.
- Physical LAN/mDNS/interface changes, Wi-Fi interruption, process force-kill,
  cross-platform memory/throughput measurements and disk-full/power-loss tests.
  An actual >4 GiB loopback transfer and process RSS check have now passed.
- Configure stable production signing keys and run the release-candidate workflow.
  PR #1 is merged; all Windows/Linux/Android checks passed on merged commit
  a696e4ac in run 38402175618. No private key or public deployment is included.

## Explicit product limitations

- Streaming uses PNG at up to 4 fps and 1920x1080, not a hardware video codec.
  It is functional initial streaming, with substantial bandwidth/CPU tradeoffs.
- Desktop capture targets one primary/single display. Multi-monitor selection,
  full extended/media-key handling and cursor compositing remain enhancements.
- Windows elevated/secure desktop input is unsupported. X11 support does not
  imply Wayland support; Wayland requires working OS portals and PipeWire.
- Android uses taps, bounded short swipes and focused editable text actions.
  Arbitrary desktop keyboard/button injection is unsupported. Accessibility must
  be enabled by the user and every capture requires valid fresh OS consent.
- Atomic no-overwrite publication requires hard-link support; unsupported
  destination filesystems fail safely. SAF export is provider-managed and does
  not inherit private-file atomicity guarantees.
- No promise of permanent Android execution, codec quality, unbroken connections
  or crash/power-loss durability beyond the tested recovery logic.

See VALIDATION.md for executed checks and the acceptance checklist. These are
remaining build/device evidence and known capability limits, not disabled remote
control placeholder files as in the original foundation.
