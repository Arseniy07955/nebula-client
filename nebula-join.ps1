<#
.SYNOPSIS
  Nebula Client: подключает Windows-ПК к сети Nebula по инвайту из панели Nebula Control.

.DESCRIPTION
  Скачивает Nebula (фиксированная версия, проверка SHA256), создаёт ключ устройства,
  получает сертификат и config.yml от панели, ставит службу Nebula, правила брандмауэра
  и задачу планировщика, которая раз в 6 часов продлевает сертификат и обновляет конфиг.
  Приватный ключ не покидает этот ПК.

.EXAMPLE
  Подключиться (PowerShell от имени администратора):
  & ([scriptblock]::Create((irm 'https://example.org/nebula/v1/join.ps1'))) -Invite NF-XXXX-XXXX-XXXX

.EXAMPLE
  Состояние и список участников:   & "$env:ProgramData\NebulaClient\nebula-join.ps1" -Status
  Отключиться и всё удалить:       & "$env:ProgramData\NebulaClient\nebula-join.ps1" -Uninstall
#>
[CmdletBinding()]
param(
    [string]$Invite,
    [string]$Name,
    [string]$Server = '__SERVER__',
    [switch]$Sync,
    [switch]$Status,
    [switch]$Uninstall,
    # без службы, брандмауэра и планировщика: только ключ, сертификат и конфиг (для проверки)
    [switch]$NoService,
    [string]$DataDir = (Join-Path $env:ProgramData 'NebulaClient'),
    [switch]$Pause
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # с прогресс-баром Invoke-WebRequest качает в разы медленнее
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$ClientVersion = 'nebula-client/0.1'
$NebulaVersion = '1.9.3'
$NebulaBuilds = @{
    'AMD64' = @{ File = 'nebula-windows-amd64.zip'; Sha256 = '65ead0e8104a36e35230511e8d983458353a0edb6c4e332464da727b8efce11e' }
    'ARM64' = @{ File = 'nebula-windows-arm64.zip'; Sha256 = '599c49e2a1360eb448795e48b1afe9259bd79043d41e959f1e920d1a8238ef58' }
}
$Bin = Join-Path $DataDir 'bin'
$StateFile = Join-Path $DataDir 'device.json'
$SelfPath = Join-Path $DataDir 'nebula-join.ps1'
$LogFile = Join-Path $DataDir 'sync.log'
$ServiceName = 'Nebula'          # имя задаёт сам nebula.exe -service install
$TaskName = 'Nebula Client Sync'
$FwGroup = 'Nebula Client'
$Utf8 = New-Object System.Text.UTF8Encoding $false

function Say([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }
function Note([string]$Text) { Write-Host "    $Text" -ForegroundColor DarkGray }

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    return (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Write-Log([string]$Text) {
    if ((Test-Path $LogFile) -and (Get-Item $LogFile).Length -gt 1MB) { Remove-Item $LogFile -Force }
    Add-Content -Path $LogFile -Value ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Text) -Encoding UTF8
}

function Get-ApiError($Err) {
    $text = $null
    if ($Err.ErrorDetails -and $Err.ErrorDetails.Message) { $text = $Err.ErrorDetails.Message }
    elseif ($Err.Exception.Response) {
        try { $text = (New-Object IO.StreamReader($Err.Exception.Response.GetResponseStream())).ReadToEnd() } catch { }
    }
    $code = 0
    if ($Err.Exception.Response) { $code = [int]$Err.Exception.Response.StatusCode }
    $msg = $Err.Exception.Message
    if ($text) { try { $msg = ($text | ConvertFrom-Json).error } catch { $msg = $text } }
    return @{ Code = $code; Message = $msg }
}

function Invoke-Api([string]$Method, [string]$Path, $Body, [string]$Token) {
    $headers = @{}
    if ($Token) { $headers['Authorization'] = "Bearer $Token" }
    $req = @{ Uri = "$Server$Path"; Method = $Method; Headers = $headers; UseBasicParsing = $true; TimeoutSec = 30 }
    if ($null -ne $Body) {
        $req['ContentType'] = 'application/json; charset=utf-8'
        $req['Body'] = $Utf8.GetBytes(($Body | ConvertTo-Json -Compress))
    }
    try { return Invoke-RestMethod @req }
    catch {
        $e = Get-ApiError $_
        $ex = New-Object Exception($e.Message)
        $ex.Data['Code'] = $e.Code
        throw $ex
    }
}

function Set-FileText([string]$Path, [string]$Text) {
    # возвращает $true, если содержимое поменялось
    if ((Test-Path $Path) -and ([IO.File]::ReadAllText($Path) -eq $Text)) { return $false }
    [IO.File]::WriteAllText($Path, $Text, $Utf8)
    return $true
}

function Protect-DataDir {
    if (-not (Test-Admin)) { return }
    # доступ только SYSTEM и администраторам: здесь приватный ключ и токен устройства
    & icacls.exe $DataDir /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
}

function Install-NebulaBinaries {
    $marker = Join-Path $Bin 'VERSION'
    if ((Test-Path (Join-Path $Bin 'nebula.exe')) -and (Test-Path $marker) -and ((Get-Content $marker -Raw).Trim() -eq $NebulaVersion)) { return }
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    $build = $NebulaBuilds[$arch.ToUpper()]
    if (-not $build) { throw "Архитектура $arch не поддерживается, нужна AMD64 или ARM64" }
    $url = "https://github.com/slackhq/nebula/releases/download/v$NebulaVersion/$($build.File)"
    $zip = Join-Path ([IO.Path]::GetTempPath()) ("nebula-$NebulaVersion-" + [Guid]::NewGuid().ToString('N') + '.zip')
    Say "Скачиваю Nebula $NebulaVersion ($arch)"
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
        $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($hash -ne $build.Sha256) { throw "Контрольная сумма архива Nebula не совпала ($hash). Установка остановлена" }
        if (Test-Path $Bin) { Remove-Item $Bin -Recurse -Force }
        Expand-Archive -Path $zip -DestinationPath $Bin -Force
        Set-Content -Path $marker -Value $NebulaVersion -Encoding ASCII
    }
    finally { Remove-Item $zip -Force -ErrorAction SilentlyContinue }
}

function Save-DeviceFiles($R) {
    $changed = $false
    foreach ($pair in @(@('host.crt', $R.cert), @('ca.crt', $R.ca), @('config.yml', $R.config))) {
        if (Set-FileText (Join-Path $DataDir $pair[0]) ($pair[1] -replace "`r`n", "`n")) { $changed = $true }
    }
    if ($R.ssh_host_key) {
        if (Set-FileText (Join-Path $DataDir 'ssh_host_ed25519_key') ($R.ssh_host_key -replace "`r`n", "`n")) { $changed = $true }
    }
    return $changed
}

function Test-NebulaConfig {
    $out = & (Join-Path $Bin 'nebula.exe') -test -config (Join-Path $DataDir 'config.yml') 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Nebula не приняла конфиг: $($out | Select-Object -Last 3 | Out-String)" }
}

function Get-OwnService {
    $svc = Get-CimInstance Win32_Service -Filter "Name='$ServiceName'" -ErrorAction SilentlyContinue
    if (-not $svc) { return $null }
    if ($svc.PathName -notlike "*$Bin*") {
        throw "На этом ПК уже есть служба '$ServiceName' из другой установки Nebula ($($svc.PathName)). Её нужно убрать вручную"
    }
    return $svc
}

function Install-NebulaService {
    $exe = Join-Path $Bin 'nebula.exe'
    $cfg = Join-Path $DataDir 'config.yml'
    if (-not (Get-OwnService)) {
        Say 'Ставлю службу Nebula'
        & $exe -service install -config $cfg | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'nebula.exe не смог установить службу' }
        & sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/10000/restart/60000 | Out-Null
    }
    Restart-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
    if ((Get-Service $ServiceName).Status -ne 'Running') { Start-Service $ServiceName }
}

function Set-NetworkRules($Net) {
    Get-NetFirewallRule -Group $FwGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    $exe = Join-Path $Bin 'nebula.exe'
    # служба не может показать окно «разрешить доступ», поэтому разрешаем nebula.exe заранее
    New-NetFirewallRule -DisplayName 'Nebula Client: nebula.exe' -Group $FwGroup -Direction Inbound -Program $exe -Action Allow -Profile Any | Out-Null
    New-NetFirewallRule -DisplayName "Nebula Client: ping из $($Net.cidr)" -Group $FwGroup -Direction Inbound -Protocol ICMPv4 -IcmpType 8 `
        -RemoteAddress $Net.cidr -Action Allow -Profile Any | Out-Null
    # новый адаптер Windows считает общедоступной сетью и режет входящие, ставим «частную»
    for ($i = 0; $i -lt 20; $i++) {
        $prof = Get-NetConnectionProfile -InterfaceAlias $Net.dev -ErrorAction SilentlyContinue
        if ($prof) {
            if ($prof.NetworkCategory -ne 'Private') { Set-NetConnectionProfile -InterfaceAlias $Net.dev -NetworkCategory Private -ErrorAction SilentlyContinue }
            return
        }
        Start-Sleep -Seconds 1
    }
    Note "Адаптер $($Net.dev) пока не появился, тип сети поменяется при следующей синхронизации"
}

function Register-SyncTask {
    $act = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$SelfPath`" -Sync"
    $triggers = @(
        (New-ScheduledTaskTrigger -AtStartup),
        (New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(5) -RepetitionInterval (New-TimeSpan -Hours 6))
    )
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 10) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $TaskName -Action $act -Trigger $triggers -Principal $principal -Settings $settings -Force | Out-Null
}

function Save-SelfCopy {
    if ($PSCommandPath -and ((Resolve-Path $PSCommandPath).Path -eq $SelfPath)) { return }
    $text = if ($PSCommandPath) { [IO.File]::ReadAllText($PSCommandPath) } else { Invoke-RestMethod -Uri "$Server/v1/join.ps1" -UseBasicParsing }
    $text = $text -replace '__SERVER__', $Server
    # с BOM, иначе Windows PowerShell 5.1 прочитает русский текст в ANSI
    [IO.File]::WriteAllText($SelfPath, $text, (New-Object System.Text.UTF8Encoding $true))
}

function Show-Peers($Peers, [string]$MyIp) {
    if (-not $Peers) { return }
    Write-Host ''
    Write-Host 'Участники сети:'
    foreach ($p in $Peers) {
        $mark = if ($p.ip -eq $MyIp) { ' (это вы)' } elseif ($p.online) { ' в сети' } else { '' }
        Write-Host ("  {0,-16} {1}{2}" -f $p.ip, $p.name, $mark)
    }
}

function Default-Name {
    $n = ($env:COMPUTERNAME.ToLower() -replace '[^a-z0-9._-]', '-').Trim('-')
    if (-not $n) { $n = 'pc' }
    return $n
}

function Start-Elevated {
    $file = $PSCommandPath
    if (-not $file) {
        $file = Join-Path ([IO.Path]::GetTempPath()) 'nebula-join.ps1'
        $text = (Invoke-RestMethod -Uri "$Server/v1/join.ps1" -UseBasicParsing) -replace '__SERVER__', $Server
        [IO.File]::WriteAllText($file, $text, (New-Object System.Text.UTF8Encoding $true))
    }
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$file`"", '-Server', "`"$Server`"", '-Pause')
    if ($Invite) { $argv += @('-Invite', $Invite) }
    if ($Name) { $argv += @('-Name', "`"$Name`"") }
    if ($Status) { $argv += '-Status' }
    if ($Uninstall) { $argv += '-Uninstall' }
    Say 'Нужны права администратора, открываю окно с запросом UAC'
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argv | Out-Null
}

# ------------------------------------------------------------------ действия

function Invoke-Enroll {
    if (Test-Path $StateFile) {
        throw "Этот ПК уже подключён. Чтобы подключиться заново, сначала выполните: & `"$SelfPath`" -Uninstall"
    }
    if (-not $Invite) { $script:Invite = Read-Host 'Код инвайта (NF-XXXX-XXXX-XXXX)' }
    $def = Default-Name
    if (-not $Name) {
        $script:Name = Read-Host "Имя устройства латиницей [$def]"
        if (-not $Name) { $script:Name = $def }
    }
    New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
    Protect-DataDir
    Install-NebulaBinaries

    $key = Join-Path $DataDir 'host.key'
    $pub = Join-Path $DataDir 'host.pub'
    if (-not (Test-Path $key)) {
        Say 'Создаю ключ устройства'
        & (Join-Path $Bin 'nebula-cert.exe') keygen -out-key $key -out-pub $pub | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'nebula-cert не смог создать ключ' }
    }
    $confDir = $DataDir -replace '\\', '/'
    while ($true) {
        Say "Подключаюсь к $Server как $Name"
        try {
            $r = Invoke-Api POST '/v1/enroll' @{ invite = $Invite; name = $Name; public_key = [IO.File]::ReadAllText($pub); conf_dir = $confDir; client = $ClientVersion }
            break
        }
        catch {
            if ($_.Exception.Data['Code'] -eq 409 -and [Environment]::UserInteractive) {
                Write-Host $_.Exception.Message -ForegroundColor Yellow
                $script:Name = Read-Host 'Другое имя устройства'
                if (-not $Name) { throw 'Подключение отменено' }
                continue
            }
            throw
        }
    }
    Save-DeviceFiles $r | Out-Null
    $state = [ordered]@{ server = $Server; token = $r.token; name = $r.name; ip = $r.ip; network = $r.network; enrolled = (Get-Date).ToString('s') }
    Set-FileText $StateFile ($state | ConvertTo-Json) | Out-Null
    Save-SelfCopy
    Test-NebulaConfig

    if ($NoService) {
        Note 'Режим -NoService: служба, брандмауэр и планировщик не трогались'
    }
    else {
        Install-NebulaService
        Set-NetworkRules $r.network
        Register-SyncTask
    }
    Write-Host ''
    Write-Host "Готово. Сеть «$($r.network.title)», ваш адрес $($r.ip)" -ForegroundColor Green
    Note 'Друзей ищите по IP из списка ниже. Поиск игр «по локальной сети» через Nebula не работает, подключайтесь по IP'
    Show-Peers $r.peers $r.ip
}

function Read-State {
    if (-not (Test-Path $StateFile)) { throw 'Этот ПК ещё не подключён. Запустите скрипт с -Invite' }
    $s = Get-Content $StateFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $script:Server = $s.server
    return $s
}

function Invoke-Sync {
    $s = Read-State
    try {
        $r = Invoke-Api POST '/v1/sync' @{ conf_dir = ($DataDir -replace '\\', '/'); client = $ClientVersion } $s.token
    }
    catch {
        Write-Log "sync: ошибка $($_.Exception.Data['Code']): $($_.Exception.Message)"
        throw
    }
    if (Save-DeviceFiles $r) {
        Test-NebulaConfig
        if (-not $NoService -and (Get-OwnService)) { Restart-Service -Name $ServiceName -Force }
        Write-Log "sync: сертификат или конфиг обновлены, сертификат до $([DateTimeOffset]::FromUnixTimeSeconds($r.not_after).LocalDateTime)"
    }
    else { Write-Log 'sync: без изменений' }
    if (-not $NoService -and (Get-OwnService)) {
        if ((Get-Service $ServiceName).Status -ne 'Running') { Start-Service $ServiceName; Write-Log 'sync: служба была остановлена, запустил' }
        $prof = Get-NetConnectionProfile -InterfaceAlias $r.network.dev -ErrorAction SilentlyContinue
        if ($prof -and $prof.NetworkCategory -ne 'Private') { Set-NetConnectionProfile -InterfaceAlias $r.network.dev -NetworkCategory Private -ErrorAction SilentlyContinue }
    }
}

function Show-Status {
    $s = Read-State
    $svc = Get-Service $ServiceName -ErrorAction SilentlyContinue
    Write-Host "Устройство: $($s.name), адрес $($s.ip), сеть «$($s.network.title)» ($($s.network.cidr))"
    Write-Host ("Служба Nebula: {0}" -f $(if ($svc) { $svc.Status } else { 'не установлена' }))
    $crt = Join-Path $DataDir 'host.crt'
    if (Test-Path $crt) {
        $j = & (Join-Path $Bin 'nebula-cert.exe') print -json -path $crt | ConvertFrom-Json
        Write-Host "Сертификат до: $($j.details.notAfter)"
    }
    try { Show-Peers (Invoke-Api GET '/v1/peers' $null $s.token).peers $s.ip }
    catch { Write-Host "Панель недоступна: $($_.Exception.Message)" -ForegroundColor Yellow }
    if (Test-Path $LogFile) { Write-Host ''; Write-Host 'Последние синхронизации:'; Get-Content $LogFile -Tail 5 | ForEach-Object { Note $_ } }
}

function Invoke-Uninstall {
    Say 'Удаляю службу, задачу и правила брандмауэра'
    if (Get-OwnService) {
        $exe = Join-Path $Bin 'nebula.exe'
        & $exe -service stop 2>&1 | Out-Null
        & $exe -service uninstall 2>&1 | Out-Null
    }
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Get-NetFirewallRule -Group $FwGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    if (Test-Path $DataDir) {
        Set-Location $env:SystemRoot
        Remove-Item $DataDir -Recurse -Force
    }
    Write-Host 'Готово, Nebula Client удалён' -ForegroundColor Green
}

# ------------------------------------------------------------------ запуск

try {
    if (-not ($Sync -or $Status -or $Uninstall)) {
        if ($Server -eq ('__' + 'SERVER__') -or -not $Server) { throw 'Не задан адрес панели: -Server https://...' }
        if ($Server -notmatch '^https://' -and $Server -notmatch '^http://(127\.0\.0\.1|localhost)[:/]') { throw 'Адрес панели должен быть https://' }
        $Server = $Server.TrimEnd('/')
    }
    if (-not $NoService -and -not (Test-Admin)) {
        if ($Sync) { throw 'Синхронизация запускается планировщиком от SYSTEM' }
        Start-Elevated
        return
    }
    if ($Uninstall) { Invoke-Uninstall }
    elseif ($Status) { Show-Status }
    elseif ($Sync) { Invoke-Sync }
    else { Invoke-Enroll }
}
catch {
    Write-Host ''
    Write-Host "Ошибка: $($_.Exception.Message)" -ForegroundColor Red
    if ($Sync) { exit 1 }
}
finally {
    if ($Pause) { Write-Host ''; Read-Host 'Нажмите Enter, чтобы закрыть окно' | Out-Null }
}
