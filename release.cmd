@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

if "%~1"=="" (
  echo   用法: release.cmd ^<版本号^> "提交说明"
  echo   例子: release.cmd 1.0.1 "fix: 修复漏判"
  exit /b 1
)

set "VER=%~1"
set "MSG=%~2"
if "%MSG%"=="" set "MSG=release %VER%"

set "PATH=%~dp0.toolchain\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"

rem 优先 PowerShell 7：5.1 按 GBK 解析脚本，中文会出错
set "PS=pwsh.exe"
where pwsh >nul 2>nul || set "PS=powershell.exe"

echo.
echo   === WGOpt Core 发布 v%VER% ===
echo.

rem ---------- 1. 同步版本号（Cargo.toml + 资源文件） ----------
echo   [1/6] 同步版本号...
"%PS%" -NoProfile -ExecutionPolicy Bypass -File "scripts\bump-version.ps1" %VER%
if errorlevel 1 (
  echo   [错误] 版本号同步失败
  exit /b 1
)

rem ---------- 2. 测试 ----------
echo   [2/6] 运行测试...
cargo test --quiet
if errorlevel 1 (
  echo   [错误] 测试失败
  exit /b 1
)

rem ---------- 3. 编译 ----------
echo   [3/6] 编译发布版本...
cargo build --release
if errorlevel 1 (
  echo   [错误] 编译失败
  exit /b 1
)

rem ---------- 4. 部署图标 ----------
if not exist "target\release\tray" mkdir "target\release\tray"
copy /y "assets\tray\*.ico" "target\release\tray\" >nul

rem ---------- 5. 提交 ----------
echo   [4/6] 提交代码...
git add -A
git commit -q -m "%MSG%"
if errorlevel 1 echo   [提示] 没有需要提交的改动，继续

rem ---------- 6. 打标签并推送 ----------
echo   [5/6] 创建标签 v%VER%...
git tag -a "v%VER%" -m "WGOpt Core v%VER% - %MSG%"
if errorlevel 1 (
  echo   [错误] 标签创建失败（可能已存在）
  exit /b 1
)

echo   [6/6] 推送到 GitHub...
git push origin main
git push origin "v%VER%"
if errorlevel 1 (
  echo   [错误] 推送失败
  exit /b 1
)

echo.
echo   完成！v%VER% 已发布。
echo   产物: target\release\wgopt.exe
echo.
echo   如需创建 GitHub Release 并附带 exe：
echo     scripts\publish-release.cmd %VER%
echo.
