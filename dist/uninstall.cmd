@echo off
chcp 65001 >nul
rem Nebula Client: удаляет службу, значок, автозапуск и все данные устройства.
net session >nul 2>&1 || (powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'" & exit /b)
set "DEST=%ProgramFiles%\Nebula Client"
taskkill /IM nebula-client.exe /F >nul 2>&1
"%DEST%\nebula-client-service.exe" uninstall --purge
reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "Nebula Client" /f >nul 2>&1
cd /d "%TEMP%"
rem одной строкой: cmd дочитает её до того, как файл скрипта исчезнет вместе с папкой
(rmdir /S /Q "%DEST%") & echo Nebula Client удалён. & timeout /t 5 >nul & exit /b 0
