# WGOpt Core

无界面的系统优化后台程序：常驻系统托盘，按托盘菜单或全局快捷键手动执行一轮优化，
可随时停止并还原。

原版 [WGOpt](https://github.com/bludoge305/WGOpt) 是 Tauri 桌面应用（界面 + 托盘）。
本仓库去掉图形界面，只保留核心：托盘状态显示、托盘开关、配置文件控制、系统通知。

> **没有游戏检测与自动优化。** 本版本不轮询进程、不采样 GPU、不识别游戏，
> 优化只在用户明确要求时执行。原版那套识别与自动触发逻辑不在本仓库内。

## 功能

**托盘常驻**：程序不占任务栏、不进 Alt+Tab，只在通知区域显示一个图标。

**托盘显示当前优化状态**，三态图标 + 悬浮提示：

| 图标 | 含义 | 提示 |
|---|---|---|
| 蓝色（原图） | 待机 | `WGOpt · 空闲` |
| 蓝色 + 绿色对勾 | 优化已生效 | `WGOpt · 已优化` |
| 灰色 + 暂停 | 总开关关闭 | `WGOpt · 已停用` |

**托盘右键菜单**控制总开关与各项：

- 顶部一行显示当前状态与已完成项数（如 `状态：已优化 7/8`）
- `启用优化总开关` —— 关闭后立刻还原已做的改动
- `立即开始优化` / `停止优化并还原` —— **同一个菜单项**，按当前状态显示对应文案，右侧显示快捷键
- `启用快捷键（Ctrl+F2）` —— 全局快捷键开关，勾选后立即注册/注销
- 8 个优化项逐项勾选（等价于改配置文件，会立即写回）
- `开机自启`、`打开配置文件目录`、`退出 WGOpt`

**优化状态切换**：`立即开始优化` 与 `停止优化并还原` 不是两个功能，而是同一个
开关的两个方向。未优化时显示前者、执行优化；已优化时显示后者、执行还原；
正在优化时该菜单项灰掉。菜单文案、灰化状态、快捷键行为全部由
`app::Shared::toggle_action` 一个判定产出，没有第二套条件。

**全局快捷键**默认 `Ctrl+F2`，按下同样是状态切换：

- 当前未优化 → 执行「立即开始优化」；当前已优化 → 执行「停止优化并还原」
- **开始方向会先识别前台进程**：读当前前台窗口所属的进程名，通过排除清单
  校验后记为提升目标，写进配置的 `game_process`，并把它提到「高」优先级
- 优化进行中不响应，与菜单项的灰化条件一致
- 判定与投递走 `Shared::request_toggle`，与点击菜单项是同一个函数，
  菜单能做的切换快捷键就能做，菜单灰掉的快捷键也不做
- 输入场景不误触：焦点在 `Edit` / `RichEdit` / `Scintilla` 一类编辑控件、
  光标所在窗口也确实是编辑控件、或正在操作菜单时直接忽略
  （只看"有光标"会误伤全屏游戏——它们会创建一个类名为空的隐藏窗口挂 caret）
- 按住不放只触发一次（注册时带 `MOD_NOREPEAT`）；连按另有 1 秒闸门，
  防止后台还没切到「优化中」时排队出多轮
- 组合键被别的程序占用时，托盘开关自动回到关闭并弹一条提醒

**前台进程识别**（只在「开始优化」方向执行）：

| 情况 | 行为 |
|---|---|
| 前台是普通程序（游戏等） | 记为提升目标，写入 `game_process`，提到「高」 |
| 前台进程在 `excluded_processes` 里 | 不提升，日志记一行原因，优化照常进行 |
| 前台是受保护的系统进程 | 同上，拒绝提升 |
| 前台是 WGOpt 自己 | 同上 |
| 前台窗口无效（桌面 / 锁屏 / 切换中） | 不提升，日志记「没有可识别的前台窗口」 |
| 配置写盘失败 | 本次仍按识别结果提权，弹出警告说明「下次启动不会记得」 |
| 目标进程有多个同名实例 | 全部提升 |
| 目标进程已退出 | 跳过，不报错 |

还原时按快照把提升/降级的优先级都改回去；进程已退出的那一项会如实报
「恢复失败」而不是装作成功。

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
  "enabled_items": ["priority", "background", "memory", "wallpaper", "xbox", "search", "dns", "clipboard"],
  "autostart": false,
  "notify_on_start": true,
  "notify_on_stop": true,
  "restore_on_exit": true,
  "hotkey_enabled": true,
  "hotkey": "Ctrl+F2",
  "game_process": "BlackMythWukong.exe",
  "excluded_processes": ["explorer.exe", "chrome.exe", "dwm.exe", "..."],
}
```

| 字段 | 说明 |
|---|---|
| `enabled` | 总开关。关闭后不做任何优化，托盘显示"已停用" |
| `enabled_items` | 启用的优化项 id，见下表 |
| `autostart` | 开机自启（实际状态以注册表为准） |
| `notify_on_start` | 开始优化时弹系统通知 |
| `notify_on_stop` | 停止优化并还原时弹系统通知 |
| `restore_on_exit` | 退出程序时还原可还原项 |
| `hotkey_enabled` | 全局快捷键开关，等价于托盘里的「启用快捷键」 |
| `hotkey` | 快捷键组合，默认 `Ctrl+F2`；改完需重启程序生效 |
| `game_process` | **程序自动写入**：上次按快捷键时识别到的前台进程。供本次及之后的优化轮次使用。手改成路径也会被归一化成文件名 |
| `excluded_processes` | **绝不允许提升优先级的进程清单**。大小写不敏感，写入时自动转小写去重。默认 65 项，见下节 |

配置写坏时程序**按停用处理并记日志**，不会静默回退成"全开"——后台进程不该在配置有问题时继续结束进程、停服务。

旧版本配置里缺少的字段会按默认补齐：`game_process` 为空（还没识别过），
`excluded_processes` 填入默认排除清单（而不是空清单——空清单等于关掉保护）。

**默认排除清单**（65 项，`config::default_excluded_processes`）：

| 类别 | 进程 |
|---|---|
| 系统外壳与关键进程 | `explorer.exe` `dwm.exe` `csrss.exe` `wininit.exe` `winlogon.exe` `services.exe` `lsass.exe` `smss.exe` `svchost.exe` `taskhostw.exe` `sihost.exe` `ctfmon.exe` `runtimebroker.exe` `fontdrvhost.exe` `audiodg.exe` `searchhost.exe` `startmenuexperiencehost.exe` `shellexperiencehost.exe` `applicationframehost.exe` `textinputhost.exe` `lockapp.exe` |
| 浏览器 | `chrome.exe` `msedge.exe` `firefox.exe` `brave.exe` `opera.exe` `vivaldi.exe` `iexplore.exe` `chromium.exe` `zen.exe` |
| 开发与终端 | `code.exe` `devenv.exe` `pycharm64.exe` `idea64.exe` `windowsterminal.exe` `conhost.exe` `cmd.exe` `powershell.exe` `pwsh.exe` `wsl.exe` `wslhost.exe` |
| 通讯与办公 | `wechat.exe` `weixin.exe` `qq.exe` `dingtalk.exe` `feishu.exe` `lark.exe` `teams.exe` `slack.exe` `discord.exe` `telegram.exe` `winword.exe` `excel.exe` `powerpnt.exe` `wps.exe` `notion.exe` `obsidian.exe` `typora.exe` |
| 媒体与直播 | `vlc.exe` `potplayermini64.exe` `mpc-hc64.exe` `wmplayer.exe` `obs64.exe` `obs32.exe` |
| 本程序 | `wgopt.exe` |

要加自己的程序，直接往 `excluded_processes` 里追加进程名（带 `.exe`）。
`PROTECTED_PROCESSES` 是另一道硬性防线：即使从排除清单里删掉
`csrss.exe`，也不会被提升。

**系统通知**：状态变化时弹气泡通知——优化完成（含失败项数）、停止并还原、开关切换、
快捷键注册失败。

## 8 项优化

白名单与硬性排除沿用原版：

| id | 内容 | 需管理员 | 可还原 |
|---|---|---|---|
| `priority` | 目标进程（`game_process`）提到「高」，白名单后台进程降到「低于正常」 | 否 | 是 |
| `background` | 结束白名单内的后台进程 | 是 | 否 |
| `memory` | 整理所有进程工作集（EmptyWorkingSet） | 否 | 否 |
| `wallpaper` | 关闭 Wallpaper Engine | 否 | 是 |
| `xbox` | 停止 XblAuthManager 服务 | 是 | 是 |
| `search` | 停止 WSearch 服务 | 是 | 是 |
| `dns` | 刷新 DNS 解析缓存 | 否 | 否 |
| `clipboard` | 清空剪贴板 | 否 | 否 |

`priority` 的提升目标来自快捷键识别的前台进程（`game_process`）。没有目标时
（还没按过快捷键、或前台是排除清单里的进程、或进程已退出）只做后台降级。
命令行 `--optimize-once` 不经过快捷键，所以使用配置里上次记录的目标。

进程终止只允许命中 `BACKGROUND_WHITELIST`，另有 `PROTECTED_PROCESSES` 硬性排除；服务操作只允许 `SERVICE_WHITELIST` 内的两个。可还原项在执行前记录原状态，停止优化时还原。

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

## 命令行

后台程序没有界面，排障靠这几个开关：

```bash
wgopt                     # 启动托盘常驻（默认）
wgopt --status            # 查看配置、权限、内存、各项开关
wgopt --optimize-once     # 执行一轮优化（不常驻）
wgopt --restore           # 说明还原入口（快照只存在于运行中的进程）
wgopt --version
```

除 `--optimize-once` 外都不需要管理员权限，不会弹 UAC。`--optimize-once` 要停服务、
结束进程、改优先级，所以会先提权。

日志在 `%APPDATA%\WGOpt\wgopt.log`，单文件上限 512 KB 自动滚动。

## 退出与还原

还原依据的是快照，不是界面上的状态：优化跑到一半时关掉总开关、或者直接退出程序，
已经改过的进程优先级、服务、Wallpaper Engine 一样会被还原。托盘「退出」与窗口
关闭走同一条收尾路径，不会出现只还原一半的情况。`restore_on_exit` 为 `false`
时退出不还原。

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
| `hotkey_trigger` | 走出货代码：待机时按下开始、已优化时按下还原、还原后可再次开始、总开关关闭时不动、快捷键开关关闭时注销且不触发、连按被闸门拦住 |

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
