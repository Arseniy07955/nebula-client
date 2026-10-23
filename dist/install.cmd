@echo off
chcp 65001 >nul
rem Nebula Client: копирует программу в Program Files, ставит службу и значок в трее.
net session >nul 2>&1 || (powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'" & exit /b)
set "DEST=%ProgramFiles%\Nebula Client"
echo Устанавливаю Nebula Client в "%DEST%"
taskkill /IM nebula-client.exe /F >nul 2>&1
sc stop NebulaClient >nul 2>&1 && timeout /t 3 /nobreak >nul
if not exist "%DEST%" mkdir "%DEST%"
copy /Y "%~dp0nebula-client.exe" "%DEST%\" >nul || goto fail
copy /Y "%~dp0nebula-client-service.exe" "%DEST%\" >nul || goto fail
copy /Y "%~dp0uninstall.cmd" "%DEST%\" >nul
"%DEST%\nebula-client-service.exe" install || goto fail
reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "Nebula Client" /t REG_SZ /d "\"%DEST%\nebula-client.exe\"" /f >nul
rem через explorer, чтобы значок запустился без прав администратора
start "" explorer.exe "%DEST%\nebula-client.exe"
echo Готово. Значок Nebula Client появится в трее, нажмите на него, чтобы подключиться.
timeout /t 5 >nul
exit /b 0
:fail
echo Установка не удалась.
pause
exit /b 1
