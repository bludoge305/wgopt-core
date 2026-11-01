@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0.."

if "%~1"=="" (
  echo   用法: publish-release.cmd ^<版本号^>
  exit /b 1
)

set "VER=%~1"
set "EXE=target\release\wgopt.exe"

if not exist "%EXE%" (
  echo   [错误] 找不到 %EXE%，请先运行 release.cmd
  exit /b 1
)

where gh >nul 2>nul
if errorlevel 1 (
  echo   [错误] 未找到 gh CLI
  exit /b 1
)

echo.
echo   创建 GitHub Release v%VER%...
echo.
gh release create "v%VER%" "%EXE%" ^
  --title "WGOpt Core v%VER%" ^
  --notes "无界面游戏优化后台程序 v%VER%。下载 wgopt.exe 后与 tray 图标目录放在一起运行。" ^
  --latest

if errorlevel 1 (
  echo   [错误] 创建 Release 失败
  exit /b 1
)

echo.
echo   完成。注意：exe 需要与 tray\ 图标目录一起分发，否则托盘图标会回退到系统默认图标。
echo.
