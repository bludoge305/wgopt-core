//! 安全白名单：允许被操作的后台进程与服务。
//!
//! 原版策略原样保留——进程终止只允许命中 `BACKGROUND_WHITELIST`，
//! 系统关键进程另有硬性排除；服务操作只允许命中 `SERVICE_WHITELIST`。

/// 允许被结束的后台进程白名单（进程名小写匹配）。
///
/// 覆盖常见的更新器、云同步、通讯软件后台、下载器、动态壁纸。
///
/// 注意：Windows 更新、Defender 等由系统保护的服务进程（如
/// `MoUsoCoreWorker.exe`、`usoclient.exe`、`wuauclt.exe`）即使以管理员
/// 身份也无法终止，放进白名单只会产生"无权结束"的误导性提示，因此不列入。
pub const BACKGROUND_WHITELIST: &[&str] = &[
    // 更新器（第三方）
    "software_reporter_tool.exe",
    "googleupdate.exe",
    "adobearm.exe",
    "adobeupdater.exe",
    "jusched.exe",
    // 云同步
    "onedrive.exe",
    "dropbox.exe",
    "baidunetdisk.exe",
    "baidunetdiskhost.exe",
    "weiyun.exe",
    "aliyundrive.exe",
    // 通讯后台
    "qqprotect.exe",
    "qqpcrtray.exe",
    "txplatform.exe",
    "wechatapp.exe",
    "wechatweb.exe",
    "dingtalk.exe",
    // 下载器
    "thunder.exe",
    "thunderplatform.exe",
    "xldl.exe",
    // 动态壁纸
    "wallpaper32.exe",
    "wallpaper64.exe",
    "ui32.exe",
    "webwallpaper32.exe",
    "webwallpaper64.exe",
];

/// 绝不允许结束的进程（即使被白名单误命中）。
pub const PROTECTED_PROCESSES: &[&str] = &[
    "system",
    "system idle process",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "explorer.exe",
    "dwm.exe",
    "svchost.exe",
    "fontdrvhost.exe",
    "ctfmon.exe",
    "sihost.exe",
    "taskhostw.exe",
    "runtimebroker.exe",
    "audiodg.exe",
    "wgopt.exe",
];

/// 允许操作的服务白名单。
pub const SERVICE_WHITELIST: &[&str] = &["XblAuthManager", "WSearch"];

/// 会被识别的游戏进程名（常见启动器与游戏）。
pub const KNOWN_GAMES: &[&str] = &[
    "blackmythwukong.exe",
    "b1-win64-shipping.exe",
    "gow.exe",
    "eldenring.exe",
    "cs2.exe",
    "dota2.exe",
    "valorant.exe",
    "league of legends.exe",
    "genshinimpact.exe",
    "yuanshen.exe",
    "starrail.exe",
    "overwatch.exe",
    "destiny2.exe",
    "r5apex.exe",
    "pubg.exe",
    "tslgame.exe",
    "forzahorizon5.exe",
    "cyberpunk2077.exe",
    "bg3.exe",
    "bg3_dx11.exe",
];

/// 是否属于受保护进程。
pub fn is_protected(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PROTECTED_PROCESSES.iter().any(|p| *p == lower)
}

/// 是否属于可结束的后台进程白名单。
pub fn in_background_whitelist(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    BACKGROUND_WHITELIST.iter().any(|p| *p == lower)
}

/// 是否是已知游戏进程名。
pub fn is_known_game(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    KNOWN_GAMES.iter().any(|g| *g == lower)
}
