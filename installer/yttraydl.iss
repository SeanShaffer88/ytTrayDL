; ytTrayDL installer (Inno Setup 6) — per-user install, no admin.
; Built in CI:  iscc /DAppVersion=X.Y.Z installer\yttraydl.iss
; The portable zip stays the no-install option; this exists for
; Start-menu integration, optional desktop shortcut / startup,
; and a clean uninstaller. User data in %APPDATA%\ytTrayDL
; (settings, history, managed yt-dlp/ffmpeg) is deliberately NOT
; removed by the uninstaller — it belongs to the user, and the
; portable build can keep using it.

#ifndef AppVersion
  #define AppVersion "0.5.0"
#endif
; 32-bit builds (CI passes both): /DExeSourceDir=..\target\i686-pc-windows-msvc\release /DArchSuffix=x86
#ifndef ExeSourceDir
  #define ExeSourceDir "..\target\release"
#endif
#ifndef ArchSuffix
  #define ArchSuffix "x64"
#endif
#define AppName "ytTrayDL"
#define AppPublisher "Sean Shaffer"
#define AppURL "https://github.com/SeanShaffer88/ytTrayDL"
#define AppExe "yttraydl.exe"

[Setup]
AppId={{F85DF45F-C581-47D6-8605-ADFF0A87D50A}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}
AppUpdatesURL={#AppURL}/releases
DefaultDirName={localappdata}\Programs\ytTrayDL
DefaultGroupName=ytTrayDL
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=ytTrayDL-Setup-v{#AppVersion}-windows-{#ArchSuffix}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=force

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "startup"; Description: "Start ytTrayDL when Windows starts (tray, no window)"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "{#ExeSourceDir}\yttraydl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\ytTrayDL"; Filename: "{app}\{#AppExe}"; Comment: "Clipboard video downloader (yt-dlp front-end)"
Name: "{autodesktop}\ytTrayDL"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Same value ytTrayDL's own "Start with Windows" writes, with the
; --background flag so autostart goes straight to the tray.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "ytTrayDL"; ValueData: """{app}\{#AppExe}"" --background"; Tasks: startup; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch ytTrayDL"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
