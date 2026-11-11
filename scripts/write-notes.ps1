# 生成 GitHub Release 说明文字。
#
# 用 PowerShell 而不是 cmd 的 echo：cmd 在 GBK 代码页下会把 UTF-8 中文写坏，
# 生成的 markdown 变成乱码。这里统一用 UTF-8 无 BOM 写出。

param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$OutFile
)

$ErrorActionPreference = "Stop"

# here-string 里的反引号是转义符，写 3 个会被 PowerShell 吃成一个，
# 生成出 `` 这种坏掉的 markdown 代码块。用单引号字符串拼出栅栏。
$fence = '```'

$content = @"
## WGOpt Core v$Version

WGOpt 的无界面版本：去掉了原版 Tauri 图形界面，改为系统托盘常驻后台程序。
优化机制、白名单、游戏识别算法与原版逐项一致。

### 安装

下载 ``WGOpt-Core-$Version-x64-setup.exe`` 双击安装（用户级安装，不需要管理员权限）。

安装后在开始菜单创建快捷方式，控制面板「应用和功能」里可以卸载。
托盘图标可能位于任务栏的「显示隐藏的图标」中，可拖到任务栏常驻。

静默安装：``WGOpt-Core-$Version-x64-setup.exe /S``

静默卸载：``"%LOCALAPPDATA%\WGOpt Core\uninstall.exe" /S``

### 功能

- 托盘三态图标显示优化状态（待机 / 已优化 / 已停用），悬浮提示带游戏名
- 托盘右键菜单：状态行、总开关、自动优化、开始/还原（同一个状态切换项）、8 项逐项开关、开机自启
- 全局快捷键 ``Ctrl+F2`` 按当前状态切换：未优化时开始、已优化时还原，可在配置文件里改键、托盘菜单里开关
- 配置文件 ``%APPDATA%\WGOpt\config.json`` 控制各项开关
- 状态变化时弹系统通知（检测到游戏、优化完成、游戏退出已还原）

### 8 项优化

进程优先级、关闭后台进程、释放内存、关闭 Wallpaper Engine、
停止 Xbox Live / Windows Search 服务、清理 DNS、清空剪贴板。

### 命令行

$fence
wgopt --status          查看配置与权限
wgopt --detect          一轮游戏识别 + 判定依据
wgopt --optimize-once   跑一轮优化
$fence

### 实测

8 项中 7 项生效，内存 4.19 GB → 8.34 GB（释放 4.15 GB），WSearch 服务真实停止。
"@

$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($OutFile, $content, $utf8)

Write-Output "release notes written: $OutFile"
