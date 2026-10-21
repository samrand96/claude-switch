; Claude Switch — Windows installer (Inno Setup 7).
;
; Per-user, no administrator rights: everything Claude Switch touches lives in the user's
; profile (HKCU, %USERPROFILE%\.cswitch, the user's Start menu). The installer drops a copy of
; cswitch.exe into {app} and then runs `cswitch install`, which does the real work — the same
; code path as running `cswitch install` by hand: ~/.cswitch/bin, cswitchw.exe, claude-<profile>
; commands, PATH, shortcuts and the claude:// link router. Uninstalling runs `cswitch uninstall`.
;
; Build (paths are relative to this file):
;   ISCC.exe /DAppVersion=1.2.3 /DArch=x64 /DSourceExe=..\..\target\x86_64-pc-windows-msvc\release\cswitch.exe cswitch.iss
; Defines: AppVersion (required), Arch = x64 | arm64, SourceExe (required), OutputDir.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceExe
  #error Pass /DSourceExe=<path to cswitch.exe>
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

#if Arch == "x64"
  #define AllowedArchs "x64compatible"
#elif Arch == "arm64"
  #define AllowedArchs "arm64"
#else
  #error Arch must be x64 or arm64
#endif

; VersionInfoVersion only takes numbers: "1.2.3-beta.1" -> "1.2.3".
#define NumericVersion Copy(AppVersion, 1, Pos("-", AppVersion + "-") - 1)

[Setup]
; One AppId for both architectures, so switching x64 <-> arm64 is an upgrade, not a second app.
AppId={{89EA45E0-CFC8-4438-81CD-D5E2BA066807}
AppName=Claude Switch
AppVersion={#AppVersion}
AppVerName=Claude Switch {#AppVersion}
AppPublisher=samrand96
AppPublisherURL=https://github.com/samrand96/claude-switch
AppSupportURL=https://github.com/samrand96/claude-switch/issues
AppUpdatesURL=https://github.com/samrand96/claude-switch/releases
AppComments=Work and personal Claude (Claude Code + Claude Desktop) side by side, fully isolated.
VersionInfoVersion={#NumericVersion}
VersionInfoProductName=Claude Switch
VersionInfoDescription=Claude Switch Setup
PrivilegesRequired=lowest
ArchitecturesAllowed={#AllowedArchs}
ArchitecturesInstallIn64BitMode={#AllowedArchs}
DefaultDirName={autopf}\Claude Switch
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
LicenseFile=..\..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=claude-switch-{#AppVersion}-windows-{#Arch}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
SetupLogging=yes
UninstallDisplayName=Claude Switch
UninstallDisplayIcon={app}\cswitch.exe
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "cswitch.exe"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

; No [Icons]: `cswitch install` owns the "Claude Switch" Start-menu folder (it recreates it with
; one colored shortcut per profile), so anything Inno put there would be wiped.

[Run]
; Opens the menu, which starts the setup wizard on first run (profiles, browsers, sign-in).
Filename: "{app}\cswitch.exe"; Description: "Set up Claude Switch now (profiles and sign-in)"; Flags: postinstall nowait skipifsilent

[UninstallDelete]
Type: dirifempty; Name: "{app}"

[Code]
var
  PurgeProfiles: Boolean;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    WizardForm.StatusLabel.Caption := 'Setting up Claude Switch for your account...';
    if not Exec(ExpandConstant('{app}\cswitch.exe'), 'install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      SuppressibleMsgBox('Claude Switch could not be started to finish the installation:' + #13#10 +
        SysErrorMessage(ResultCode) + #13#10#13#10 +
        'Open a terminal and run:' + #13#10 + ExpandConstant('"{app}\cswitch.exe" install'),
        mbError, MB_OK, IDOK)
    else if ResultCode <> 0 then
      SuppressibleMsgBox('Claude Switch was copied, but "cswitch install" reported a problem (exit code ' +
        IntToStr(ResultCode) + ').' + #13#10#13#10 +
        'Open a terminal and run these to see the details and fix it:' + #13#10 +
        ExpandConstant('"{app}\cswitch.exe" install') + #13#10 +
        ExpandConstant('"{app}\cswitch.exe" doctor --fix'),
        mbError, MB_OK, IDOK);
  end;
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  PurgeProfiles := False;
  if not UninstallSilent then
    PurgeProfiles := MsgBox('Also delete your Claude Switch profiles?' + #13#10#13#10 +
      'Yes removes every profile stored by Claude Switch (its Claude Code and Claude Desktop ' +
      'sign-ins, settings and history under %USERPROFILE%\.cswitch).' + #13#10 +
      'No keeps them, so reinstalling brings everything back.' + #13#10#13#10 +
      'Your own ~/.claude and Claude Desktop folders are never deleted.',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
  Params: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    Params := 'uninstall --yes';
    if PurgeProfiles then
      Params := Params + ' --purge';
    if not Exec(ExpandConstant('{app}\cswitch.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      Log('cswitch uninstall could not be started: ' + SysErrorMessage(ResultCode))
    else
      Log('cswitch uninstall exited with ' + IntToStr(ResultCode));
  end;
end;
