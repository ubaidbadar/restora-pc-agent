; Windows-only one-EXE packaging candidate.
; Requires a *separately generated and trusted-signed* Restora PWA MSIX.
; Unconfigured builds fail before packaging. Never build a release with dummy IDs.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef PwaSource
  #error PwaSource is required (signed MSIX file)
#endif
#ifndef PwaSha256
  #error PwaSha256 is required (pinned SHA-256)
#endif
#ifndef PwaPackageName
  #error PwaPackageName is required (Partner Center PWA package identity)
#endif
#ifndef PwaPublisher
  #error PwaPublisher is required
#endif
#ifndef PwaApplicationId
  #error PwaApplicationId is required
#endif

[Setup]
AppId={{A6DFE9D4-69A6-4AA0-AE7D-1B4E716E6DB2}
AppName=Restora Agent
AppVersion={#AppVersion}
AppPublisher=Badar Digital
DefaultDirName={localappdata}\Programs\Restora Agent
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=dist
OutputBaseFilename=Restora-Setup-{#AppVersion}-x64
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
CloseApplications=yes
RestartApplications=no
UninstallDisplayName=Restora Agent
WizardStyle=modern

[Tasks]
Name: "autostart"; Description: "Start background printing when signing in to Windows"; Flags: checkedonce

[Files]
Source: "scripts\provision-pwa.ps1"; Flags: dontcopy
Source: "{#PwaSource}"; DestName: "RestoraPwa.msix"; Flags: dontcopy
Source: "target\release\restora-pc-agent.exe"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "RestoraPrintAgent"; ValueData: """{app}\restora-pc-agent.exe"" --background"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\restora-pc-agent.exe"; Flags: nowait skipifsilent
Filename: "{app}\restora-pc-agent.exe"; Parameters: "--background"; Flags: nowait skipifnotsilent

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
  Params: String;
begin
  Result := '';
  { The Store version launches its own service. Block two listeners and
    require a deliberate, tested migration before installing this EXE. }
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
              '-NoLogo -NoProfile -NonInteractive -Command "if (Get-AppxPackage -Name ''BadarDigital.RestoraPrint'') { exit 31 }; if (Get-NetTCPConnection -LocalPort 17777 -State Listen -ErrorAction SilentlyContinue) { exit 32 }"',
              '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    Result := 'Windows cannot check for the existing Store Agent.';
    Exit;
  end;
  if ResultCode <> 0 then
  begin
    Result := 'Another Restora printing service is installed or port 17777 is already in use. Stop or migrate the existing Agent before setup; printing was left unchanged.';
    Exit;
  end;
  ExtractTemporaryFile('provision-pwa.ps1');
  ExtractTemporaryFile('RestoraPwa.msix');

  { Install signed PWA before installing the agent; stop if its identity, hash,
    signature, or Windows registration fails. The browser is never managed. }
  Params := '-NoLogo -NoProfile -NonInteractive -File "' +
    ExpandConstant('{tmp}\provision-pwa.ps1') + '" -PackagePath "' +
    ExpandConstant('{tmp}\RestoraPwa.msix') +
    '" -ExpectedSha256 "{#PwaSha256}" -PackageName "{#PwaPackageName}"' +
    ' -ExpectedPublisher "{#PwaPublisher}" -ApplicationId "{#PwaApplicationId}"';

  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
              Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    Result := 'Windows could not start Restora PWA installation.';
  end
  else if ResultCode <> 0 then
  begin
    Result := 'The signed Restora PWA could not be installed. Setup has stopped without replacing the print engine. See Windows AppX deployment logs.';
  end;
end;
