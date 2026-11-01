# WGOpt Core 项目约定

## 这是什么

原版 [WGOpt](https://github.com/bludoge305/WGOpt) 是 Tauri 桌面应用（图形界面 + 托盘）。
本仓库是它的**无界面版本**：去掉 WebView 与全部界面，只保留托盘常驻与优化核心。

**优化机制、白名单、游戏识别算法一律照搬原版，不做"改进"。** 想改核心逻辑先去
原版确认，再同步过来；本仓库不发明新策略。

关联仓库：

| 仓库 | 定位 |
|---|---|
| WGOpt（私有） | 带图形界面的桌面版 |
| **wgopt-core（本仓库，公开）** | 无界面托盘常驻版 |

## 代码结构

```
src/
  winapi.rs          Win32 封装：进程/服务/内存/剪贴板/DNS
  whitelist.rs       白名单与硬性排除（改动须与原版一致）
  item.rs            8 个优化项定义、Status 枚举
  snapshot.rs        可还原改动的快照
  engine.rs          8 项优化与还原
  gpu.rs             GPU 占用采集（PDH 计数器）
  game_detect.rs     游戏识别（GPU 主判据 + 多维加权）
  config.rs          配置读写、开机自启
  app.rs             后台守护：状态机、3s 轮询、自动优化/还原
  tray.rs            托盘图标、右键菜单、气泡通知
  window.rs          不可见消息窗口（HWND_MESSAGE）与消息循环
  single_instance.rs 命名互斥体 + 事件
  logging.rs         文件日志（512 KB 滚动）
  elevate.rs         运行时 runas 提权
  main.rs            入口、CLI 子命令、菜单命令处理
assets/
  wgopt.rc / wgopt.ico       exe 图标与版本资源
  tray/{idle,active,off}.ico 托盘三态图标
```

## 硬性约束

**不要给托盘加 `Mutex`。** 托盘对象只在主线程访问（tick 与菜单都在消息循环线程上），
用 `Rc<RefCell>` 即可。用 `Mutex` 会在 `TrackPopupMenu` 模态期间与 WM_TIMER 死锁，
表现为程序"未响应"——这个坑已经踩过一次。

**不要用 application manifest 声明 `requireAdministrator`。** MinGW 链接时 Rust
已嵌入默认 manifest，再嵌一份会让 ld 报 `.rsrc merge failure: multiple non-default
manifests`。提权走运行时 `ShellExecuteExW(runas)`，与原版一致。

**配置解析失败必须按停用处理。** 不能静默回退成默认全开：后台进程若在用户配置
写坏时继续结束进程、停服务，属于误伤。见 `config::load_from_dir` 与
`broken_json_fails_safe_to_disabled` 测试。

**优化项、白名单、阈值不要随手调。** `THRESHOLD = 60`、`BACKGROUND_WHITELIST`、
`PROTECTED_PROCESSES`、`SERVICE_WHITELIST` 都是原版实测过的值。

## Shell：必须用 PowerShell 7

```bash
"/c/Program Files/PowerShell/7/pwsh.exe" -NoProfile -File xxx.ps1
```

**不要用 `powershell`**（5.1）。它按 GBK 解析 `.ps1`，脚本里一有中文就报
「字符串缺少终止符」并把输出变成乱码。

补充：
- 生成 `.ps1` 时用 Python 以 `utf-8-sig`（带 BOM）写出，5.1 也能读。
- 改 JSON 配置时不要写 BOM，`serde_json` 会报 `expected value at line 1 column 1`。
  `load_from_dir` 已经会剥掉 BOM，但写的时候仍不要加。
- Agent 的 Bash 工具是 Git Bash（MINGW64）。原生命令的参数斜杠要写两遍
  （`tasklist //FO CSV`），否则 MSYS 会把它当路径改写。
- 终端代码页是 936，看中文输出要 `| iconv -f utf-8 -t gb18030`。

## 构建与运行

```bat
deploy.cmd
```

等价于 `cargo build --release` + 把 `assets\tray\*.ico` 拷到 `target\release\tray\`。

**托盘图标是必需文件。** exe 单独拷走时图标会找不到，托盘不显示。程序会依次查找
`<exe目录>\tray\`、`<exe目录>\assets\tray\`、`%APPDATA%\WGOpt\icons\`。

编译前若 `wgopt.exe` 正在运行，链接会因文件占用失败（`os error 5`）。**它是以管理员
身份跑的，普通权限杀不掉**，需要提权停进程。

## 验证方式

后台程序没有界面，改动后必须实际运行验证，不能只看编译通过：

```bat
wgopt --status          :: 配置、权限、内存、各项开关
wgopt --detect          :: 一轮游戏识别 + 判定依据
wgopt --optimize-once   :: 真跑一遍 8 项，看每项 detail 与耗时
```

托盘验证（Win11 新图标默认在溢出区，需点开"显示隐藏的图标"）：

```powershell
# UI Automation 按类名找托盘按钮，读 Name 就是 tooltip
$desk = [System.Windows.Automation.AutomationElement]::RootElement
$c = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty, "SystemTray.NormalButton")
$desk.FindAll([System.Windows.Automation.TreeScope]::Descendants, $c) |
    Where-Object { $_.Current.Name -like 'WGOpt*' } | ForEach-Object { $_.Current.Name }
```

注意：对整个桌面做 `FindAll(Descendants, TrueCondition)` 会遍历 5000+ 元素、耗时几十秒，
尽量用类名或窗口范围收窄。

## 版本管理：每次改动都要发版

```bat
release.cmd 1.0.1 "fix: 修复 xxx"
```

同步版本号（`Cargo.toml` + `assets\wgopt.rc`）→ 跑测试 → 编译 → 提交 → 打 tag → 推送。

改动类型对应版本位：
- 新增功能 → 次版本号（1.0.0 → 1.1.0）
- 修 bug / 调优 → 修订号（1.0.0 → 1.0.1）
- 与原版不同步的不兼容改动 → 主版本号

## 与桌面版的关系

原版 `desktop/src-tauri/src/commands.rs` 曾依赖一个 `wgopt-core` crate
（`item` / `snapshot` / `whitelist` 子模块、`config::load_from_dir`）。本仓库
**不是**那个 crate——那个版本从未公开存在过，原版 desktop 因此构建不了。

本仓库是独立可用的无界面程序。若要让桌面版复用，需要另做一层适配，
不要把界面需求硬塞进本仓库。
