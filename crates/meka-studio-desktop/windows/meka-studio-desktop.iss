; The desktop app's Windows installer, for Inno Setup 6. The release workflow compiles it with
;   iscc /DAppVersion=<version> /DBundle=<folder> windows\meka-studio-desktop.iss
; where the folder holds meka-studio-desktop.exe, meka.exe, LICENSE and THIRD-PARTY-NOTICES.txt.
; It installs for the current user unless the person installing chooses all users, keeps the
; app's settings and Meka's sessions on uninstall, and quits a running app before it replaces or
; removes it.

#ifndef AppVersion
  #error Pass the version: /DAppVersion=<version>
#endif
#ifndef Bundle
  #error Pass the folder with the app's files: /DBundle=<folder>
#endif

; The app's name, `productName` in tauri.conf.json.
#define AppName "Meka Studio Desktop"
#define AppExe "meka-studio-desktop.exe"

[Setup]
AppId={{0FEA7F3A-5BD4-40B4-A5D0-35DF69D422BA}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=K4YT3X
AppPublisherURL=https://github.com/k4yt3x/meka-studio
AppSupportURL=https://github.com/k4yt3x/meka-studio/issues
AppUpdatesURL=https://github.com/k4yt3x/meka-studio/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=meka-studio-desktop-x86_64-windows-setup
SetupIconFile=..\icons\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Bundle}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bundle}\meka.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bundle}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bundle}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The shortcut's AppUserModelID registers the name the app's notifications go out under, which is
; its identifier in tauri.conf.json; Windows shows no notification from a program without one.
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; AppUserModelID: "bot.meka.studio"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; AppUserModelID: "bot.meka.studio"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  { Held while an installed copy runs, not a portable one; the app's src/system.rs names it too. }
  RunningMutex = 'bot.meka.studio.running';
  { How long the app has to quit: its server's 15 seconds, and Meka's 45, with time to spare. }
  QuitWaitSeconds = 75;
  { The WebView2 runtime's client ID, under which Edge's updater records the installed version. }
  WebView2Client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WebView2Bootstrapper = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';
  QuitFailed = '{#AppName} is still running. Quit it from its tray icon, then try again.';

var
  QuitPage: TOutputMarqueeProgressWizardPage;

procedure InitializeWizard;
begin
  QuitPage := CreateOutputMarqueeProgressPage('Quitting {#AppName}',
    'Setup waits for {#AppName} to stop Meka.');
end;

{ Whether the app runs. }
function AppRuns: Boolean;
begin
  Result := CheckForMutexes(RunningMutex);
end;

{ Asks the app in the install folder to quit, which the installed copy running does, wherever it
  is installed, and waits for it to. Returns whether no installed copy runs then. One running with
  nothing in the install folder uses none of its files, and is left running. }
function QuitApp(Page: TOutputMarqueeProgressWizardPage): Boolean;
var
  AppPath: String;
  ResultCode, Waited: Integer;
begin
  AppPath := ExpandConstant('{app}\{#AppExe}');
  if not AppRuns or not FileExists(AppPath) then begin
    Result := True;
    Exit;
  end;
  Log('Asking {#AppName} to quit');
  if not Exec(AppPath, '--quit', '', SW_HIDE, ewNoWait, ResultCode) then
    Log('Could not start ' + AppPath + ': ' + SysErrorMessage(ResultCode));
  if Page <> nil then
    Page.Show;
  try
    Waited := 0;
    while AppRuns and (Waited < QuitWaitSeconds * 10) do begin
      if Page <> nil then
        Page.Animate;
      Sleep(100);
      Waited := Waited + 1;
    end;
  finally
    if Page <> nil then
      Page.Hide;
  end;
  Result := not AppRuns;
end;

{ Whether the WebView2 runtime, which draws the app's window, is installed, for the machine or the
  user. }
function HasWebView2: Boolean;
var
  Version: String;
begin
  Result :=
    (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\'
      + WebView2Client, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'))
    or (RegQueryStringValue(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\' + WebView2Client,
      'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

{ Installs the WebView2 runtime with Microsoft's bootstrapper. Returns why it could not, or ''. }
function InstallWebView2: String;
var
  Bootstrapper: String;
  ResultCode: Integer;
begin
  Result := '';
  Log('Installing the WebView2 runtime');
  try
    DownloadTemporaryFile(WebView2Bootstrapper, 'MicrosoftEdgeWebview2Setup.exe', '', nil);
  except
    Result := '{#AppName} needs Microsoft''s WebView2 runtime, which could not be downloaded: '
      + GetExceptionMessage + '. Connect to the internet, then try again.';
    Exit;
  end;
  Bootstrapper := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
  if not Exec(Bootstrapper, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)
    or (ResultCode <> 0) then
    Result := Format('{#AppName} needs Microsoft''s WebView2 runtime, whose installer failed '
      + '(%d). Install it from https://developer.microsoft.com/microsoft-edge/webview2, '
      + 'then try again.', [ResultCode]);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not QuitApp(QuitPage) then begin
    Result := QuitFailed;
    Exit;
  end;
  if not HasWebView2 then
    Result := InstallWebView2;
end;

function InitializeUninstall: Boolean;
begin
  Result := QuitApp(nil);
  if not Result then
    MsgBox(QuitFailed, mbError, MB_OK);
end;
