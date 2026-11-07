@echo off
rem Instala o Halo Studio para este usuario (sem admin), liga o inicio com o
rem Windows e abre o app. Rode com DOIS CLIQUES no Explorer: assim tudo vai
rem para o AppData de verdade (o que roda de dentro do Claude fica numa copia
rem particular do app Claude, que o Windows nao ve no login).
setlocal
cd /d "%~dp0"
set "SETUP=%~dp0src-tauri\target\release\bundle\nsis\Halo Studio_0.1.0_x64-setup.exe"
set "EXE=%LOCALAPPDATA%\Halo Studio\halo-studio.exe"
set "CFGDIR=%APPDATA%\com.evert.halostudio"
set "SAVED=%~dp0dev\config-do-halo.json"

if not exist "%SETUP%" (
  echo Nao achei o instalador. Rode "npm.cmd run build" antes.
  pause
  exit /b 1
)

echo Fechando o Halo Studio, se estiver aberto...
taskkill /im halo-studio.exe /f >nul 2>&1
timeout /t 1 /nobreak >nul

echo Instalando o Halo Studio...
"%SETUP%" /S
if errorlevel 1 (
  echo A instalacao falhou.
  pause
  exit /b 1
)

rem Traz as configuracoes (saidas, mapa da fan, efeitos) se ainda nao houver.
if not exist "%CFGDIR%\config.json" if exist "%SAVED%" (
  echo Copiando as configuracoes...
  mkdir "%CFGDIR%" 2>nul
  copy /y "%SAVED%" "%CFGDIR%\config.json" >nul
)

echo Ligando o inicio com o Windows...
reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "Halo Studio" /t REG_SZ /d "\"%EXE%\" --autostart" /f >nul

echo Abrindo o Halo Studio na bandeja (perto do relogio)...
start "" "%EXE%" --autostart
echo.
echo Pronto. O Halo Studio esta na bandeja, religou as luzes e vai iniciar sozinho com o Windows.
echo Para desligar: Halo Studio, Configuracoes, Iniciar com o Windows.
timeout /t 8 >nul
