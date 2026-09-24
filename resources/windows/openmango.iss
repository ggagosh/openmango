; Per-user OpenMango installer. Built by scripts/release_windows.sh:
;   ISCC /DAppVersion=0.2.2 /DArch=x86_64|arm64 /DSourceDir=<staged app> openmango.iss
#ifndef AppVersion
  #error Pass /DAppVersion, /DArch, and /DSourceDir
#endif

[Setup]
; Never change AppId: Windows uses it to recognize upgrades of the same app.
AppId={{E61F5184-E7BC-4EEB-9D8E-31082FC50C80}
AppName=OpenMango
AppVersion={#AppVersion}
AppVerName=OpenMango {#AppVersion}
AppPublisher=OpenMango
AppPublisherURL=https://openmango.app
AppSupportURL=https://github.com/ggagosh/openmango/issues
AppUpdatesURL=https://github.com/ggagosh/openmango/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\OpenMango
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0
SetupArchitecture=x64
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#endif
OutputBaseFilename=OpenMango-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile=openmango.ico
UninstallDisplayIcon={app}\OpenMango.exe
UninstallDisplayName=OpenMango
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Updates run after OpenMango exits; this closes any other copy still using its files.
CloseApplications=force
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; Replace bundled tools wholesale so an update never mixes versions.
Type: filesandordirs; Name: "{app}\bin"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\OpenMango"; Filename: "{app}\OpenMango.exe"
Name: "{autodesktop}\OpenMango"; Filename: "{app}\OpenMango.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\OpenMango.exe"; Description: "{cm:LaunchProgram,OpenMango}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; The Task Scheduler entry OpenMango adds for tasks that run while it's closed, if there is one.
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""OpenMango\Run due tasks"" /F"; Flags: runhidden; RunOnceId: "DeleteTasksRunner"
