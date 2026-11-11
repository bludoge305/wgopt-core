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
- `游戏运行时自动优化` —— 同时是后台游戏检测的开关：关闭后立即停用检测（不再轮询、不再采样 CPU/GPU、不再识别），重新开启时恢复
- `立即开始优化` / `停止优化并还原` —— 前者的右侧显示当前快捷键
- `启用快捷键（Ctrl+F2）` —— 全局快捷键开关，勾选后立即注册/注销
- 8 个优化项逐项勾选（等价于改配置文件，会立即写回）
- `开机自启`、`打开配置文件目录`、`退出 WGOpt`

**全局快捷键**默认 `Ctrl+F2`，按下等同于点「立即开始优化」：

- 只在「立即开始优化」可用时触发（总开关开着、当前不是优化中/已优化），
  判定与菜单灰化条件完全一致；不可用时只写日志，不弹通知打断游戏
- 输入场景不误触：前台有文本光标、焦点在 `Edit` / `RichEdit` / `Scintilla`
  一类编辑控件、或正在操作菜单时直接忽略
- 按住不放只触发一次（注册时带 `MOD_NOREPEAT`）；连按另有 1 秒闸门，
  防止后台还没切到「优化中」时排队出两轮
- 组合键被别的程序占用时，托盘开关自动回到关闭并弹一条提醒

按键写法：`Ctrl` / `Alt` / `Shift` / `Win` 用 `+` 与主键相连，例如
`Ctrl+F2`、`Alt+Shift+F9`、`Ctrl+Space`。必须带至少一个修饰键——裸键注册成
全局热键会把正常输入吞掉。主键可用 `A`–`Z`、`0`–`9`、`F1`–`F24`、
`Space` `Tab` `Enter` `Esc` `Ins` `Del` `Home` `End` `PgUp` `PgDn`
`Up` `Down` `Left` `Right`，以及 `` ` `` `-` `=` `[` `]` `\` `;` `'` `,` `.` `/`。

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
  "restore_on_exit": true,
  "hotkey_enabled": true,
  "hotkey": "Ctrl+F2"
}
```

| 字段 | 说明 |
|---|---|
| `enabled` | 总开关。关闭后不做任何优化，托盘显示"已停用" |
| `auto_optimize` | 检测到游戏后是否自动优化；同时是后台游戏检测的开关，关闭后不再轮询识别 |
| `enabled_items` | 启用的优化项 id，见下表 |
| `autostart` | 开机自启（实际状态以注册表为准） |
| `notify_on_start` | 开始优化时弹系统通知 |
| `notify_on_stop` | 停止/自动还原时弹系统通知 |
| `restore_on_exit` | 退出程序时还原可还原项 |
| `hotkey_enabled` | 全局快捷键开关，等价于托盘里的「启用快捷键」 |
| `hotkey` | 快捷键组合，默认 `Ctrl+F2`；改完需重启程序生效 |

配置写坏时程序**按停用处理并记日志**，不会静默回退成"全开"——后台进程不该在配置有问题时继续结束进程、停服务。

**系统通知**：状态变化时弹气泡通知——检测到游戏、优化完成（含失败项数）、游戏退出已还原、开关切换。

**后台检测只在需要时运行**：`auto_optimize` 关闭时守护线程不采样 CPU/GPU、不调用游戏识别，
托盘状态回到「待机」，重新开启后重建采样基线再恢复检测。总开关关闭同理。

## 8 项优化

与原版逐项一致，白名单与硬性排除也照搬：

| id | 内容 | 需管理员 | 可还原 |
|---|---|---|---|
| `priority` | 游戏进程提到「高」，白名单后台进程降到「低于正常」 | 否 | 是 |
| `background` | 结束白名单内的后台进程 | 是 | 否 |
| `memory` | 整理所有进程工作集（EmptyWorkingSet） | 否 | 否 |
| `wallpaper` | 关闭 Wallpaper Engine | 否 | 是 |
| `xbox` | 停止 XblAuthManager 服务 | 是 | 是 |
| `search` | 停止 WSearch 服务 | 是 | 是 |
| `dns` | 刷新 DNS 解析缓存 | 否 | 否 |
| `clipboard` | 清空剪贴板 | 否 | 否 |

进程终止只允许命中 `BACKGROUND_WHITELIST`，另有 `PROTECTED_PROCESSES` 硬性排除；服务操作只允许 `SERVICE_WHITELIST` 内的两个。可还原项在执行前记录原状态，停止优化或游戏退出时还原。

`wallpaper` 在结束进程**之前**先记下它的可执行文件路径，还原时按该路径重新启动；
若用户已经自己把 Wallpaper Engine 开回来了，则不会启动第二份。

`background` 的白名单里同样包含 Wallpaper Engine（原版策略如此）。默认顺序下它排在
`wallpaper` 之前，实际是它先结束 `wallpaper64.exe`，随后的 `wallpaper` 项只看得到
「未检测到」。所以 `background` 在结束 Wallpaper Engine 前也会记录路径，两项共用同一份
快照字段与还原逻辑：

- 只还原**本程序关掉**的进程。优化时原本没运行就没有记录，还原不做任何事
- 一次优化可能关掉主进程与 UI/Web 子进程，还原只启动一个主进程，其余由它自行派生
- 还原前按进程名检查（32/64 位主进程都算），用户手动开回来或已经重启过则不重复启动
- 启动后隔一段时间复查进程是否仍在：缺运行库、Steam 未启动导致立即退出的，
  如实报「失败」而不是「已重新启动」
- 连续两轮优化中间没有还原时，待还原的关闭记录会带进下一轮，不会被快照重置丢掉

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

## 验证 Wallpaper Engine 还原

还原链路需要真实进程，用 examples 走托盘同一对函数：

```bat
cargo run --release --example wallpaper_roundtrip
cargo run --release --example wallpaper_restore_edges
cargo run --release --example wallpaper_manual_recover
cargo run --release --example wallpaper_not_running
```

| 例子 | 覆盖内容 |
|---|---|
| `wallpaper_roundtrip` | 关闭 → 还原重启；`background` 先命中的路径；连续两轮优化不还原 |
| `wallpaper_restore_edges` | 已在运行不重启、路径失效、启动后立即退出、空快照不动作 |
| `wallpaper_manual_recover` | 优化后用户手动开回 WE，还原不启动第二份 |
| `wallpaper_not_running` | 优化时 WE 原本未运行，快照为空、还原不凭空启动 |

前两个需要管理员权限（要结束/启动进程），最后一个要求先退出 WE。

## 验证快捷键

三个例子都不改配置、不执行优化：

```bat
cargo run --example hotkey_probe
cargo run --example hotkey_edit_gate
cargo run --release --example hotkey_trigger
```

| 例子 | 覆盖内容 |
|---|---|
| `hotkey_probe` | 注册 `Ctrl+F2` 成功、重复注册被系统拒绝（1409）、`SendInput` 真按一次收到恰好 1 次 `WM_HOTKEY`、注销后组合键可被重新注册 |
| `hotkey_edit_gate` | 焦点在 `Edit` 控件时判为编辑场景（应忽略），焦点在普通窗口时放行 |
| `hotkey_trigger` | 走出货代码：总开关关闭时不触发、快捷键开关关闭时注销且不触发、真按一次投递 `Start` 并跑完一轮、连按被闸门拦住 |

`hotkey_trigger` 只跑 `dns` 一项（刷新解析缓存，无副作用），跑完自动还原。

## 安装

下载 `WGOpt-Core-<版本>-x64-setup.exe` 双击安装。安装程序与原版 WGOpt 的行为一致：

- **用户级安装**，不需要管理员权限，装到 `%LOCALAPPDATA%\WGOpt Core`
- 开始菜单创建「WGOpt Core」与「卸载 WGOpt Core」
- 控制面板「应用和功能」里可以看到并卸载
- 静默安装：`WGOpt-Core-<版本>-x64-setup.exe /S`
- 静默卸载：`"%LOCALAPPDATA%\WGOpt Core\uninstall.exe" /S`

卸载会清除程序文件、开始菜单和注册表项，但**保留** `%APPDATA%\WGOpt` 下的配置与日志，
重装后配置还在。

也可以不安装，直接把 `wgopt.exe` 与 `tray\` 目录放在同一处运行。

## 构建

需要 Rust（GNU 工具链）、MinGW binutils，以及 NSIS。

`PATH` 里必须有 `as.exe` 与 `dlltool.exe`，否则链接 `windows` crate 时会报
`error calling dlltool 'dlltool.exe': program not found`。Rust 自带的
`.../rustlib/x86_64-pc-windows-gnu/bin/self-contained/dlltool.exe` 不够用——
它缺少 `as.exe`，链接阶段仍会失败。装一份完整的 MinGW-w64，或像原版仓库那样
把工具链放在 `.toolchain\mingw64\bin` 并加进 `PATH`。

```bat
deploy.cmd            :: 编译 exe 并部署托盘图标
build-installer.cmd   :: 打安装包
```

产物：

| 文件 | 说明 |
|---|---|
| `target\release\wgopt.exe` | 主程序，约 820 KB |
| `target\release\bundle\WGOpt-Core-<版本>-x64-setup.exe` | 安装包，约 740 KB |

`build-installer.cmd` 会自动找 NSIS：先看 `NSIS_HOME`，再找 Tauri 缓存的
`%LOCALAPPDATA%\tauri\NSIS\makensis.exe`（与原版安装包同源），最后找系统安装的 NSIS。

**托盘图标是必需文件。** 程序会依次在 exe 同目录的 `tray\`、`assets\tray\`、
`%APPDATA%\WGOpt\icons\` 里查找 `idle.ico` / `active.ico` / `off.ico`。
安装包会自动把三个图标放到 `tray\` 下；手动部署时用 `deploy.cmd` 一次拷好。

程序启动时会用 `runas` 自我提权（与原版一致），需要管理员权限才能停服务、结束其他用户的后台进程。

## 版本

版本号在 `Cargo.toml`。发布：

```bat
release.cmd 1.0.1 "fix: 修复 xxx"
```

脚本会同步版本号、跑测试、编译 exe、打安装包、提交、打 tag 并推送。
需要把安装包挂到 GitHub Release：

```bat
scripts\publish-release.cmd 1.0.1
```

## 许可

与 WGOpt 原版一致。
