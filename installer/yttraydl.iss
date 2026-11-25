; ytTrayDL installer (Inno Setup 6) — per-user install, no admin.
; Built in CI:  iscc /DAppVersion=X.Y.Z installer\yttraydl.iss
; The portable zip stays the no-install option; this exists for
; Start-menu integration, optional desktop shortcut / startup,
; and a clean uninstaller. User data in %APPDATA%\ytTrayDL
; (settings, history, managed yt-dlp/ffmpeg) is deliberately NOT
; removed by the uninstaller — it belongs to the user, and the
; portable build can keep using it.

#ifndef AppVersion
  #define AppVersion "0.13.4"
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
VersionInfoVersion={#AppVersion}
VersionInfoProductVersion={#AppVersion}
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
Name: "downloadsicon"; Description: "Create a desktop shortcut for Downloads"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "convertericon"; Description: "Create a desktop shortcut for Video converter"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "tagsicon"; Description: "Create a desktop shortcut for Lyrics and tags"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "playericon"; Description: "Create a desktop shortcut for Player"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "startup"; Description: "Start ytTrayDL when Windows starts (tray, no window)"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
; Extract the exact candidate before installation. It shares the application's
; canonical provisioner but is never copied as a second installed executable.
Source: "{#ExeSourceDir}\yttraydl.exe"; DestName: "tool-provisioner.exe"; Flags: dontcopy
Source: "{#ExeSourceDir}\yttraydl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\ytTrayDL"; Filename: "{app}\{#AppExe}"; Comment: "Clipboard video downloader (yt-dlp front-end)"
Name: "{autodesktop}\ytTrayDL"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

Name: "{autodesktop}\ytTrayDL Downloads"; Filename: "{app}\{#AppExe}"; Parameters: "--downloads"; Tasks: downloadsicon

Name: "{autodesktop}\ytTrayDL Video Converter"; Filename: "{app}\{#AppExe}"; Parameters: "--converter"; Tasks: convertericon

Name: "{autodesktop}\ytTrayDL Lyrics and Tags"; Filename: "{app}\{#AppExe}"; Parameters: "--tags"; Tasks: tagsicon

Name: "{autodesktop}\ytTrayDL Player"; Filename: "{app}\{#AppExe}"; Parameters: "--player"; Tasks: playericon

[Registry]
; Same value ytTrayDL's own "Start with Windows" writes, with the
; --background flag so autostart goes straight to the tray.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "ytTrayDL"; ValueData: """{app}\{#AppExe}"" --background"; Tasks: startup; Flags: uninsdeletevalue
; Record exact legacy value cleanup even if the candidate startup task is OFF.
; ValueType none does not create/change the value during installation.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "ytTrayDL"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch ytTrayDL"; Flags: nowait postinstall skipifsilent; Check: CanLaunchApplication

; Inno removes only its tracked files. Preserve user downloads and other
; unowned content under {app}; never recursively delete the application folder.

[UninstallDelete]
; Exact legacy optional shortcut only; never delete a directory or sibling.
Type: files; Name: "{autodesktop}\ytTrayDL.lnk"
Type: files; Name: "{autodesktop}\ytTrayDL Downloads.lnk"
Type: files; Name: "{autodesktop}\ytTrayDL Video Converter.lnk"
Type: files; Name: "{autodesktop}\ytTrayDL Lyrics and Tags.lnk"
Type: files; Name: "{autodesktop}\ytTrayDL Player.lnk"

[Code]
var
  ToolsPage: TInputOptionWizardPage;
  ToolsProgress: TOutputMarqueeProgressWizardPage;
  ToolsProvisioned: Boolean;
  ProvisionAttempt: Integer;
  CancelToolsSetup: Boolean;

procedure CancelButtonClick(CurPageID: Integer; var Cancel, Confirm: Boolean);
begin
  if CancelToolsSetup then begin
    Cancel := True;
    Confirm := False;
  end;
end;

function ToolsDownloadMode: String;
begin
  Result := Lowercase(ExpandConstant('{param:TOOLSDOWNLOAD|required}'));
end;

function InitializeSetup: Boolean;
begin
  Result := (ToolsDownloadMode = 'required') or (ToolsDownloadMode = 'skip');
  if not Result then
    Log('Invalid /TOOLSDOWNLOAD mode; expected required or skip.');
end;

procedure InitializeWizard;
begin
  ToolsPage := CreateInputOptionPage(wpSelectTasks, 'Download Tools',
    'Choose how to prepare yt-dlp and ffmpeg.',
    'yt-dlp downloads media. ffmpeg merges and converts it.' + #13#10 +
    'Setup reuses available tools and downloads only missing tools (up to about 150 MB).' + #13#10 +
    'Your settings, history, existing tools, and downloaded media are preserved.',
    True, False);
  ToolsPage.Add('Use Existing Tools and Download Missing Tools (Recommended)');
  ToolsPage.Add('Skip Downloads - Install Offline (Do Not Launch Automatically)');
  ToolsPage.SelectedValueIndex := 0;
  if ToolsDownloadMode = 'skip' then
    ToolsPage.SelectedValueIndex := 1;
  ToolsProgress := CreateOutputMarqueeProgressPage('Download Tools',
    'Checking existing tools and preparing missing tools.');
end;

function CanLaunchApplication: Boolean;
begin
  { Offline installs never launch automatically, even if older tools exist. }
  Result := ToolsProvisioned;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Helper, Report, Failure: String;
  ReportText: AnsiString;
  ResultCode, Choice: Integer;
  Started: Boolean;
begin
  Result := '';
  if ToolsProvisioned then
    Exit;
  if (ToolsDownloadMode = 'skip') or (ToolsPage.SelectedValueIndex = 1) then begin
    Log('Media tool downloads explicitly skipped. Application auto-launch disabled.');
    Exit;
  end;

  ExtractTemporaryFile('tool-provisioner.exe');
  Helper := ExpandConstant('{tmp}\tool-provisioner.exe');
  repeat
    ProvisionAttempt := ProvisionAttempt + 1;
    Report := ExpandConstant('{tmp}\tools-report-') + IntToStr(ProvisionAttempt) + '.json';
    Failure := '';
    Started := False;
    ResultCode := -1;
    if not WizardSilent then begin
      ToolsProgress.SetText('Checking existing tools and preparing missing tools...',
        'Available tools are reused; missing tools are downloaded and verified.' + #13#10 +
        'This bounded operation must finish before Setup can continue or cancel.');
      ToolsProgress.Show;
      ToolsProgress.Animate;
    end;
    try
      Started := Exec(Helper, '--ensure-tools --app-dir "' + ExpandConstant('{app}') + '" --report "' + Report + '"',
        ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ResultCode);
    finally
      if not WizardSilent then
        ToolsProgress.Hide;
    end;
    if Started and (ResultCode = 0) and FileExists(Report) then begin
      ToolsProvisioned := True;
      Log('Media tool provisioning succeeded; canonical helper reported success.');
      Exit;
    end;
    if not Started then
      Failure := 'The tool downloader could not start: ' + SysErrorMessage(ResultCode)
    else
      Failure := 'Tools could not be found, downloaded or verified (code ' + IntToStr(ResultCode) + ').';
    Log(Failure);
    if LoadStringFromFile(Report, ReportText) then begin
      Failure := Failure + #13#10 + Copy(String(ReportText), 1, 2000);
      Log('Tool provisioner details: ' + Copy(String(ReportText), 1, 2000));
    end;
    if WizardSilent then begin
      { Returning an error from PrepareToInstall aborts silent setup before
        files/registration changes. No message box is shown. }
      Result := Failure + ' Use /TOOLSDOWNLOAD=skip for an offline installation.';
      Exit;
    end;
    Choice := TaskDialogMsgBox('Tool Download Failed',
      Failure + #13#10 + #13#10 + 'Existing tool copies and settings are preserved.' + #13#10 +
      'Retry, install without downloading tools, or cancel Setup.',
      mbError, MB_ABORTRETRYIGNORE, ['Retry', 'Skip Tools', 'Cancel Setup'], 0);
    if Choice = IDIGNORE then begin
      ToolsPage.SelectedValueIndex := 1;
      Log('User chose Skip Tools after a failed download. Auto-launch disabled.');
      Exit;
    end;
    if Choice <> IDRETRY then begin
      CancelToolsSetup := True;
      Result := 'Setup was cancelled before installation because tools were not prepared.';
      { Queue closing until PrepareToInstall returns and Inno re-enables
        cancellation. The CancelButtonClick flag prevents a second prompt. }
      PostMessage(WizardForm.Handle, $0010, 0, 0);
      Exit;
    end;
  until False;
end;
