@echo off
setlocal
cd /d "%~dp0.."
rem ASCII-only output on purpose: cmd under GBK code page mangles Chinese.
if "%~1"=="" (
  echo   usage: publish-release.cmd ^<version^>
  exit /b 1
)
set "VER=%~1"
set "SETUP=target\release\bundle\WGOpt-Core-%VER%-x64-setup.exe"
set "NOTES=target\release\bundle\release-notes-%VER%.md"
if not exist "%SETUP%" (
  echo   [ERROR] installer not found: %SETUP%
  echo   run release.cmd %VER% first
  exit /b 1
)
where gh >nul 2>nul
if errorlevel 1 (
  echo   [ERROR] gh CLI not found
  exit /b 1
)
if not exist "%NOTES%" (
  echo   [ERROR] release notes not found: %NOTES%
  exit /b 1
)
echo.
echo   creating GitHub release v%VER%...
echo.
gh release create "v%VER%" "%SETUP%" --title "WGOpt Core v%VER%" --notes-file "%NOTES%" --latest
if errorlevel 1 (
  echo   [ERROR] release creation failed
  exit /b 1
)
echo.
echo   done.
echo.
