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
| 1.0.1 | feat | 新增 NSIS 安装包：用户级安装到 `%LOCALAPPDATA%\WGOpt Core`、开始菜单、控制面板条目、静默安装；修复残留注册表导致装到错误路径 |
| 1.0.0 | feat | 首个版本：去掉 Tauri 界面，改为托盘常驻；托盘三态图标显示优化状态、托盘控制总开关与各项开关、配置文件控制、状态变化系统通知 |

## 与桌面版的关系

两者版本号各自独立，机制保持同步：

| wgopt-core | 对应的 WGOpt 桌面版机制 |
|---|---|
| 1.0.0 | 8 项优化、白名单、GPU 识别（对应桌面版 1.3.x / 1.4.x 的核心） |

核心机制改动时，两边都要跟，并在本表记录对应关系。
