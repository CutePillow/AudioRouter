#define MyAppName "AudioRouter"
#ifndef MyAppVersion
#define MyAppVersion "1.1.0"
#endif
#define MyAppPublisher "AudioRouter"
#define MyAppExeName "audiorouter-desktop.exe"

[Setup]
AppId={{B4C7D6E2-6B5B-4A5C-9C38-5A8C6C9E4C31}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\AudioRouter
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\Self-disk
OutputBaseFilename=AudioRouter-Setup-{#MyAppVersion}
SetupIconFile=..\assets\icons\audiorouter-icon.ico
UninstallDisplayIcon={app}\audiorouter.ico
UninstallDisplayName={#MyAppName}
WizardStyle=modern
WizardSizePercent=100
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
AppMutex=Local\AudioRouter.Desktop.SingleInstance

[Languages]
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加快捷方式："; Flags: unchecked

[Files]
Source: "..\target\release\audiorouter-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion restartreplace
Source: "..\assets\icons\audiorouter-icon.ico"; DestDir: "{app}"; DestName: "audiorouter.ico"; Flags: ignoreversion

[InstallDelete]
Type: filesandordirs; Name: "{app}\audiorouter.ico"

[Dirs]
Name: "{app}\data"; Permissions: users-modify

[Icons]
Name: "{group}\AudioRouter"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\audiorouter.ico"
Name: "{group}\卸载 AudioRouter"; Filename: "{uninstallexe}"; IconFilename: "{app}\audiorouter.ico"
Name: "{autodesktop}\AudioRouter"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\audiorouter.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 AudioRouter"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
