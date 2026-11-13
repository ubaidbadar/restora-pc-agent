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
Source: "target\release\restora-pc-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "scripts\provision-pwa.ps1"; Flags: dontcopy
Source: "{#PwaSource}"; DestName: "RestoraPwa.msix"; Flags: dontcopy

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "RestoraPrintAgent"; ValueData: """{app}\restora-pc-agent.exe"" --headless --background"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\restora-pc-agent.exe"; Parameters: "--headless"; Flags: nowait postinstall skipifsilent; Description: "Open Restora"

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
  Params: String;
begin
  Result := '';
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
