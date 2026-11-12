@echo off
setlocal
cd /d "%~dp0"

rem ASCII-only output: cmd under GBK code page mangles Chinese.

rem PowerShell must be 7 (pwsh). 5.1 parses .ps1 as GBK and chokes on the
rem Chinese strings in those scripts. No fallback: missing pwsh is an error.
set "PWSH=%ProgramFiles%\PowerShell\7\pwsh.exe"
if not exist "%PWSH%" (
  where pwsh >nul 2>nul || (
    echo   [ERROR] pwsh 7 not found; install PowerShell 7
    exit /b 1
  )
  set "PWSH=pwsh.exe"
)

if "%~1"=="" (
  echo   usage: release.cmd ^<version^> "commit message"
  echo   example: release.cmd 1.0.1 "fix: something"
  exit /b 1
)

set "VER=%~1"
set "MSG=%~2"
if "%MSG%"=="" set "MSG=release %VER%"

set "PATH=%~dp0.toolchain\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo   === WGOpt Core release v%VER% ===
echo.

echo   [1/7] sync version numbers...
"%PWSH%" -NoProfile -ExecutionPolicy Bypass -File "scripts\bump-version.ps1" %VER%
if errorlevel 1 (
  echo   [ERROR] version sync failed
  exit /b 1
)

echo   [2/7] running tests...
cargo test --quiet
if errorlevel 1 (
  echo   [ERROR] tests failed
  exit /b 1
)

echo   [3/7] building release binary...
cargo build --release
if errorlevel 1 (
  echo   [ERROR] build failed
  exit /b 1
)

echo   [4/7] deploying tray icons...
if not exist "target\release\tray" mkdir "target\release\tray"
copy /y "assets\tray\*.ico" "target\release\tray\" >nul

echo   [5/7] building installer...
call build-installer.cmd
if errorlevel 1 (
  echo   [ERROR] installer build failed
  exit /b 1
)

echo   [6/7] committing and tagging...
git add -A
git commit -q -m "%MSG%"
if errorlevel 1 echo   [note] nothing to commit, continuing
git tag -a "v%VER%" -m "WGOpt Core v%VER% - %MSG%"
if errorlevel 1 (
  echo   [ERROR] tag creation failed ^(already exists?^)
  exit /b 1
)

echo   [7/7] pushing to GitHub...
git push origin main
if errorlevel 1 (
  echo   [ERROR] push failed
  exit /b 1
)
git push origin "v%VER%"
if errorlevel 1 (
  echo   [ERROR] tag push failed
  exit /b 1
)

echo.
echo   done: v%VER% released.
echo   binary   : target\release\wgopt.exe
echo   installer: target\release\bundle\WGOpt-Core-%VER%-x64-setup.exe
echo.
echo   to attach the installer to a GitHub Release:
echo     scripts\publish-release.cmd %VER%
echo.
