@echo off
setlocal enabledelayedexpansion
cd /d "%~dp0"

rem read version via Python: batch parsing of Cargo.toml is fragile
for /f "delims=" %%v in ('python -c "import re;t=open('Cargo.toml',encoding='utf-8').read();m=re.search(r'(?m)^version\s*=\s*\x22([^\x22]+)\x22',t);print(m.group(1) if m else '')"') do set "VER=%%v"

if "%VER%"=="" (
  echo   [ERROR] cannot read version from Cargo.toml
  exit /b 1
)

rem four-part version for exe resource info
for /f "tokens=1,2,3 delims=." %%a in ("%VER%") do set "VERQUAD=%%a.%%b.%%c.0"

rem prefer the NSIS copy cached by Tauri (same one the original installer uses)
set "MAKENSIS="
if defined NSIS_HOME if exist "%NSIS_HOME%\makensis.exe" set "MAKENSIS=%NSIS_HOME%\makensis.exe"
if not defined MAKENSIS if exist "%LOCALAPPDATA%\tauri\NSIS\makensis.exe" set "MAKENSIS=%LOCALAPPDATA%\tauri\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%ProgramFiles(x86)%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles(x86)%\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%ProgramFiles%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles%\NSIS\makensis.exe"
if not defined MAKENSIS (
  echo   [ERROR] makensis.exe not found; install NSIS or set NSIS_HOME
  exit /b 1
)

if not exist "target\release\wgopt.exe" (
  echo   [ERROR] target\release\wgopt.exe missing; run deploy.cmd first
  exit /b 1
)

set "ROOT=%CD%"
set "OUTDIR=%ROOT%\target\release\bundle"
if not exist "%OUTDIR%" mkdir "%OUTDIR%"
set "OUTFILE=%OUTDIR%\WGOpt-Core-%VER%-x64-setup.exe"

echo.
echo   === building installer WGOpt Core %VER% ===
echo   NSIS  : %MAKENSIS%
echo   output: %OUTFILE%
echo.

"%MAKENSIS%" /V2 /DSRCDIR="%ROOT%" /DVERSION="%VER%" /DVERSIONQUAD="%VERQUAD%" /DOUTFILE="%OUTFILE%" "installer\installer.nsi"
if errorlevel 1 (
  echo.
  echo   [ERROR] installer build failed
  exit /b 1
)

rem release notes: PowerShell writes them; cmd echo would mangle UTF-8 Chinese
set "NOTES=%OUTDIR%\release-notes-%VER%.md"
pwsh -NoProfile -ExecutionPolicy Bypass -File "scripts\write-notes.ps1" -Version %VER% -OutFile "%NOTES%"
if errorlevel 1 echo   [WARN] release notes generation failed

echo.
for %%f in ("%OUTFILE%") do echo   installer: %%~nxf  ^(%%~zf bytes^)
echo   notes    : release-notes-%VER%.md
echo.
