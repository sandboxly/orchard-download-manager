; Windows installer (Inno Setup 6). Built in CI from the staged app folder
; (packaging/stage.sh win-x64 ... dist\orchard-download-manager):
;   iscc /DAppVersion=1.0.0 packaging\windows\orchard-download-manager.iss
; Output: dist\Orchard-Download-Manager-Setup.exe
; Installs for the current user or for all users (the user chooses; /CURRENTUSER
; or /ALLUSERS on the command line), adds a Start menu entry, an optional
; desktop shortcut and an uninstaller. Silent installs:
; /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-
; Keep the AppId: later versions upgrade an installation in place with it.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{7C8BB7BB-8001-44C5-91BF-AB4A0D533DA9}
AppName=Orchard Download Manager
AppVersion={#AppVersion}
AppVerName=Orchard Download Manager {#AppVersion}
AppPublisher=Stephan Lengl
AppPublisherURL=https://github.com/sandboxly/orchard-download-manager
AppSupportURL=https://github.com/sandboxly/orchard-download-manager/issues
AppUpdatesURL=https://github.com/sandboxly/orchard-download-manager/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Orchard Download Manager
DefaultGroupName=Orchard Download Manager
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=..\icons\app.ico
UninstallDisplayIcon={app}\orchard-download-manager.ico
UninstallDisplayName=Orchard Download Manager
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=Orchard-Download-Manager-Setup
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\dist\orchard-download-manager\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
; The same icon as the one in the executable, as a file: shortcuts and Apps &
; features point at it.
Source: "..\icons\app.ico"; DestDir: "{app}"; DestName: "orchard-download-manager.ico"; Flags: ignoreversion

[InstallDelete]
; A new version replaces the license folder completely.
Type: filesandordirs; Name: "{app}\licenses"

[Registry]
; The app registers itself with the browsers for their extension (under
; HKCU, on every start; see src/native_host.rs). Only removed here, on
; uninstall: never created by the installer.
Root: HKCU; Subkey: "Software\Google\Chrome\NativeMessagingHosts\io.github.sandboxly.orchard_download_manager"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Chromium\NativeMessagingHosts\io.github.sandboxly.orchard_download_manager"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Edge\NativeMessagingHosts\io.github.sandboxly.orchard_download_manager"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\io.github.sandboxly.orchard_download_manager"; Flags: uninsdeletekey dontcreatekey

[Icons]
Name: "{autoprograms}\Orchard Download Manager"; Filename: "{app}\orchard-download-manager.exe"; IconFilename: "{app}\orchard-download-manager.ico"
Name: "{autodesktop}\Orchard Download Manager"; Filename: "{app}\orchard-download-manager.exe"; IconFilename: "{app}\orchard-download-manager.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\orchard-download-manager.exe"; Description: "{cm:LaunchProgram,Orchard Download Manager}"; Flags: nowait postinstall skipifsilent
