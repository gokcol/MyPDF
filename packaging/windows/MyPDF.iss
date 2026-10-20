; The MyPDF installer for 64-bit Windows, built with Inno Setup 6 by the
; "Windows build" workflow:
;   ISCC /DAppVersion=1.5.1 /DSource=pack\MyPDF /O<folder> /F<file name> MyPDF.iss
; It installs for the current user without asking for administrator rights
; (or for everyone, if chosen), adds a Start menu entry and an uninstaller,
; and, when ticked, lists MyPDF in the "Open with" menu for PDF files.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\pack\MyPDF"
#endif

[Setup]
AppId={{8C4E2F6A-5B1D-4C7E-9A2F-3D6B1E0F7A51}
AppName=MyPDF
AppVersion={#AppVersion}
AppVerName=MyPDF {#AppVersion}
AppPublisher=MyPDF contributors
AppPublisherURL=https://github.com/gokcol/MyPDF
AppSupportURL=https://github.com/gokcol/MyPDF/issues
AppUpdatesURL=https://github.com/gokcol/MyPDF/releases
VersionInfoProductName=MyPDF
VersionInfoProductVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
VersionInfoDescription=MyPDF installer
VersionInfoCopyright=© 2026 MyPDF contributors · MIT OR Apache-2.0
DefaultDirName={autopf}\MyPDF
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=MyPDF.ico
UninstallDisplayIcon={app}\MyPDF.exe
UninstallDisplayName=MyPDF
InfoBeforeFile=before-install.txt
ChangesAssociations=yes
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"

[CustomMessages]
english.OpenWith=Add MyPDF to the "Open with" menu for PDF files
turkish.OpenWith=MyPDF'i PDF dosyalarının "Birlikte aç" menüsüne ekle
english.Integration=Windows integration:
turkish.Integration=Windows ile bütünleşme:

[Tasks]
Name: "openwith"; Description: "{cm:OpenWith}"; GroupDescription: "{cm:Integration}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\MyPDF.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
; The bridge statically links CTranslate2/SentencePiece and the MSVC runtime.
; These explicit entries make a missing runtime or its notices a build failure.
Source: "{#Source}\translation\mypdf_ct2.dll"; DestDir: "{app}\translation"; Flags: ignoreversion
Source: "{#Source}\translation\mypdf_ct2.dll.sha256"; DestDir: "{app}\translation"; Flags: ignoreversion
Source: "{#Source}\translation\licenses\*"; DestDir: "{app}\translation\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\MyPDF"; Filename: "{app}\MyPDF.exe"
Name: "{autodesktop}\MyPDF"; Filename: "{app}\MyPDF.exe"; Tasks: desktopicon

[Registry]
; "Open with" for PDF files. The default PDF app stays as it is, and
; uninstalling removes these entries again.
Root: HKA; Subkey: "Software\Classes\Applications\MyPDF.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "MyPDF"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\MyPDF.exe\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\MyPDF.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\MyPDF.exe"" ""%1"""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\MyPDF.Document"; ValueType: string; ValueName: ""; ValueData: "PDF Document"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\MyPDF.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\MyPDF.exe,0"; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\MyPDF.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\MyPDF.exe"" ""%1"""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "MyPDF.Document"; ValueData: ""; Flags: uninsdeletevalue; Tasks: openwith

[Run]
Filename: "{app}\MyPDF.exe"; Description: "{cm:LaunchProgram,MyPDF}"; Flags: nowait postinstall skipifsilent
