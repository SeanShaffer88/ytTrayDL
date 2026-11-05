; ytTrayDL installer (Inno Setup 6) — per-user install, no admin.
; Built in CI:  iscc /DAppVersion=X.Y.Z installer\yttraydl.iss
; The portable zip stays the no-install option; this exists for
; Start-menu integration, optional desktop shortcut / startup,
; and a clean uninstaller. User data in %APPDATA%\ytTrayDL
; (settings, history, managed yt-dlp/ffmpeg) is deliberately NOT
; removed by the uninstaller — it belongs to the user, and the
; portable build can keep using it.

#ifndef AppVersion
  #define AppVersion "0.7.1"
#endif
; CI builds x64 only and passes /DAppVersion (the define above is the local-build fallback).
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
; Exceptional in-place migration for verified 0.6.1/0.7.0 inventories: append
; retains their unsafe recursive uninstall action. Replace the log while keeping
; all three tracked files and explicit legacy optional-resource cleanup below.
; Future removed/renamed inventory needs exact legacy cleanup + migration tests.
; https://jrsoftware.org/ishelp/topic_setup_uninstalllogmode.htm
; https://jrsoftware.org/ishelp/topic_sameappnotes.htm
UninstallLogMode=overwrite
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
; Record exact legacy value cleanup even if the candidate startup task is OFF.
; ValueType none does not create/change the value during installation.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "ytTrayDL"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch ytTrayDL"; Flags: nowait postinstall skipifsilent

; Inno removes only its tracked files. Preserve user downloads and other
; unowned content under {app}; never recursively delete the application folder.

[UninstallDelete]
; Exact legacy optional shortcut only; never delete a directory or sibling.
Type: files; Name: "{autodesktop}\ytTrayDL.lnk"
