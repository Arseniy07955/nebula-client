# Сборка установщика: cargo build, официальный архив Nebula с проверкой SHA256, ISCC.
#   powershell -ExecutionPolicy Bypass -File dist\windows\build.ps1 [-Iscc путь\ISCC.exe] [-NoBuild]
param(
    [string]$Iscc = $(if ($env:ISCC) { $env:ISCC } else { "ISCC.exe" }),
    [switch]$NoBuild
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$root = Resolve-Path "$here\..\.."
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value

# должно совпадать с crates/service/src/nebula.rs
$nebulaVer = "1.9.3"
$zip = "nebula-windows-amd64.zip"
$sum = "65ead0e8104a36e35230511e8d983458353a0edb6c4e332464da727b8efce11e"

if (-not $NoBuild) {
    Push-Location $root
    cargo build --release
    if ($LASTEXITCODE) { throw "cargo build failed" }
    Pop-Location
}

$payload = "$here\payload"
New-Item -ItemType Directory -Force $payload | Out-Null
Copy-Item "$root\target\release\nebula-client.exe", "$root\target\release\nebula-client-service.exe" $payload -Force

$zipPath = "$payload\$zip"
if (-not (Test-Path $zipPath) -or (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower() -ne $sum) {
    Write-Host "Скачиваю Nebula $nebulaVer"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest "https://github.com/slackhq/nebula/releases/download/v$nebulaVer/$zip" -OutFile $zipPath -UseBasicParsing
}
$got = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
if ($got -ne $sum) { throw "SHA256 $zip не совпал: $got" }

& $Iscc "/DVersion=$version" "$here\nebula-client.iss"
if ($LASTEXITCODE) { throw "ISCC failed" }

# SHA256SUMS.txt по всем файлам релиза: по нему служба проверяет скачанное обновление
$out = "$root\dist\out"
$lines = Get-ChildItem $out -File | Where-Object Name -ne "SHA256SUMS.txt" | Sort-Object Name |
    ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name }
[IO.File]::WriteAllText("$out\SHA256SUMS.txt", ($lines -join "`n") + "`n")
Get-Item "$out\nebula-client-$version-setup.exe", "$out\SHA256SUMS.txt"
