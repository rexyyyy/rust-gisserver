; =============================================================================
;  rust-gisserver  Windows 安装程序
; -----------------------------------------------------------------------------
;  功能：
;    1) 标准白底对话框 + 天空蓝横幅页头（使用 NSIS 内置 header bitmap）
;    2) 配置页采集 map-api / manager 两个服务端口
;    3) 复制 run-env 全部文件，写入 .env 配置（密码默认 123456）
;    4) 用 NSSM 把 map-api / manager 两个 exe 注册为 Windows 服务
;    5) 卸载时停止并删除服务，保留 logs/data/config
;
;  依赖插件（NSIS 3.x Unicode 发行版自带）：
;    nsDialogs  nsExec  System
;
;  编译： D:\tools\NSIS-v3.12-CHS-Plus-NanLon\NSIS\makensis.exe install.nsi
;  服务： installscript\nssm\nssm.exe（x64，2.24-101 及以上）
; =============================================================================

Unicode true
XPStyle on
SetCompressor /SOLID lzma
RequestExecutionLevel admin

;--------------------------------
; 产品元数据

!define APP_NAME       "rust-gisserver"
!define APP_PUBLISHER  "rust-gisserver"
!define APP_VERSION    "0.8.2.0"

; 服务名（NSSM 注册用，实际进程 mapserve.exe / manager.exe）
!define SVC_MAP_NAME   "mapserve"
!define SVC_MGR_NAME   "map-manager"

;--------------------------------
; 路径

; 源：项目的 run-env 目录（扁平化部署结构）
!define SRC "${__FILEDIR__}\..\run-env"

; SQL 初始化脚本目录
!define SQL_DIR "${__FILEDIR__}\sql"

; NSSM 位置
!define NSSM_DIR "${__FILEDIR__}\nssm"
!define NSSM_SRC "${NSSM_DIR}\nssm.exe"

;--------------------------------
; 编译期 nssm 检测

!if /FileExists "${NSSM_SRC}"
  !define HAVE_NSSM
!else
  !warning "未找到 nssm.exe，安装包将跳过 Windows 服务注册。路径: ${NSSM_SRC}"
!endif

;--------------------------------
; 安装程序属性

Name "${APP_NAME}"
OutFile "${APP_NAME}_Setup.exe"
InstallDir "d:\${APP_NAME}"
InstallDirRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "InstallLocation"

VIProductVersion "${APP_VERSION}"
VIAddVersionKey "ProductName"    "${APP_NAME}"
VIAddVersionKey "FileVersion"    "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "FileDescription" "${APP_NAME} Installer"
VIAddVersionKey "LegalCopyright" "Copyright (C) ${APP_PUBLISHER}"
VIAddVersionKey "CompanyName"    "${APP_PUBLISHER}"

!include MUI2.nsh
!include LogicLib.nsh
!include nsDialogs.nsh
!include WinMessages.nsh
!include FileFunc.nsh
!include x64.nsh

;--------------------------------
; NSSM 调用宏

!macro NSSM ARGS
  nsExec::ExecToLog '"$INSTDIR\tools\nssm.exe" ${ARGS}'
  Pop $0
!macroend

;--------------------------------
; 界面

!define MUI_ICON          "${NSISDIR}\Contrib\Graphics\Icons\modern-install-blue.ico"
!define MUI_UNICON        "${NSISDIR}\Contrib\Graphics\Icons\modern-uninstall-blue.ico"
!define MUI_ABORTWARNING
!define MUI_CUSTOMFUNCTION_GUIINIT "MyGUIInit"

; 品牌化页头（使用 NSIS 内置 header bitmap）
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_BITMAP       "${NSISDIR}\Contrib\Graphics\Header\nsis3-metro.bmp"
!define MUI_HEADERIMAGE_BITMAP_NOSTRETCH
!define MUI_HEADERIMAGE_RIGHT

; 安装进度页
!define MUI_INSTFILESPAGE_PROGRESSBAR "smooth"

; 页面

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY

; 配置页（自定义）
Page custom ConfigCreate ConfigLeave

; 安装进度
!insertmacro MUI_PAGE_INSTFILES

; 完成页
Page custom FinishCreate

; 卸载页面

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

;--------------------------------
; 语言

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

;--------------------------------
; 变量

Var MapPort        ; map-api 端口
Var MgrPort        ; manager 端口
Var WantService    ; "1"/"0"
Var WantShortcut   ; "1"/"0"
Var ErrFlag
Var VPort
Var VLabel
Var SvcErr

; 控件句柄
Var hMapPort
Var hMgrPort
Var hChkSvc
Var hChkSc
Var hFontTitle
Var hFontBody

;--------------------------------
; 初始化

Function .onInit
  ; 单实例互斥
  System::Call 'kernel32::CreateMutexW(i 0, i 0, w "${APP_NAME}InstallerMutex") p .r0 ?e'
  Pop $0
  StrCmp $0 0 skip_mutex
    MessageBox MB_ICONEXCLAMATION "安装程序已在运行。"
    Abort
  skip_mutex:

  ; 默认配置值（与 .env 一致）
  StrCpy $MapPort "8080"
  StrCpy $MgrPort "6180"
  StrCpy $WantService "1"
  StrCpy $WantShortcut "1"

  ; 读取上次安装的值
  ReadRegStr $0 HKLM "Software\${APP_NAME}" "MapPort"
  ${If} $0 != ""
    StrCpy $MapPort $0
    ReadRegStr $0 HKLM "Software\${APP_NAME}" "MgrPort"
    StrCpy $MgrPort $0
    ReadRegStr $0 HKLM "Software\${APP_NAME}" "WantService"
    StrCpy $WantService $0
  ${EndIf}

  !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Function MyGUIInit
  InitPluginsDir
  ; 字体
  CreateFont $hFontTitle "Segoe UI" "16" "700"
  CreateFont $hFontBody  "Segoe UI" "10" "400"
FunctionEnd

;--------------------------------
; 配置页

Function ConfigCreate
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}

  !insertmacro MUI_HEADER_TEXT "服务配置" "设置两个服务的监听端口"

  ; 标题
  ${NSD_CreateLabel} 0 0 100% 24u "服务端口配置"
  Pop $0
  SendMessage $0 ${WM_SETFONT} $hFontTitle 0

  ; map-api 端口
  ${NSD_CreateLabel} 0 36u 100% 16u "地图服务端口 (map-api)"
  Pop $0
  ${NSD_CreateText} 0 52u 80u 20u "$MapPort"
  Pop $hMapPort

  ; manager 端口
  ${NSD_CreateLabel} 0 82u 100% 16u "管理端端口 (manager)"
  Pop $0
  ${NSD_CreateText} 0 98u 80u 20u "$MgrPort"
  Pop $hMgrPort

  ; 说明
  ${NSD_CreateLabel} 0 128u 100% 60u "数据库密码默认 123456（PostgreSQL 端口 5434），安装后请在 $INSTDIR\.env 中修改 MANAGER_DB_PASSWORD。$\n数据库仅供本机/内网使用：不要对所有 IP 开放（PostgreSQL listen_addresses 与 pg_hba.conf）。$\nJWT 签名密钥将在安装时随机生成，无需手工填写。"
  Pop $0

  ; 选项
  ${NSD_CreateCheckbox} 0 196u 100% 16u "注册为 Windows 服务（开机自启，使用 NSSM）"
  Pop $hChkSvc
  ${If} $WantService == 1
    ${NSD_Check} $hChkSvc
  ${EndIf}

  ${NSD_CreateCheckbox} 0 216u 100% 16u "创建开始菜单快捷方式"
  Pop $hChkSc
  ${If} $WantShortcut == 1
    ${NSD_Check} $hChkSc
  ${EndIf}
FunctionEnd

Function ConfigLeave
  ; 采集控件
  ${NSD_GetText} $hMapPort $MapPort
  ${NSD_GetText} $hMgrPort $MgrPort
  ${NSD_GetState} $hChkSvc $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $WantService "1"
  ${Else}
    StrCpy $WantService "0"
  ${EndIf}
  ${NSD_GetState} $hChkSc $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $WantShortcut "1"
  ${Else}
    StrCpy $WantShortcut "0"
  ${EndIf}

  ; 端口校验
  StrCpy $ErrFlag 0
  StrCpy $VPort $MapPort
  StrCpy $VLabel "map-api"
  Call ValidatePort
  StrCpy $VPort $MgrPort
  StrCpy $VLabel "manager"
  Call ValidatePort
  ${If} $ErrFlag == 1
    Abort
  ${EndIf}

  ${If} $MapPort == $MgrPort
    MessageBox MB_ICONSTOP "两个服务端口不能相同（都是 $MapPort），请修改后继续。"
    Abort
  ${EndIf}

  ${If} $WantService == 1
    !ifndef HAVE_NSSM
      MessageBox MB_ICONEXCLAMATION "本安装包未找到 nssm.exe，无法注册 Windows 服务。$\n安装后可手动运行 start-manager.bat / start-mapserver.bat 启动服务。"
    !endif
  ${EndIf}
FunctionEnd

;--------------------------------
; 端口校验

Function ValidatePort
  ${If} $VPort == ""
    MessageBox MB_ICONSTOP "$VLabel 端口不能为空。"
    StrCpy $ErrFlag 1
    Return
  ${EndIf}
  StrCpy $R9 0
  ${Do}
    StrCpy $R7 $VPort 1 $R9
    ${If} $R7 == ""
      ${ExitDo}
    ${EndIf}
    ${If} $R7 < "0"
    ${OrIf} $R7 > "9"
      MessageBox MB_ICONSTOP "$VLabel 端口必须为纯数字：$VPort"
      StrCpy $ErrFlag 1
      Return
    ${EndIf}
    IntOp $R9 $R9 + 1
  ${Loop}
  ${If} $R9 > 5
    MessageBox MB_ICONSTOP "$VLabel 端口过长：$VPort"
    StrCpy $ErrFlag 1
    Return
  ${EndIf}
  IntOp $R8 $VPort + 0
  ${If} $R8 < 1
  ${OrIf} $R8 > 65535
    MessageBox MB_ICONSTOP "$VLabel 端口超出范围（1-65535）：$VPort"
    StrCpy $ErrFlag 1
  ${EndIf}
FunctionEnd

;--------------------------------
; 完成页

Function FinishCreate
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}

  !insertmacro MUI_HEADER_TEXT "安装完成" "rust-gisserver 已安装到系统"

  ${NSD_CreateLabel} 0 0 100% 24u "安装完成"
  Pop $0
  SendMessage $0 ${WM_SETFONT} $hFontTitle 0

  StrCpy $1 "map-api  : http://127.0.0.1:$MapPort/$\nmanager  : http://127.0.0.1:$MgrPort/$\n安装位置 : $INSTDIR$\n数据库密码默认: 123456（请尽快修改）$\nJWT 密钥: 安装时已随机生成$\n安全提示: 数据库勿对所有 IP 开放（仅本机/内网）"
  ${NSD_CreateLabel} 0 40u 100% 100u "$1"
  Pop $0
  SendMessage $0 ${WM_SETFONT} $hFontBody 0

  ${If} $WantService == 1
    ${NSD_CreateLabel} 0 148u 100% 18u "服务已注册为 Windows 服务（mapserve / map-manager），开机自动启动。"
    Pop $0
  ${Else}
    ${NSD_CreateLabel} 0 148u 100% 18u "未注册 Windows 服务，可通过 start-manager.bat / start-mapserver.bat 手动启动。"
    Pop $0
  ${EndIf}
FunctionEnd

;--------------------------------
; 安装区段

Section "InstallFiles" SEC_FILES
  SectionIn RO

  SetOutPath "$INSTDIR"
  SetOverwrite ifnewer

  ; 顶层文件：.env（密码 123456 已预置）+ 启动脚本
  File "${SRC}\.env"
  File "${SRC}\start-manager.bat"
  File "${SRC}\start-mapserver.bat"
  ; 创建停止脚本
  WriteIniStr "$INSTDIR\stop_all.bat" "" "" "@echo off`r`nnet stop ${SVC_MAP_NAME}`r`nnet stop ${SVC_MGR_NAME}`r`npause"

  ; bin 目录（所有 exe + dll）
  CreateDirectory "$INSTDIR\bin"
  SetOutPath "$INSTDIR\bin"
  File "${SRC}\bin\*.exe"
  File "${SRC}\bin\*.dll"

  ; config 目录（递归）
  CreateDirectory "$INSTDIR\config"
  SetOutPath "$INSTDIR\config"
  File /r "${SRC}\config\*.*"

  ; web 目录（前端静态文件）
  CreateDirectory "$INSTDIR\web"
  SetOutPath "$INSTDIR\web"
  File /r "${SRC}\web\*.*"

  ; share\proj（PROJ 数据）
  CreateDirectory "$INSTDIR\share\proj"
  SetOutPath "$INSTDIR\share\proj"
  File "${SRC}\share\proj\*.*"

  ; data 目录
  CreateDirectory "$INSTDIR\data"
  SetOutPath "$INSTDIR\data"
  File /r "${SRC}\data\*.*"

  ; cache 目录（foyer 缓存 - 不复制占位文件，只创建空目录）
  CreateDirectory "$INSTDIR\cache"
  CreateDirectory "$INSTDIR\cache\foyer"

  ; logs 目录（空）
  CreateDirectory "$INSTDIR\logs"

  ; sql 初始化脚本目录
  ${IfNot} ${FileExists} "${SQL_DIR}\system.sql"
    DetailPrint "SQL 目录不存在：${SQL_DIR}（跳过打包，用户需自备数据库初始化脚本）"
  ${Else}
    CreateDirectory "$INSTDIR\sql"
    SetOutPath "$INSTDIR\sql"
    File "${SQL_DIR}\*.sql"
    File "${SQL_DIR}\*.ps1"
  ${EndIf}

  ; env 多语言配置目录（如果存在）
  ${IfNot} ${FileExists} "${SRC}\env"
  ${Else}
    CreateDirectory "$INSTDIR\env"
    SetOutPath "$INSTDIR\env"
    File /r "${SRC}\env\*.*"
  ${EndIf}

  ; NSSM
  !ifdef HAVE_NSSM
    CreateDirectory "$INSTDIR\tools"
    SetOutPath "$INSTDIR\tools"
    File "${NSSM_SRC}"
  !endif
SectionEnd

Section "Configure" SEC_CONFIG
  SectionIn RO

  ; 写入部署配置：端口 + 随机 JWT 密钥（每次安装生成，避免使用固定密钥）
  DetailPrint "生成部署配置（端口 / 随机 JWT 密钥）..."
  StrCpy $R9 "$TEMP\rgs-configure.ps1"
  FileOpen $0 $R9 w
  FileWrite $0 "$$p = '$INSTDIR\.env'$\r$\n"
  FileWrite $0 "$$c = [IO.File]::ReadAllText($$p)$\r$\n"
  FileWrite $0 "$$c = $$c -replace '(?m)^MAPSRV_PORT=.*$$', 'MAPSRV_PORT=$MapPort'$\r$\n"
  FileWrite $0 "$$c = $$c -replace '(?m)^MAPSERVER_MANAGER_URL=.*$$', 'MAPSERVER_MANAGER_URL=http://127.0.0.1:$MgrPort'$\r$\n"
  FileWrite $0 "$$b = New-Object byte[] 32$\r$\n"
  FileWrite $0 "[Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($$b)$\r$\n"
  FileWrite $0 "$$jwt = ($$b | ForEach-Object { $$_.ToString('x2') }) -join ''$\r$\n"
  FileWrite $0 "$$c = $$c -replace '(?m)^MANAGER_JWT_SECRET=.*$$', ('MANAGER_JWT_SECRET=' + $$jwt)$\r$\n"
  FileWrite $0 "[IO.File]::WriteAllText($$p, $$c, [System.Text.UTF8Encoding]::new($$false))$\r$\n"
  FileWrite $0 "Write-Output '部署配置已生成'$\r$\n"
  FileClose $0
  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$R9"'
  Pop $0
  Pop $1
  Delete $R9
  ${If} $0 != 0
    DetailPrint "配置生成警告: $1"
  ${Else}
    DetailPrint "$1"
  ${EndIf}
SectionEnd

Section "RegisterServices" SEC_SERVICE
  SectionIn RO
  !ifdef HAVE_NSSM
    ${If} $WantService == 1
      StrCpy $SvcErr 0

      ; ---- mapserve (mapserve.exe) ----
      DetailPrint "注册服务 ${SVC_MAP_NAME} ..."
      !insertmacro NSSM 'stop "${SVC_MAP_NAME}"'
      !insertmacro NSSM 'remove "${SVC_MAP_NAME}" confirm'
      Sleep 300
      !insertmacro NSSM 'install "${SVC_MAP_NAME}" "$SYSDIR\cmd.exe"'
      ${If} $0 != 0
        DetailPrint "nssm install ${SVC_MAP_NAME} 失败（退出码 $0）"
        StrCpy $SvcErr 1
      ${Else}
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" DisplayName "mapserve"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" Description "R GIS map engine (mapserve.exe), hosted by NSSM"'
        ; 启动命令：cmd /c start-mapserver.bat（bat 自动加载 .env 并设置 PROJ 环境）
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppParameters "/c start-mapserver.bat"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppDirectory "$INSTDIR"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" Start SERVICE_AUTO_START'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppStdout "$INSTDIR\logs\mapapi_out.log"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppStderr "$INSTDIR\logs\mapapi_err.log"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppAppendStdout true'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppExit Default Restart'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppRestartDelay 5000'
        ; 显式设置环境变量（NSSM 继承 System 环境）
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppEnvironmentExtra "PROJ_DLL=$INSTDIR\bin\proj_9.dll"'
        !insertmacro NSSM 'set "${SVC_MAP_NAME}" AppEnvironmentExtra "PROJ_DATA=$INSTDIR\share\proj"'
        DetailPrint "已注册 ${SVC_MAP_NAME}，发送启动命令..."
        !insertmacro NSSM 'start "${SVC_MAP_NAME}"'
      ${EndIf}

      ; ---- map-manager ----
      DetailPrint "注册服务 ${SVC_MGR_NAME} ..."
      !insertmacro NSSM 'stop "${SVC_MGR_NAME}"'
      !insertmacro NSSM 'remove "${SVC_MGR_NAME}" confirm'
      Sleep 300
      !insertmacro NSSM 'install "${SVC_MGR_NAME}" "$SYSDIR\cmd.exe"'
      ${If} $0 != 0
        DetailPrint "nssm install ${SVC_MGR_NAME} 失败（退出码 $0）"
        StrCpy $SvcErr 1
      ${Else}
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" DisplayName "map-manager"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" Description "R GIS manager (manager.exe), hosted by NSSM"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppParameters "/c start-manager.bat"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppDirectory "$INSTDIR"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" Start SERVICE_AUTO_START'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppStdout "$INSTDIR\logs\manager_out.log"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppStderr "$INSTDIR\logs\manager_err.log"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppAppendStdout true'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppExit Default Restart'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppRestartDelay 5000'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppEnvironmentExtra "PROJ_DLL=$INSTDIR\bin\proj_9.dll"'
        !insertmacro NSSM 'set "${SVC_MGR_NAME}" AppEnvironmentExtra "PROJ_DATA=$INSTDIR\share\proj"'
        DetailPrint "已注册 ${SVC_MGR_NAME}，发送启动命令..."
        !insertmacro NSSM 'start "${SVC_MGR_NAME}"'
      ${EndIf}

      ${If} $SvcErr == 1
        MessageBox MB_ICONEXCLAMATION "部分 Windows 服务注册失败（详见安装日志）。$\n可手动运行 start-manager.bat / start-mapserver.bat 启动。"
      ${Else}
        DetailPrint "两个 Windows 服务已注册并发送启动命令（首次启动加载 PROJ 等运行库可能需 5~30 秒）。"
      ${EndIf}
    ${Else}
      DetailPrint "已取消注册 Windows 服务。"
    ${EndIf}
  !else
    DetailPrint "未打包 nssm.exe，跳过服务注册。"
  !endif
SectionEnd

Section "Shortcuts" SEC_SHORTCUTS
  SectionIn RO
  ${If} $WantShortcut == 1
    CreateDirectory "$SMPROGRAMS\${APP_NAME}"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\启动地图服务.lnk" "$INSTDIR\start-mapserver.bat"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\启动管理端.lnk" "$INSTDIR\start-manager.bat"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\停止所有服务.lnk" "$INSTDIR\stop_all.bat"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\卸载 ${APP_NAME}.lnk" "$INSTDIR\Uninstall.exe"
  ${EndIf}
SectionEnd

Section "-Post" SEC_POST
  ; 记录配置
  WriteRegStr   HKLM "Software\${APP_NAME}" "MapPort"   "$MapPort"
  WriteRegStr   HKLM "Software\${APP_NAME}" "MgrPort"   "$MgrPort"
  WriteRegStr   HKLM "Software\${APP_NAME}" "InstallDir" "$INSTDIR"
  WriteRegStr   HKLM "Software\${APP_NAME}" "WantService" "$WantService"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr   HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "DisplayVersion"  "${APP_VERSION}"
  WriteRegStr   HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "Publisher"       "${APP_PUBLISHER}"
  WriteRegStr   HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "NoRepair" 1
SectionEnd

;--------------------------------
; 卸载

Function un.onInit
  System::Call 'kernel32::CreateMutexW(i 0, i 0, w "${APP_NAME}UninstallMutex") p .r0 ?e'
  Pop $0
  StrCmp $0 0 skip_umutex
    MessageBox MB_ICONEXCLAMATION "卸载程序已在运行。"
    Abort
  skip_umutex:
  ReadRegStr $INSTDIR HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "InstallLocation"
  ${If} $INSTDIR == ""
    ReadRegStr $INSTDIR HKLM "Software\${APP_NAME}" "InstallDir"
  ${EndIf}
FunctionEnd

Section "Uninstall"
  ${If} $INSTDIR == ""
    StrCpy $INSTDIR "$PROGRAMFILES64\${APP_NAME}"
  ${EndIf}

  ; 1) 停止并删除服务
  !ifdef HAVE_NSSM
    ${If} ${FileExists} "$INSTDIR\tools\nssm.exe"
      nsExec::ExecToLog '"$INSTDIR\tools\nssm.exe" stop "${SVC_MAP_NAME}"'
      nsExec::ExecToLog '"$INSTDIR\tools\nssm.exe" remove "${SVC_MAP_NAME}" confirm'
      nsExec::ExecToLog '"$INSTDIR\tools\nssm.exe" stop "${SVC_MGR_NAME}"'
      nsExec::ExecToLog '"$INSTDIR\tools\nssm.exe" remove "${SVC_MGR_NAME}" confirm'
    ${Else}
      nsExec::ExecToLog 'net stop "${SVC_MAP_NAME}"'
      nsExec::ExecToLog 'net stop "${SVC_MGR_NAME}"'
      nsExec::ExecToLog 'sc delete "${SVC_MAP_NAME}"'
      nsExec::ExecToLog 'sc delete "${SVC_MGR_NAME}"'
    ${EndIf}
  !else
    nsExec::ExecToLog 'net stop "${SVC_MAP_NAME}"'
    nsExec::ExecToLog 'net stop "${SVC_MGR_NAME}"'
    nsExec::ExecToLog 'sc delete "${SVC_MAP_NAME}"'
    nsExec::ExecToLog 'sc delete "${SVC_MGR_NAME}"'
  !endif
  Sleep 800

  ; 2) 删除快捷方式
  Delete "$SMPROGRAMS\${APP_NAME}\*.lnk"
  RMDir  "$SMPROGRAMS\${APP_NAME}"

  ; 3) 删除程序文件（保留 logs/data/config/.env）
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\web"
  RMDir /r "$INSTDIR\share"
  RMDir /r "$INSTDIR\tools"
  RMDir /r "$INSTDIR\cache"
  RMDir /r "$INSTDIR\env"
  Delete "$INSTDIR\start-manager.bat"
  Delete "$INSTDIR\start-mapserver.bat"
  Delete "$INSTDIR\stop_all.bat"
  Delete "$INSTDIR\Uninstall.exe"

  MessageBox MB_ICONINFORMATION "已卸载程序组件。以下数据被保留：$\n  $INSTDIR\config$\n  $INSTDIR\data$\n  $INSTDIR\logs$\n  $INSTDIR\.env$\n$\n如需彻底删除，请手动移除 $INSTDIR。"

  ; 4) 注册表
  DeleteRegKey HKLM "Software\${APP_NAME}"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
SectionEnd
