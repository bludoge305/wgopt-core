@echo off
setlocal
cd /d "%~dp0"

rem ASCII-only output: cmd under GBK code page mangles Chinese.

set "PATH=%~dp0.toolchain\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo   === build wgopt.exe ===
echo.
cargo build --release
if errorlevel 1 (
  echo   [ERROR] build failed
  exit /b 1
)

echo.
echo   === deploy tray icons ===
if not exist "target\release\tray" mkdir "target\release\tray"
copy /y "assets\tray\*.ico" "target\release\tray\" >nul
if errorlevel 1 (
  echo   [ERROR] failed to copy icons
  exit /b 1
)
dir /b "target\release\tray"

echo.
echo   binary   : target\release\wgopt.exe
echo   installer: build-installer.cmd
echo.
