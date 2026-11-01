# WGOpt Core

无界面的游戏优化后台程序：常驻系统托盘，检测到游戏时按原版 WGOpt 的机制自动优化，游戏退出后自动还原。

原版 [WGOpt](https://github.com/bludoge305/WGOpt) 是 Tauri 桌面应用（界面 + 托盘）。本仓库去掉图形界面，只保留核心：托盘状态显示、托盘开关、配置文件控制、系统通知。

## 功能

**托盘常驻**：程序不占任务栏、不进 Alt+Tab，只在通知区域显示一个图标。

**托盘显示当前优化状态**，三态图标 + 悬浮提示：

| 图标 | 含义 | 提示 |
|---|---|---|
| 蓝色（原图） | 待机，未检测到游戏 | `WGOpt · 空闲` |
| 蓝色 + 绿色对勾 | 优化已生效 | `WGOpt · 已优化 · <游戏名>` |
| 灰色 + 暂停 | 总开关关闭 | `WGOpt · 已停用` |

**托盘右键菜单**控制总开关与各项：

- 顶部一行显示当前状态与已完成项数（如 `状态：已优化 7/8 · 黑神话：悟空`）
- `启用优化总开关` —— 关闭后立刻还原已做的改动，并停止一切检测
- `游戏运行时自动优化` —— 关闭后只提示检测到游戏，不自动执行
- `立即开始优化` / `停止优化并还原`
- 8 个优化项逐项勾选（等价于改配置文件，会立即写回）
- `开机自启`、`打开配置文件目录`、`退出 WGOpt`

**配置文件**控制单次启动时的初始状态，路径 `%APPDATA%\WGOpt\config.json`
（与原版 Tauri 版的 `app_config_dir` 同一目录，从桌面版切过来不会丢配置）：

```json
{
  "enabled": true,
  "auto_optimize": true,
  "enabled_items": ["priority", "background", "memory", "wallpaper", "xbox", "search", "dns", "clipboard"],
  "autostart": false,
  "notify_on_start": true,
  "notify_on_stop": true,
  "restore_on_exit": true
}
```

| 字段 | 说明 |
|---|---|
| `enabled` | 总开关。关闭后不做任何优化，托盘显示"已停用" |
| `auto_optimize` | 检测到游戏后是否自动优化 |
| `enabled_items` | 启用的优化项 id，见下表 |
| `autostart` | 开机自启（实际状态以注册表为准） |
| `notify_on_start` | 开始优化时弹系统通知 |
| `notify_on_stop` | 停止/自动还原时弹系统通知 |
| `restore_on_exit` | 退出程序时还原可还原项 |

配置写坏时程序**按停用处理并记日志**，不会静默回退成"全开"——后台进程不该在配置有问题时继续结束进程、停服务。

**系统通知**：状态变化时弹气泡通知——检测到游戏、优化完成（含失败项数）、游戏退出已还原、开关切换。

## 8 项优化

与原版逐项一致，白名单与硬性排除也照搬：

| id | 内容 | 需管理员 | 可还原 |
|---|---|---|---|
| `priority` | 游戏进程提到「高」，白名单后台进程降到「低于正常」 | 否 | 是 |
| `background` | 结束白名单内的后台进程 | 是 | 否 |
| `memory` | 整理所有进程工作集（EmptyWorkingSet） | 否 | 否 |
| `wallpaper` | 关闭 Wallpaper Engine | 否 | 否 |
| `xbox` | 停止 XblAuthManager 服务 | 是 | 是 |
| `search` | 停止 WSearch 服务 | 是 | 是 |
| `dns` | 刷新 DNS 解析缓存 | 否 | 否 |
| `clipboard` | 清空剪贴板 | 否 | 否 |

进程终止只允许命中 `BACKGROUND_WHITELIST`，另有 `PROTECTED_PROCESSES` 硬性排除；服务操作只允许 `SERVICE_WHITELIST` 内的两个。可还原项在执行前记录原状态，停止优化或游戏退出时还原。

## 游戏识别

沿用原版 v1.3.0 起的方案：以 **GPU 3D 引擎占用**为主判据
（`\GPU Engine(*)\Utilization Percentage`，与任务管理器同源），
辅以全屏状态、窗口尺寸、内存、线程、CPU、进程名特征与父子链加权评分，
阈值 60 分。GPU 数据缺失时（纯 CPU 游戏、驱动未暴露计数器）由其他维度兜底。

轮询间隔 3 秒；游戏退出后等待 6 秒才停止优化，避免切窗口导致反复启停。

## 命令行

后台程序没有界面，排障靠这几个开关：

```bash
wgopt                     # 启动托盘常驻（默认）
wgopt --status            # 查看配置、权限、内存、各项开关
wgopt --detect            # 执行一轮游戏识别并输出判定依据
wgopt --optimize-once     # 执行一轮优化（不常驻）
wgopt --restore           # 说明还原入口（快照只存在于运行中的进程）
wgopt --version
```

日志在 `%APPDATA%\WGOpt\wgopt.log`，单文件上限 512 KB 自动滚动。

## 构建

需要 Rust（GNU 工具链）与 MinGW binutils：

```bash
cargo build --release
```

产物 `target/release/wgopt.exe`（约 820 KB）。

**运行时需要 `target/release/tray/` 下的三个图标**（`idle.ico` / `active.ico` / `off.ico`），
程序会在 exe 同目录的 `tray\`、`assets\tray\` 以及配置目录的 `icons\` 里依次查找。
用 `deploy.cmd` 一次拷好：

```bat
deploy.cmd
```

程序启动时会用 `runas` 自我提权（与原版一致），需要管理员权限才能停服务、结束其他用户的后台进程。

## 版本

版本号在 `Cargo.toml`。发布：

```bat
release.cmd 1.0.1 "fix: 修复 xxx"
```

脚本会同步版本号、编译、跑测试、提交、打 tag 并推送。

## 许可

与 WGOpt 原版一致。
