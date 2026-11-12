# 生成 GitHub Release 说明文字。
#
# 用 PowerShell 而不是 cmd 的 echo：cmd 在 GBK 代码页下会把 UTF-8 中文写坏，
# 生成的 markdown 变成乱码。这里统一用 UTF-8 无 BOM 写出。
#
# 注意：下面这段文字必须与 README 的「功能」章节保持一致。改功能时两边一起改，
# 否则发布会写出描述旧行为的说明（v2.1.1 之前就出现过：说明里还有已删除的
# 游戏识别与自动优化）。

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

WGOpt 的无界面版本：去掉原版 Tauri 图形界面，改为系统托盘常驻后台程序。
优化机制与白名单照搬原版；本版本不含游戏检测，优化只在手动触发时执行。

### 安装

下载 ``WGOpt-Core-$Version-x64-setup.exe`` 双击安装（用户级安装，不需要管理员权限）。

安装后在开始菜单创建快捷方式，控制面板「应用和功能」里可以卸载。
托盘图标可能位于任务栏的「显示隐藏的图标」中，可拖到任务栏常驻。

静默安装：``WGOpt-Core-$Version-x64-setup.exe /S``

静默卸载：``"%LOCALAPPDATA%\WGOpt Core\uninstall.exe" /S``

### 功能

- 托盘三态图标显示优化状态（待机 / 已优化 / 已停用）
- 托盘右键菜单：状态行、总开关、开始/还原（同一个状态切换项）、8 项逐项开关、开机自启
- 全局快捷键 ``Ctrl+F2`` 按当前状态切换：未优化时开始、已优化时还原
- 按下快捷键时会识别当前前台进程，把它提到「高」优先级并记入配置 ``game_process``
- 配置 ``%APPDATA%\WGOpt\config.json`` 控制各项开关；``excluded_processes``
  列出绝不提权的进程（默认含系统外壳、浏览器、通讯软件等 65 项）
- 状态变化时弹系统通知（优化完成、停止并还原、快捷键未生效）

### 这个版本没有游戏检测

不会轮询进程、不会采样 GPU、不会自动优化。优化只在你按 ``Ctrl+F2``
或点托盘菜单时执行一次。需要的自动化请用原版桌面版。

### 8 项优化

进程优先级、关闭后台进程、释放内存、关闭 Wallpaper Engine、
停止 Xbox Live / Windows Search 服务、清理 DNS、清空剪贴板。

可还原项（优先级、服务、Wallpaper Engine）在停止优化或退出程序时恢复。

### 命令行

$fence
wgopt                   启动托盘常驻
wgopt --status          查看配置与权限
wgopt --optimize-once   跑一轮优化（不常驻，会提权）
wgopt --restore         说明还原入口
$fence

### 本版修复

- 全屏游戏里快捷键失效：编辑场景判定原先只看有没有文本光标，而游戏会
  创建类名为空的隐藏窗口挂光标，导致永远判成「正在编辑文本」。现在要求
  光标所在窗口确实是编辑控件
- 停止优化后优先级无法还原：修改优先级时缺少查询权限，「原优先级」永远
  记成「未知」，还原时被当成非法值静默跳过
"@

$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($OutFile, $content, $utf8)

Write-Output "release notes written: $OutFile"
