#ifndef AppVersion
  #define AppVersion "1.0.5"
#endif

#ifndef BuildRoot
  #define BuildRoot "..\target\release"
#endif

#ifndef ArtifactsDir
  #define ArtifactsDir "..\artifacts"
#endif

#define AppName "SENZO"
#define AppExeName "senzo.exe"

[Setup]
AppId={{9B7477C6-8E3D-4EA7-A128-EB249D052C6C}
AppName={#AppName}
AppVerName={#AppName} {#AppVersion}
AppVersion={#AppVersion}
AppPublisher=SENZO Project
AppPublisherURL=https://github.com/Sensei002/senzo
AppSupportURL=https://github.com/Sensei002/senzo/issues
AppUpdatesURL=https://github.com/Sensei002/senzo/releases
DefaultDirName={localappdata}\Programs\SENZO
DefaultGroupName=SENZO
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Older clients launch Setup before their shutdown has completed. Restart Manager
; still requests a graceful close first; force is the fallback for a stuck client.
CloseApplications=force
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
OutputDir={#ArtifactsDir}
OutputBaseFilename=SENZO-Setup-v{#AppVersion}-x64
SetupIconFile=..\app\assets\app.ico
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
VersionInfoVersion={#AppVersion}
VersionInfoCompany=SENZO Project
VersionInfoDescription={#AppName} Installer
VersionInfoCopyright=Copyright (C) 2026 SENZO Project
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}

[InstallDelete]
; Remove obsolete Stremio Native files and shortcuts when upgrading this install.
Type: files; Name: "{app}\stremio-native.exe"
Type: files; Name: "{group}\Stremio Native.lnk"
Type: filesandordirs; Name: "{userprograms}\Stremio Native"
Type: files; Name: "{autodesktop}\Stremio Native.lnk"
Type: files; Name: "{userstartup}\Stremio Native.lnk"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "startup"; Description: "Start SENZO when I sign in"; GroupDescription: "Startup:"; Flags: unchecked
Name: "magnet"; Description: "Open magnet links with SENZO"; GroupDescription: "Link handling:"; Flags: unchecked

[Files]
Source: "{#BuildRoot}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildRoot}\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildRoot}\msvc-runtime\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildRoot}\licenses\mpv\LICENSE.GPL"; DestDir: "{app}\licenses\mpv"; Flags: ignoreversion
Source: "{#BuildRoot}\licenses\mpv\LICENSE.LGPL"; DestDir: "{app}\licenses\mpv"; Flags: ignoreversion

; AppUserModelID must match SetCurrentProcessExplicitAppUserModelID in the app so
; Windows resolves the friendly name and icon shown in the media overlay / taskbar.
[Icons]
Name: "{group}\SENZO"; Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"; AppUserModelID: "com.senzo.desktop"
Name: "{autodesktop}\SENZO"; Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon; AppUserModelID: "com.senzo.desktop"
Name: "{userstartup}\SENZO"; Filename: "{app}\{#AppExeName}"; Parameters: "--start-hidden"; WorkingDir: "{app}"; Tasks: startup; AppUserModelID: "com.senzo.desktop"

[Registry]
Root: HKCU; Subkey: "Software\Classes\stremio"; ValueType: string; ValueData: "URL:SENZO Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\stremio"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\stremio\DefaultIcon"; ValueType: string; ValueData: "{app}\{#AppExeName},0"
Root: HKCU; Subkey: "Software\Classes\stremio\shell\open\command"; ValueType: string; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\magnet"; ValueType: string; ValueData: "URL:Magnet Protocol"; Flags: uninsdeletekey; Tasks: magnet
Root: HKCU; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Tasks: magnet
Root: HKCU; Subkey: "Software\Classes\magnet\DefaultIcon"; ValueType: string; ValueData: "{app}\{#AppExeName},0"; Tasks: magnet
Root: HKCU; Subkey: "Software\Classes\magnet\shell\open\command"; ValueType: string; ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: magnet

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch SENZO"; WorkingDir: "{app}"; Flags: nowait postinstall
