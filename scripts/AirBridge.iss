; Compile through Prepare-Release.ps1, which supplies version and staging paths.
[Setup]
AppId={{5C265431-42D6-4F51-B04E-A32A57202BE1}
AppName=AirBridge
AppVersion={#AppVersion}
AppVerName=AirBridge {#AppVersion}
DefaultDirName={localappdata}\Programs\AirBridge
DefaultGroupName=AirBridge
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#ReleaseDir}
OutputBaseFilename=AirBridge-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icons\AirBridge.ico
UninstallDisplayIcon={app}\AirBridge.exe
LicenseFile=..\LICENSE
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\AirBridge"; Filename: "{app}\AirBridge.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\AirBridge"; Filename: "{app}\AirBridge.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\AirBridge.exe"; Description: "{cm:LaunchProgram,AirBridge}"; Flags: nowait postinstall skipifsilent

; Do not delete settings, identities or logs during uninstall.
