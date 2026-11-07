#ifndef AppVersion
#define AppVersion "1.0.0"
#endif

[Setup]
AppId={{8B3392E7-8547-49C3-9A52-9A8C5B3206D3}
AppName=Earbuds Tray
AppVersion={#AppVersion}
AppPublisher=marcwgls
AppPublisherURL=https://github.com/marcwgls/earbuds-tray-windows
PrivilegesRequired=lowest
DefaultDirName={autopf}\Earbuds Tray
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=earbuds-tray-setup
UninstallDisplayIcon={app}\earbuds-tray.exe
SetupIconFile=..\assets\app-icon.ico
CloseApplications=force
RestartApplications=no
LicenseFile=..\LICENSE

[Files]
Source: "..\target\release\earbuds-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Earbuds Tray"; Filename: "{app}\earbuds-tray.exe"

[Tasks]
Name: "autostart"; Description: "Start with Windows"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "EarbudsTray"; ValueData: """{app}\earbuds-tray.exe"""; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "EarbudsTray"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\AppUserModelId\EarbudsTray"; ValueType: none; Flags: uninsdeletekey

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\earbuds-tray"

[Run]
Filename: "{app}\earbuds-tray.exe"; Description: "Start Earbuds Tray"; Flags: nowait postinstall
