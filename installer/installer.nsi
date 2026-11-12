; WGOpt Core 安装程序
;
; 与原版 WGOpt 的 Tauri NSIS 包保持一致的行为：
;   · 用户级安装（不需要管理员），装到 %LOCALAPPDATA%\WGOpt Core
;   · 开始菜单快捷方式、控制面板「应用和功能」条目
;   · 支持静默安装 /S，便于脚本分发
;
; 关于"安装前结束正在运行的实例"：
; 本程序的常驻进程始终以管理员身份运行（启动时自己 runas 提权），
; 普通权限的安装程序杀不掉它；用 taskkill /IM wgopt.exe 还会误伤
; 同名的桌面版 WGOpt。所以这里不做强杀，而是检测程序文件是否被占用，
; 提示用户从托盘菜单退出后重试——干净且不会误伤。

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

; ---- 这些由 build-installer.cmd 通过 /D 传入 ----
!ifndef SRCDIR
  !error "SRCDIR 未定义：请用 build-installer.cmd 调用"
!endif
!ifndef VERSION
  !define VERSION "1.0.0"
!endif
!ifndef VERSIONQUAD
  !define VERSIONQUAD "1.0.0.0"
!endif
!ifndef OUTFILE
  !define OUTFILE "${SRCDIR}\target\release\bundle\WGOpt-Core-${VERSION}-x64-setup.exe"
!endif

!define PRODUCTNAME "WGOpt Core"
!define APPNAME     "WGOpt"
!define PUBLISHER   "WGOpt"
!define HOMEPAGE    "https://github.com/bludoge305/wgopt-core"
!define MAINEXE     "wgopt.exe"
!define UNINSTKEY   "Software\Microsoft\Windows\CurrentVersion\Uninstall\WGOptCore"
!define RUNKEY      "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${PRODUCTNAME}"
BrandingText "${PUBLISHER}"
OutFile "${OUTFILE}"

; 用户级安装：不需要管理员，装到当前用户目录（与原版 Tauri 包一致）
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"

VIProductVersion "${VERSIONQUAD}"
VIAddVersionKey "ProductName"     "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "WGOpt 游戏优化后台程序 安装程序"
VIAddVersionKey "FileVersion"     "${VERSIONQUAD}"
VIAddVersionKey "ProductVersion"  "${VERSIONQUAD}"
VIAddVersionKey "LegalCopyright"  "${PUBLISHER}"
VIAddVersionKey "CompanyName"     "${PUBLISHER}"

; ---- 界面 ----
!define MUI_ICON   "${SRCDIR}\assets\wgopt.ico"
!define MUI_UNICON "${SRCDIR}\assets\wgopt.ico"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "安装 WGOpt Core ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "WGOpt Core 是常驻系统托盘的游戏优化程序。$\r$\n$\r$\n安装后会：$\r$\n  · 在通知区域显示托盘图标，检测到游戏时自动优化$\r$\n  · 在开始菜单创建快捷方式$\r$\n$\r$\n程序需要管理员权限才能停止服务、调整进程优先级，启动时会请求提权。"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_TITLE "WGOpt Core 安装完成"
!define MUI_FINISHPAGE_TEXT "程序已安装到：$\r$\n$INSTDIR$\r$\n$\r$\n点击「完成」立即启动（首次启动会请求管理员权限）。$\r$\n托盘图标可能位于任务栏的「显示隐藏的图标」中，可拖到任务栏常驻。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${MAINEXE}"
!define MUI_FINISHPAGE_RUN_TEXT "立即启动 WGOpt Core"
!define MUI_FINISHPAGE_NOREBOOTSUPPORT
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"

; ------------------------------------------------------------------ 文件占用检测

; 确认 $INSTDIR\wgopt.exe 没有被正在运行的实例占用。
;
; 运行中的 exe 映像被 Windows 以只读+删除共享方式映射，写入会被拒绝，
; 用「以追加方式打开」就能判断。被占用时提示用户从托盘退出再重试。
Function CheckNotInUse
  ${IfNot} ${FileExists} "$INSTDIR\${MAINEXE}"
    Return
  ${EndIf}

  ${Do}
    ClearErrors
    FileOpen $0 "$INSTDIR\${MAINEXE}" a
    ${IfNot} ${Errors}
      FileClose $0
      ${ExitDo}
    ${EndIf}

    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION \
      "WGOpt Core 正在运行，程序文件被占用，无法覆盖。$\r$\n$\r$\n请右键点击通知区域的 WGOpt 图标，选择「退出 WGOpt」，$\r$\n然后点击「重试」继续安装。" \
      IDRETRY CheckNotInUse_retry
    Abort

    CheckNotInUse_retry:
  ${Loop}
FunctionEnd

; ------------------------------------------------------------------ 安装

Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"

  Call CheckNotInUse

  ; 主程序
  File "${SRCDIR}\target\release\wgopt.exe"

  ; 托盘图标：放在 exe 同目录的 tray\ 下，程序按这个相对路径查找
  CreateDirectory "$INSTDIR\tray"
  SetOutPath "$INSTDIR\tray"
  File "${SRCDIR}\assets\tray\idle.ico"
  File "${SRCDIR}\assets\tray\active.ico"
  File "${SRCDIR}\assets\tray\off.ico"

  SetOutPath "$INSTDIR"
  File /oname=README.md     "${SRCDIR}\README.md"
  File /oname=VERSIONING.md "${SRCDIR}\VERSIONING.md"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; ---- 开始菜单 ----
  CreateDirectory "$SMPROGRAMS\${PRODUCTNAME}"
  CreateShortCut "$SMPROGRAMS\${PRODUCTNAME}\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINEXE}" "" "$INSTDIR\${MAINEXE}" 0
  CreateShortCut "$SMPROGRAMS\${PRODUCTNAME}\卸载 ${PRODUCTNAME}.lnk" "$INSTDIR\uninstall.exe"

  ; ---- 控制面板「应用和功能」 ----
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegStr   SHCTX "${UNINSTKEY}" "DisplayName"         "${PRODUCTNAME}"
  WriteRegStr   SHCTX "${UNINSTKEY}" "DisplayIcon"         "$INSTDIR\${MAINEXE}"
  WriteRegStr   SHCTX "${UNINSTKEY}" "DisplayVersion"      "${VERSION}"
  WriteRegStr   SHCTX "${UNINSTKEY}" "Publisher"           "${PUBLISHER}"
  WriteRegStr   SHCTX "${UNINSTKEY}" "InstallLocation"     "$INSTDIR"
  WriteRegStr   SHCTX "${UNINSTKEY}" "UninstallString"     '"$INSTDIR\uninstall.exe"'
  WriteRegStr   SHCTX "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr   SHCTX "${UNINSTKEY}" "URLInfoAbout"        "${HOMEPAGE}"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify"            1
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair"            1
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize"       "$0"

  ; 记录安装路径，供增量安装定位
  WriteRegStr SHCTX "Software\${PRODUCTNAME}" "" "$INSTDIR"
SectionEnd

; ------------------------------------------------------------------ 卸载

Section "Uninstall"
  SetShellVarContext current

  ; 运行中时提示用户先退出
  ${If} ${FileExists} "$INSTDIR\${MAINEXE}"
    ${Do}
      ClearErrors
      FileOpen $0 "$INSTDIR\${MAINEXE}" a
      ${IfNot} ${Errors}
        FileClose $0
        ${ExitDo}
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION \
        "WGOpt Core 正在运行，无法删除。$\r$\n$\r$\n请右键点击通知区域的 WGOpt 图标，选择「退出 WGOpt」，$\r$\n然后点击「重试」。" \
        IDRETRY Uninstall_retry
      Abort
      Uninstall_retry:
    ${Loop}
  ${EndIf}

  ; 先移除开机自启，避免卸载后留下会报错的启动项
  DeleteRegValue HKCU "${RUNKEY}" "${APPNAME}"

  Delete "$INSTDIR\${MAINEXE}"
  Delete "$INSTDIR\tray\idle.ico"
  Delete "$INSTDIR\tray\active.ico"
  Delete "$INSTDIR\tray\off.ico"
  RMDir  "$INSTDIR\tray"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\VERSIONING.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"

  Delete "$SMPROGRAMS\${PRODUCTNAME}\${PRODUCTNAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCTNAME}\卸载 ${PRODUCTNAME}.lnk"
  RMDir  "$SMPROGRAMS\${PRODUCTNAME}"

  DeleteRegKey SHCTX "${UNINSTKEY}"
  DeleteRegKey SHCTX "Software\${PRODUCTNAME}"

  ; 配置与日志保留在 $APPDATA\WGOpt，重装后配置还在
  DetailPrint "已保留配置与日志：$APPDATA\WGOpt"
SectionEnd

; ------------------------------------------------------------------ 初始化

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "WGOpt Core 仅支持 64 位 Windows。"
    Abort
  ${EndIf}
  SetRegView 64

  ; 已有安装且目录还在时沿用原路径，避免装出两份。
  ; 目录已不存在（用户手动删过）就跳过，否则会把程序装到用户
  ; 以为不存在的路径上，后续升级和卸载都会对不上。
  ReadRegStr $0 SHCTX "Software\${PRODUCTNAME}" ""
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\${MAINEXE}"
    StrCpy $INSTDIR $0
  ${EndIf}
FunctionEnd
