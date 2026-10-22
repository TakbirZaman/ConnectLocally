#ifndef AppVersion
  #define AppVersion "0.2.1"
#endif
[Setup]
AppId={{8D604B2C-6BD8-4B13-AE07-E4FF812A195C}
AppName=LocalConnect
AppVersion={#AppVersion}
AppPublisher=Takbir Zaman Bhuiyan
DefaultDirName={localappdata}\Programs\LocalConnect
DefaultGroupName=LocalConnect
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=LocalConnect-Windows-x64-Setup
SetupIconFile=..\..\windows\runner\resources\app_icon.ico
UninstallDisplayIcon={app}\localconnect.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\..\build\windows\x64\runner\Release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\LocalConnect"; Filename: "{app}\localconnect.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\LocalConnect"; Filename: "{app}\localconnect.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\localconnect.exe"; Description: "Open LocalConnect"; Flags: nowait postinstall skipifsilent

; No deletion rules for received files or application-support state.
; Setup and uninstall only manage the installed application bundle and shortcuts.
