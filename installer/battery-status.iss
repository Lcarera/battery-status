; Inno Setup script for Battery Status.
; Build with installer\build.ps1. It sets AppVersion and ExeDir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef ExeDir
  #error "Set ExeDir: the folder that holds the release battery-status.exe."
#endif

#define AppName "Battery Status"
#define ExeName "battery-status.exe"
; Same class name as src/tray.rs.
#define TrayClass "BatteryStatusTrayWindow"

[Setup]
; Do not change AppId: updates and the uninstaller find the app with it.
AppId={{9C80AE6C-D0F7-44F9-946D-83C2C05A4838}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#ExeName}
; Per-user install: no administrator rights. {autopf} is %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\battery-status
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputDir=Output
OutputBaseFilename=battery-status-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Files]
Source: "{#ExeDir}\{#ExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#ExeName}"

[Run]
; The first start creates config.toml and turns on "Start with Windows".
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WM_CLOSE = $0010;

{ Asks the running app to exit (it removes its tray icons), then waits up to 5 s. }
procedure StopRunningApp();
var
  Wnd: HWND;
  I: Integer;
begin
  Wnd := FindWindowByClassName('{#TrayClass}');
  if Wnd = 0 then
    Exit;
  PostMessage(Wnd, WM_CLOSE, 0, 0);
  for I := 1 to 50 do
  begin
    Sleep(100);
    if FindWindowByClassName('{#TrayClass}') = 0 then
      Exit;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunningApp();
  Result := True;
end;

{ The app writes these keys itself, so the uninstaller removes them in code. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'BatteryStatus');
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Classes\AppUserModelId\BatteryStatus.App');
  end;
end;
