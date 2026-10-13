# One-time code pairing

Install the updated LocalConnect build on **both** devices and connect them to the same local Wi-Fi/router (Ethernet is also supported). Internet is not required during use.

1. Start LocalConnect on each device.
2. On the phone, open **Devices → Show pairing code → Generate code**.
3. On the PC, choose the phone from Devices. If discovery fails, expand **Connect by IP address** and enter the phone's local IP and listening port.
4. Enter all **six digits**, including any leading zero. Select Connect.
5. Open Files to send a file. Screen viewing/control requires another explicit approval on the controlled device.

The code expires after 120 seconds, accepts at most three attempts, and can succeed only once. Close invalidates an unfinished pairing. Generate a fresh code after a failed or expired attempt; regeneration has a 15-second cooldown. Device names from discovery are untrusted.

**Remember is off by default.** Temporary trust lasts until that device's LocalConnect engine stops or the app process ends. A socket reconnect inside the same engine session can still work. To retain trust after restarting, enable Remember on **both** devices. Settings → Trusted devices removes access.

Certificate fingerprints are cryptographic identities, not biometric scanners. No Windows fingerprint hardware is needed. The pairing protocol uses SPAKE2 plus TLS-bound key confirmation instead of transmitting the raw code. Keep the displayed code private, including while sharing your screen.

Integrity passports record the completed transfer's manifest SHA-256 and peer identity. They are historical records, not signed receipts or a fresh check of the file's current contents.

Debug APKs are for testing. If Android reports an incompatible update signature, export received files before uninstalling the old app: uninstall deletes private files, device identity, trust and recovery state. A stable production signing key remains a release prerequisite.
