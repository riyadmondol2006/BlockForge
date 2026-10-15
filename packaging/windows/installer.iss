; BlockForge for Windows: the installer (Inno Setup 6.3 or newer, https://jrsoftware.org/isinfo.php).
;
; Build it with packaging\windows\build.ps1, or by hand:
;   ISCC /DAppVersion=4.0.0 /DAppVersionQuad=4.0.0.0 /DExeSource=..\..\target\release\blockforge.exe installer.iss
; The result is BlockForge-<version>-windows-x64-setup.exe in the folder given by /DOutputDir (default: ..\..\dist).
;
; What it does:
;  - installs for the current user only (no administrator prompt), into %LOCALAPPDATA%\Programs\BlockForge
;  - Start Menu shortcut, and a desktop shortcut that is ticked by default
;  - closes a running BlockForge before it replaces the files, and upgrades over an earlier install of itself
;  - finds the JAVA edition of BlockForge (installed by jpackage as an MSI with the upgrade code below), uninstalls it
;    quietly and installs this one in its place. Only the program is removed: the data folder %APPDATA%\BlockForge
;    (instances, accounts, settings) is shared by both editions and is never touched, here or by the uninstaller.
;  - supports /VERYSILENT (and /SUPPRESSMSGBOXES /NORESTART /DIR=... /LOG=...) for scripts and CI

#ifndef AppVersion
  #define AppVersion "4.0.0"
#endif
#ifndef AppVersionQuad
  #define AppVersionQuad AppVersion + ".0"
#endif
#ifndef ExeSource
  #define ExeSource "..\..\target\release\blockforge.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

#define AppName "BlockForge"
#define AppExe "blockforge.exe"
#define AppPublisher "BlockForge"
#define AppURL "https://github.com/riyadmondol2006/BlockForge"
#define AppCopyright "Copyright ATLauncher and BlockForge contributors, GPL-3.0"
; the id the shortcuts carry and the program sets for itself (src/main.rs): the taskbar groups the window with them
#define AppUserModelId "io.github.riyadmondol2006.BlockForge"

[Setup]
; never change this id: it is how a newer setup recognises an earlier install of this edition
AppId={{32084B46-4379-48D5-AF59-3010DEB6CBFA}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
AppCopyright={#AppCopyright}
AppComments=A fast, modern Minecraft launcher
VersionInfoVersion={#AppVersionQuad}
VersionInfoProductVersion={#AppVersionQuad}
VersionInfoProductTextVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoCompany={#AppPublisher}
VersionInfoCopyright={#AppCopyright}
VersionInfoDescription={#AppName} Setup
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=no
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=blockforge.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
WizardImageFile=wizard-164x314.bmp,wizard-192x386.bmp,wizard-246x459.bmp
WizardSmallImageFile=wizard-small-55x55.bmp,wizard-small-64x68.bmp,wizard-small-83x80.bmp
Compression=lzma2/ultra64
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename={#AppName}-{#AppVersion}-windows-x64-setup
; closes a running BlockForge (the Restart Manager asks it to quit, and ends it if it doesn't) before the files are
; replaced; with /VERYSILENT it does so without asking. It isn't started again by the setup: the finish page has the
; "Launch" box for that.
CloseApplications=yes
CloseApplicationsFilter={#AppExe}
RestartApplications=no
SetupMutex=BlockForgeSetupMutex
ShowLanguageDialog=no
UsePreviousLanguage=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; ticked the first time; later installs remember what the person chose
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Files]
Source: "{#ExeSource}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "A fast, modern Minecraft launcher"; AppUserModelID: "{#AppUserModelId}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "A fast, modern Minecraft launcher"; Tasks: desktopicon; AppUserModelID: "{#AppUserModelId}"

[Registry]
; lets Win+R and the Run dialog find "blockforge"
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#AppExe}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#AppExe}"; ValueType: string; ValueName: "Path"; ValueData: "{app}"

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  // The Java edition's installer (scripts/package.ps1: jpackage --win-per-user-install --win-upgrade-uuid ...) is an MSI.
  // Its upgrade code, and the same code in the "packed" form Windows Installer uses as a registry key name.
  LegacyUpgradeCode = '{BFD7C18E-448A-4ABD-80C9-5DF7629AF779}';
  LegacyUpgradeCodePacked = 'E81C7DFBA844DBA4089CD57F26A97F97';
  // the name the MSI shows in "Apps & features"
  LegacyDisplayName = 'BlockForge';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall';

var
  // MSI product codes ({GUID}) of Java editions installed for this user
  LegacyCodes: TStringList;
  // true when a Java edition for all users was found: that one needs an administrator, so it is only mentioned
  LegacyMachineWide: Boolean;

function ReverseText(const S: String): String;
var
  I: Integer;
begin
  Result := '';
  for I := Length(S) downto 1 do
    Result := Result + S[I];
end;

// Windows Installer stores product and upgrade codes as 32 digits: the first three groups reversed, then each pair of
// digits of the rest reversed. E81C7DFB... is {BFD7C18E-...}.
function PackedToGuid(const P: String): String;
var
  Tail: String;
  I: Integer;
begin
  Result := '';
  if Length(P) <> 32 then
    Exit;
  Tail := '';
  for I := 0 to 7 do
    Tail := Tail + ReverseText(Copy(P, 17 + 2 * I, 2));
  Result := '{' + ReverseText(Copy(P, 1, 8)) + '-' + ReverseText(Copy(P, 9, 4)) + '-' + ReverseText(Copy(P, 13, 4)) +
    '-' + Copy(Tail, 1, 4) + '-' + Copy(Tail, 5, 12) + '}';
  Result := Uppercase(Result);
end;

procedure AddLegacyCode(const Code: String);
begin
  if (Length(Code) = 38) and (Code[1] = '{') and (LegacyCodes.IndexOf(Uppercase(Code)) < 0) then
  begin
    Log('Java edition found: product ' + Uppercase(Code));
    LegacyCodes.Add(Uppercase(Code));
  end;
end;

// Looks through the programs listed in Apps & features for ones that are the Java edition's MSI.
procedure ScanUninstallKey(const Root: Integer; const MachineWide: Boolean);
var
  Names: TArrayOfString;
  I: Integer;
  Key, DisplayName, Command: String;
begin
  if not RegGetSubkeyNames(Root, UninstallKey, Names) then
    Exit;
  for I := 0 to GetArrayLength(Names) - 1 do
  begin
    if (Length(Names[I]) <> 38) or (Names[I][1] <> '{') then
      Continue;
    Key := UninstallKey + '\' + Names[I];
    if RegQueryStringValue(Root, Key, 'DisplayName', DisplayName) and (CompareText(DisplayName, LegacyDisplayName) = 0) and
       RegQueryStringValue(Root, Key, 'UninstallString', Command) and (Pos('msiexec', Lowercase(Command)) > 0) then
    begin
      if MachineWide then
      begin
        Log('Java edition found for all users: product ' + Names[I] + ' (needs an administrator, left alone)');
        LegacyMachineWide := True;
      end
      else
        AddLegacyCode(Names[I]);
    end;
  end;
end;

procedure FindLegacyInstalls;
var
  Products: TArrayOfString;
  I: Integer;
begin
  // 1. by upgrade code: every product of this upgrade code is listed under it (per-user installs live in HKCU)
  if RegGetValueNames(HKCU, 'Software\Microsoft\Installer\UpgradeCodes\' + LegacyUpgradeCodePacked, Products) then
    for I := 0 to GetArrayLength(Products) - 1 do
      AddLegacyCode(PackedToGuid(Products[I]));
  // 2. by name, in case the upgrade code list was cleaned up
  ScanUninstallKey(HKCU, False);
  ScanUninstallKey(HKLM, True);
end;

// True when removing this MSI could reach the data folder: the install location is the data folder or contains it.
// An MSI removes its whole install folder, so a Java edition that someone installed into %APPDATA%\BlockForge is left
// alone rather than risking their instances and accounts.
function LegacyTouchesDataFolder(const Code: String): Boolean;
var
  Location, Data: String;
begin
  Data := AddBackslash(Lowercase(ExpandConstant('{userappdata}\BlockForge')));
  Location := '';
  RegQueryStringValue(HKCU, UninstallKey + '\' + Code, 'InstallLocation', Location);
  if Location <> '' then
    Result := Pos(AddBackslash(Lowercase(Location)), Data) = 1
  else
    // no location recorded: the Java program's own folders would show up in the data folder
    Result := FileExists(ExpandConstant('{userappdata}\BlockForge\BlockForge.exe')) or
      DirExists(ExpandConstant('{userappdata}\BlockForge\runtime'));
end;

procedure RemoveLegacyInstalls;
var
  I, ResultCode: Integer;
  Code: String;
begin
  for I := 0 to LegacyCodes.Count - 1 do
  begin
    Code := LegacyCodes[I];
    if LegacyTouchesDataFolder(Code) then
    begin
      Log('Java edition ' + Code + ' is installed in the data folder: not removing it');
      Continue;
    end;
    Log('Removing the Java edition ' + Code);
    if not WizardSilent then
      WizardForm.StatusLabel.Caption := 'Removing the previous Java edition of BlockForge...';
    if Exec(ExpandConstant('{sys}\msiexec.exe'), '/x ' + Code + ' /qn /norestart', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    begin
      // 0 removed, 3010 and 1641 removed but a restart is pending, 1605 no such product (already gone)
      if (ResultCode = 0) or (ResultCode = 3010) or (ResultCode = 1641) or (ResultCode = 1605) then
        Log('Java edition ' + Code + ' removed (msiexec exit code ' + IntToStr(ResultCode) + ')')
      else
        Log('Java edition ' + Code + ' could not be removed (msiexec exit code ' + IntToStr(ResultCode) + '), installing next to it');
    end
    else
      Log('msiexec could not be started: ' + SysErrorMessage(ResultCode));
  end;
end;

function InitializeSetup: Boolean;
begin
  LegacyCodes := TStringList.Create;
  LegacyMachineWide := False;
  FindLegacyInstalls;
  Result := True;
end;

procedure DeinitializeSetup;
begin
  LegacyCodes.Free;
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo, MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := '';
  if MemoDirInfo <> '' then
    Result := Result + MemoDirInfo + NewLine + NewLine;
  if MemoTasksInfo <> '' then
    Result := Result + MemoTasksInfo + NewLine + NewLine;
  if LegacyCodes.Count > 0 then
    Result := Result + 'Upgrade:' + NewLine + Space + 'The earlier Java edition of BlockForge on this computer will be replaced.' + NewLine +
      Space + 'Your instances, accounts and settings are kept.' + NewLine + NewLine;
  if LegacyMachineWide then
    Result := Result + 'Note:' + NewLine + Space + 'A Java edition installed for all users was found. Remove it in Settings > Apps' + NewLine +
      Space + 'so that only one BlockForge is listed.' + NewLine;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    RemoveLegacyInstalls;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  // Nothing in the data folder is ever deleted: it holds the player's instances and accounts and is shared with
  // the Java edition.
  if (CurUninstallStep = usPostUninstall) and (not UninstallSilent) then
    MsgBox('BlockForge was removed. Your instances, accounts and settings in' + #13#10 +
      ExpandConstant('{userappdata}\BlockForge') + #13#10 + 'were kept.', mbInformation, MB_OK);
end;
