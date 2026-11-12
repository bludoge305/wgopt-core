@echo off
rem Put the project-local MinGW-w64 toolchain on PATH for the current shell.
rem Must be invoked with `call` so the environment survives:
rem
rem   call scripts\use-toolchain.cmd
rem   cargo build --release
rem
rem Why this exists: the `windows` crate links through MinGW, and it needs
rem as.exe + dlltool.exe. Rust's bundled self-contained dlltool cannot find
rem as.exe, so linking fails with "error calling dlltool 'dlltool.exe':
rem program not found" unless a full MinGW-w64 is on PATH.
rem
rem Get the toolchain first with: scripts\fetch-toolchain.cmd
rem
rem ASCII-only output: cmd under a GBK code page mangles Chinese.

setlocal
cd /d "%~dp0.."

set "MINGW=%CD%\.toolchain\mingw64"

if not exist "%MINGW%\bin\dlltool.exe" (
  echo   [ERROR] toolchain not found at %MINGW%
  echo   run: scripts\fetch-toolchain.cmd
  endlocal & exit /b 1
)

rem endlocal before exporting: PATH must escape the setlocal scope.
rem The outer script reads these back after the call.
endlocal & set "PATH=%MINGW%\bin;%PATH%" & set "MINGW_PREFIX=%MINGW%"

echo   toolchain on PATH: %MINGW_PREFIX%
exit /b 0
