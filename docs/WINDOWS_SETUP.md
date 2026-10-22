# Windows: install once, keep your receive folder

Download `LocalConnect-Windows-x64-Setup.exe` from the successful Windows artifact
for the tested revision. Run Setup once; it installs for your Windows user under
`%LOCALAPPDATA%\Programs\LocalConnect` and adds LocalConnect to the Start menu.
You can optionally create a desktop shortcut. On subsequent days, launch
**LocalConnect from Start** instead of reopening Setup or extracting the ZIP again.
Close LocalConnect before running a newer installer. The stable installer ID
updates the existing per-user installation. No administrator account is required.

The portable ZIP remains available: extract the entire bundle to a stable folder
once and launch `localconnect.exe` from there. Keep its DLLs and data folder together.
The Windows packages include the app-local Microsoft C++ runtime from Visual Studio's
redistributable directory; no separate runtime installer runs on the user's PC.

## Received files

Press Start LocalConnect to listen on the LAN. First start uses your Windows
Downloads folder's `LocalConnect` subfolder, falling back to Documents if Downloads
is unavailable. The app remembers the chosen receive folder in its existing
application-support location. It no longer opens a folder chooser at every start.

The Files and Settings tabs include **Open received files** and **Copy folder path**.
To select another receive folder, Stop all, then select **Change receive folder**
before starting again. Choosing a folder once persists it across app restarts.
Old received files stay in their previous location. This does not move existing
partials or change transfer recovery paths; resume old transfers only with their
registered files/folders still available.

If an external drive or saved folder disappears, startup reports the problem instead
of silently changing the receive location. Reconnect the drive or choose another
existing folder. A fresh settings file cannot infer folders selected in older builds,
which never saved that choice; select your old folder once if you want to reuse it.

Installer updates and uninstallation do not explicitly delete application-support
identity, trust or recovery data, or files in your receive folder. Preserve backups
before removing data manually. Do not select the application installation folder
for received files; keep user documents outside the app bundle.

The installer is not Windows publisher-signed yet; Android signing does not sign
Windows packages. Test installation and the receive-folder experience on a real
Windows desktop before public distribution. CI tests installation, reinstall and
uninstall, but does not establish interactive Explorer or real-device behavior.
