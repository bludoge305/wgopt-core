@echo off

setlocal

cd /d "%~dp0"



rem 本脚本刻意只用 ASCII 输出：cmd 在 GBK 代码页下会吃掉中文，

rem 之前踩过好几次，报错信息都变成乱码，得不偿失。



if "%~1"=="" (

  echo   usage: release.cmd ^<version^> "commit message"

  echo   example: release.cmd 1.0.1 "fix: something"

  exit /b 1

)



set "VER=%~1"

set "MSG=%~2"

if "%MSG%"=="" set "MSG=release %VER%"



set "PATH=%~dp0.toolchain\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"



set "PS=pwsh.exe"

where pwsh >nul 2>nul || set "PS=powershell.exe"



echo.

echo   === WGOpt Core release v%VER% ===

echo.



echo   [1/7] sync version numbers...

"%PS%" -NoProfile -ExecutionPolicy Bypass -File "scripts\bump-version.ps1" %VER%

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

