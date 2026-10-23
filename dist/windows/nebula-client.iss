; Установщик Nebula Client для Windows (Inno Setup 6).
; Собирается скриптом dist/windows/build.ps1, он же кладёт файлы в dist/windows/payload.

#ifndef Version
  #define Version "0.3.0"
#endif
#define Payload "payload"

[Setup]
AppId={{8C1B6E4A-3F0D-4E5B-9A57-6B1C2D9E4F10}
AppName=Nebula Client
AppVersion={#Version}
AppVerName=Nebula Client {#Version}
AppPublisher=Arseniy07955
AppPublisherURL=https://github.com/Arseniy07955/nebula-client
AppSupportURL=https://github.com/Arseniy07955/nebula-client/issues
VersionInfoVersion={#Version}
DefaultDirName={autopf}\Nebula Client
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=nebula-client.ico
UninstallDisplayIcon={app}\nebula-client.exe
UninstallDisplayName=Nebula Client
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
; HKCU нужен намеренно: автозапуск значка для того, кто ставит программу
UsedUserAreasWarning=no
OutputDir=..\out
OutputBaseFilename=nebula-client-{#Version}-setup

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
russian.Autostart=Запускать значок в трее при входе в Windows
english.Autostart=Start the tray icon at Windows sign-in
russian.InstallService=Устанавливаю службу Nebula Client…
english.InstallService=Installing the Nebula Client service…
russian.Launch=Открыть Nebula Client
english.Launch=Open Nebula Client
russian.Purge=Стереть ключи и настройки всех сетей?%n%nЕсли нажать «Да», устройство выйдет из сетей на панели и при повторной установке подключится заново с новым адресом. «Нет» оставит данные в ProgramData\NebulaClient.
english.Purge=Delete the keys and settings of all networks?%n%n"Yes" makes the device leave its networks on the panel; a reinstall will join again with a new address. "No" keeps the data in ProgramData\NebulaClient.
russian.ServiceFailed=Служба не установилась (код %1). Запустите установщик ещё раз или выполните «nebula-client-service.exe install» от имени администратора.
english.ServiceFailed=The service was not installed (code %1). Run the installer again or run "nebula-client-service.exe install" as administrator.

[Tasks]
Name: "autostart"; Description: "{cm:Autostart}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Payload}\nebula-client.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\nebula-client-service.exe"; DestDir: "{app}"; Flags: ignoreversion
; официальный архив Nebula: служба проверит SHA256 и не пойдёт за ним на GitHub
Source: "{#Payload}\nebula-windows-amd64.zip"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[InstallDelete]
; остался от установки через install.cmd (v0.2)
Type: files; Name: "{app}\uninstall.cmd"

[Icons]
Name: "{autoprograms}\Nebula Client"; Filename: "{app}\nebula-client.exe"
Name: "{autodesktop}\Nebula Client"; Filename: "{app}\nebula-client.exe"; Tasks: desktopicon; Check: not UpdateMode

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Nebula Client"; ValueData: """{app}\nebula-client.exe"""; Flags: uninsdeletevalue; Tasks: autostart; Check: not UpdateMode

[Run]
Filename: "{app}\nebula-client.exe"; Description: "{cm:Launch}"; Flags: nowait postinstall runasoriginaluser skipifsilent

[Code]
{ /UPDATE=1: тихое обновление от службы (она работает от SYSTEM). Её HKCU и рабочий стол
  не трогаем, значок в трей служба вернёт сама. }
function UpdateMode: Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;

procedure Run(const Exe, Params: String; var Code: Integer);
begin
  if not Exec(Exe, Params, '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := -1;
end;

{ Значок, окно и служба держат файлы: перед копированием всё останавливается.
  nebula.exe живёт в job-объекте службы и закрывается вместе с ней. }
procedure StopEverything;
var
  Code: Integer;
begin
  Run(ExpandConstant('{sys}\taskkill.exe'), '/IM nebula-client.exe /F', Code);
  Run(ExpandConstant('{sys}\net.exe'), 'stop NebulaClient', Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopEverything;
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    WizardForm.StatusLabel.Caption := CustomMessage('InstallService');
    { install создаёт службу или обновляет путь у существующей и запускает её }
    Run(ExpandConstant('{app}\nebula-client-service.exe'), 'install', Code);
    if Code <> 0 then
      SuppressibleMsgBox(FmtMessage(CustomMessage('ServiceFailed'), [IntToStr(Code)]), mbError, MB_OK, IDOK);
  end;
end;

var
  Purge: Boolean;

function InitializeUninstall(): Boolean;
begin
  Purge := False;
  if not UninstallSilent then
    Purge := MsgBox(CustomMessage('Purge'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
  Params: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    Run(ExpandConstant('{sys}\taskkill.exe'), '/IM nebula-client.exe /F', Code);
    Params := 'uninstall';
    if Purge then
      Params := Params + ' --purge';
    Run(ExpandConstant('{app}\nebula-client-service.exe'), Params, Code);
    { автозапуск мог включить сам значок в трее, а не установщик }
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Nebula Client');
  end;
end;
