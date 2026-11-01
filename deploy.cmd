@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

set "PATH=%~dp0.toolchain\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo   === 编译 wgopt.exe ===
echo.
cargo build --release
if errorlevel 1 (
  echo   [错误] 编译失败
  exit /b 1
)

echo.
echo   === 部署托盘图标 ===
if not exist "target\release\tray" mkdir "target\release\tray"
copy /y "assets\tray\*.ico" "target\release\tray\" >nul
if errorlevel 1 (
  echo   [错误] 复制图标失败
  exit /b 1
)
dir /b "target\release\tray"

echo.
echo   完成：target\release\wgopt.exe
echo   运行：target\release\wgopt.exe
echo.
