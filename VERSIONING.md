# 版本号约定

单仓单版本，遵循[语义化版本](https://semver.org/lang/zh-CN/) `主版本.次版本.修订号`。

| 位 | 何时递增 | 例子 |
|---|---|---|
| 主版本 | 与桌面版机制分叉的不兼容改动 | 配置格式重写 |
| 次版本 | 新增功能 | 新增优化项、新增识别维度 |
| 修订号 | 修 bug、调优 | 修复死锁、修复配置回退 |

## 版本号在哪

| 位置 | 用途 |
|---|---|
| `Cargo.toml` 的 `version` | 程序版本，`wgopt --version` 读取它；安装包版本也读这里 |
| `assets/wgopt.rc` 的 `FILEVERSION` / `PRODUCTVERSION` | exe 文件属性里显示的版本 |
| `assets/wgopt.rc` 的 `FileVersion` / `ProductVersion` | 资源字符串 |

前两组由 `scripts/bump-version.ps1` 一次性同步，**不要手改**。
`wgopt.rc` 的改动要重新编译才生效。
安装包版本由 `build-installer.cmd` 从 `Cargo.toml` 读取后传给 NSIS，不需要单独维护。

## 发布流程

```bat
release.cmd 1.0.1 "fix: 修复 xxx"
```

脚本依次执行：

1. `scripts/bump-version.ps1` 同步版本号（无 BOM 写入）
2. `cargo test`
3. `cargo build --release`
4. 拷贝托盘图标到 `target\release\tray\`
5. `build-installer.cmd` 打 NSIS 安装包
6. `git commit` + `git tag v1.0.1`
7. `git push origin main` + `git push origin v1.0.1`

产物：

| 文件 | 说明 |
|---|---|
| `target\release\wgopt.exe` | 主程序 |
| `target\release\bundle\WGOpt-Core-<版本>-x64-setup.exe` | 安装包（分发这个） |
| `target\release\bundle\release-notes-<版本>.md` | Release 说明，自动生成 |

需要发布到 GitHub Release：

```bat
scripts\publish-release.cmd 1.0.1
```

**分发用安装包。** 安装包会把 exe 与 `tray\` 图标一起装好；
只给裸 exe 时托盘图标找不到，托盘不显示。

## 版本历史

| 版本 | 类型 | 内容 |
|---|---|---|
| 2.1.1 | fix | 修复全屏游戏里快捷键完全失效：编辑场景判定原先只看 `hwndCaret` 是否有效，而游戏会创建类名为空的隐藏窗口并挂一个 caret，于是永远判成「前台有文本光标」。现在要求 caret 所在窗口的类名确实是编辑控件（`Edit` / `RichEdit*` / `Scintilla` / `_WwG`）才算编辑场景。实测死亡细胞（`HL_WIN`）可从「始终忽略」变为正常识别与提权 |
| 2.1.0 | feat | 快捷键「开始」方向新增前台进程识别：按下 `Ctrl+F2` 时读当前前台窗口所属进程，经排除清单校验后提到「高」优先级，并写入配置 `game_process` 供后续轮次使用。配置新增 `excluded_processes`（默认 65 项：系统外壳、浏览器、开发工具、通讯办公、播放器、本程序），命中者即使处于前台也绝不提权；另受 `PROTECTED_PROCESSES` 硬性保护。捕获失败（无前台窗口 / 排除命中 / 自身进程 / 配置写盘失败）均有明确处理与日志，不阻断优化。顺带修复 `set_process_priority` 只申请 `PROCESS_SET_INFORMATION` 导致读不到原优先级（永远「未知」）、进而使还原静默失效的缺陷；还原失败项现在如实报告 |
| 2.0.0 | breaking | 移除游戏识别与自动优化：删除 `game_detect` / `gpu` 两个模块（GPU 性能计数器采样、多维加权评分、全屏状态判定、进程名与父子链特征）、后台 3 秒轮询、`auto_optimize` 配置项与托盘「游戏运行时自动优化」菜单项、`--detect` 命令行开关、`Status.game` 与游戏名显示、`KNOWN_GAMES` 名单。优化改为纯手动触发（托盘菜单 / `Ctrl+F2`）。`priority` 项只保留下调后台进程优先级，不再提升游戏进程。快照的 `promoted` 字段一并移除 |
| 1.1.2 | fix | 托盘与退出路径的一批修复：消息窗口不再用 `HWND_MESSAGE`（收不到 `TaskbarCreated`，Explorer 重启后托盘图标永久消失），广播消息 ID 改由 `RegisterWindowMessageW` 取；托盘图标挂载失败或丢失后自动重挂；关闭总开关/退出时按快照而非 `phase` 判定是否还原（优化中途关闭会漏还原）；退出还原收进 `Shared::shutdown` 一个入口（原先托盘退出绕过 `restore_on_exit`，且可能与守护线程重复还原）；第二实例唤起菜单真正生效（命名事件原先无人等待）；`--version` / `--help` / `--status` / `--detect` 不再要求管理员，`-v` 不再落进常驻模式；DNS 刷新改运行时 `GetProcAddress`，不再依赖 MinGW 的 `libdnsapi.a` |
| 1.1.1 | fix | 「立即开始优化」与「停止优化并还原」合并为同一个状态切换项：菜单按状态显示对应文案、快捷键 `Ctrl+F2` 未优化时开始、已优化时还原；判定与投递统一收进 `Shared::toggle_action` / `request_toggle`，菜单、快捷键、灰化状态共用一份条件。优化进行中该项灰掉，快捷键同样不响应 |
| 1.1.0 | feat | 新增全局快捷键（默认 `Ctrl+F2`）触发「立即开始优化」：`RegisterHotKey` 挂在消息窗口，托盘菜单新增开关，配置新增 `hotkey_enabled` / `hotkey`；带 `MOD_NOREPEAT` 与 1 秒闸门防重复触发，前台处于编辑场景（文本光标 / `Edit` 类控件 / 正在操作菜单）时忽略 |
| 1.0.4 | fix | 修复停止优化后 Wallpaper Engine 不恢复：`background` 项（白名单含 WE）先结束进程、`wallpaper` 项随后查不到路径，快照里没有可还原记录。现在两项共用路径记录；还原只拉起一个主进程，已运行/手动恢复不重复启动，启动后立即退出如实报失败；连续两轮优化不再丢失待还原记录 |
| 1.0.3 | fix | 关闭「游戏运行时自动优化」后彻底停用后台游戏检测（不采样、不识别、不提示） |
| 1.0.2 | fix | 修复 Wallpaper Engine 只被关闭、停止优化后不还原：现在关闭前记录可执行文件路径，还原时按路径重新拉起（已在运行则不重复启动） |
| 1.0.1 | feat | 新增 NSIS 安装包：用户级安装到 `%LOCALAPPDATA%\WGOpt Core`、开始菜单、控制面板条目、静默安装；修复残留注册表导致装到错误路径 |
| 1.0.0 | feat | 首个版本：去掉 Tauri 界面，改为托盘常驻；托盘三态图标显示优化状态、托盘控制总开关与各项开关、配置文件控制、状态变化系统通知 |

## 与桌面版的关系

两者版本号各自独立，机制保持同步：

| wgopt-core | 对应的 WGOpt 桌面版机制 |
|---|---|
| 1.0.0 | 8 项优化、白名单、GPU 识别（对应桌面版 1.3.x / 1.4.x 的核心） |
| 2.0.0 | 8 项优化与白名单保持同步；游戏识别与自动优化**有意不跟进**，本版本改为纯手动触发 |
| 2.1.0 | 快捷键的前台进程识别借用了桌面版「提升游戏进程」的思路，但只按聚焦窗口判断单个进程，不做评分与轮询 |

核心机制改动时，两边都要跟，并在本表记录对应关系。唯一的例外是游戏检测与自动
触发：那是桌面版的特色，本仓库明确不提供。
