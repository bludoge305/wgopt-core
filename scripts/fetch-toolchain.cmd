@echo off
rem Fetch the MinGW-w64 toolchain used to build WGOpt Core.
rem
rem The toolchain is ~500 MB (11k files), so it is NOT committed to this repo.
rem It lives in .toolchain\ (gitignored) and this script downloads the exact
rem version so every machine builds with the same linker as.exe / dlltool.exe.
rem
rem ASCII-only output: cmd under a GBK code page mangles Chinese.
rem
rem Usage:  scripts\fetch-toolchain.cmd
rem         scripts\fetch-toolchain.cmd /force    (re-download even if present)

setlocal enabledelayedexpansion
cd /d "%~dp0.."

rem ---- pinned version -------------------------------------------------------
rem winlibs personal build: GCC 16.2.0, MinGW-w64 14.0.0 UCRT, posix threads,
rem SEH unwinding. Verified against the toolchain the project was developed on
rem (`gcc -v` reports "MinGW-W64 x86_64-ucrt-posix-seh, built by Brecht Sanders, r3").
set "WL_VER=16.2.0posix-23.1.3-14.0.0-ucrt-r3"
set "WL_FILE=winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3"
set "WL_URL=https://github.com/brechtsanders/winlibs_mingw/releases/download/%WL_VER%/%WL_FILE%.zip"

set "ROOT=%~dp0.."
for %%i in ("%ROOT%") do set "ROOT=%%~fi"
set "DEST=%ROOT%\.toolchain"
set "MINGW=%DEST%\mingw64"

rem ---- already present? -----------------------------------------------------
if exist "%MINGW%\bin\dlltool.exe" if exist "%MINGW%\bin\as.exe" (
  if /i not "%~1"=="/force" (
    echo   toolchain already present: %MINGW%
    echo   use /force to re-download
    exit /b 0
  )
)

rem ---- download -------------------------------------------------------------
if not exist "%DEST%" mkdir "%DEST%"
set "ZIP=%DEST%\%WL_FILE%.zip"

echo.
echo   === downloading MinGW-w64 toolchain ===
echo   url : %WL_URL%
echo   dest: %ZIP%
echo.

rem Try curl first (Windows 10+ ships it), fall back to PowerShell.
where curl.exe >nul 2>nul
if not errorlevel 1 (
  curl.exe -L --fail --retry 3 -o "%ZIP%" "%WL_URL%"
) else (
  powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$ErrorActionPreference='Stop';" ^
    "Invoke-WebRequest -Uri '%WL_URL%' -OutFile '%ZIP%' -UseBasicParsing"
)
if errorlevel 1 (
  echo   [ERROR] download failed
  if exist "%ZIP%" del "%ZIP%"
  exit /b 1
)

rem ---- extract --------------------------------------------------------------
echo   extracting...
rem Expand-Archive is built into PowerShell 5.1+; avoids requiring 7-Zip.
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference='Stop';" ^
  "Expand-Archive -Path '%ZIP%' -DestinationPath '%DEST%' -Force"
if errorlevel 1 (
  echo   [ERROR] extraction failed
  echo   the archive is kept at %ZIP%
  exit /b 1
)

del "%ZIP%" >nul 2>nul

rem ---- verify ---------------------------------------------------------------
if not exist "%MINGW%\bin\dlltool.exe" (
  echo   [ERROR] extraction did not produce %MINGW%\bin\dlltool.exe
  echo   check the archive layout under %DEST%
  exit /b 1
)

echo.
echo   toolchain ready: %MINGW%
echo.
echo   next steps:
echo     call scripts\use-toolchain.cmd     :: put it on PATH for this shell
echo     deploy.cmd                          :: build
exit /b 0
