; Hylki Windows x64 NSIS 安装脚本
; 支持 /S（静默）、/P（被动）、/UPDATE、/R（安装后启动）。
; 安装目录默认 %LOCALAPPDATA%\Hylki；升级时沿用注册表里记录的目录。

Unicode True

!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!ifndef VI_VERSION
  !define VI_VERSION "${VERSION}.0"
!endif

!ifndef BUILD_DIR
  !define BUILD_DIR "..\..\desktop\target\x86_64-pc-windows-msvc\release"
!endif

!ifndef OUT_DIR
  !define OUT_DIR "..\..\dist"
!endif

!define PRODUCT_NAME "Hylki"
!define PUBLISHER "Hyprlab"
!define APP_EXE "hylki-windows.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hylki"
!define REG_KEY "Software\Hylki"

Name "${PRODUCT_NAME}"
OutFile "${OUT_DIR}\Hylki_${VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\Hylki"
InstallDirRegKey HKCU "${REG_KEY}" ""
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "${VI_VERSION}"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "AGPL-3.0-or-later"
VIAddVersionKey "FileDescription" "${PRODUCT_NAME} Installer"
VIAddVersionKey "FileVersion" "${VI_VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!define MUI_ICON "..\..\data\icons\hicolor\256x256\apps\co.hyprlab.Hylki.ico"
!define MUI_UNICON "..\..\data\icons\hicolor\256x256\apps\co.hyprlab.Hylki.ico"
!define MUI_ABORTWARNING

Var PassiveMode
Var Relaunch
Var Elevated

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "运行 ${PRODUCT_NAME}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ; 升级时沿用上次的安装目录
  ${If} $INSTDIR == "$LOCALAPPDATA\Hylki"
    ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
    ${If} $0 != ""
      StrCpy $INSTDIR $0
    ${Else}
      ReadRegStr $0 HKCU "${REG_KEY}" ""
      ${If} $0 != ""
        StrCpy $INSTDIR $0
      ${EndIf}
    ${EndIf}
  ${EndIf}

  StrCpy $PassiveMode 0
  StrCpy $Relaunch 0
  StrCpy $Elevated 0

  ${GetParameters} $R0
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
    SetSilent silent
  ${EndIf}

  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
    SetSilent silent
  ${EndIf}

  ${GetOptions} $R0 "/R" $R1
  ${IfNot} ${Errors}
    StrCpy $Relaunch 1
  ${EndIf}

  ${GetOptions} $R0 "/ELEVATED" $R1
  ${IfNot} ${Errors}
    StrCpy $Elevated 1
  ${EndIf}

  ${If} ${Silent}
    StrCpy $PassiveMode 1
  ${EndIf}

  Call EnsureWritableInstallDir
FunctionEnd

; 安装目录不可写时（例如装在 Program Files 却以普通权限静默更新）先提权重试一次，
; 仍不可写就退回当前用户目录，避免更新把程序弄丢。
Function EnsureWritableInstallDir
  CreateDirectory "$INSTDIR"
  ClearErrors
  FileOpen $9 "$INSTDIR\.hylki-write-test" w
  ${IfNot} ${Errors}
    FileClose $9
    Delete "$INSTDIR\.hylki-write-test"
    Return
  ${EndIf}

  ${If} $Elevated == 1
    StrCpy $INSTDIR "$LOCALAPPDATA\Hylki"
    Return
  ${EndIf}

  ExecShell "runas" '"$EXEPATH" $R0 /ELEVATED'
  Quit
FunctionEnd

Section "MainSection" SecMain
  SetShellVarContext current

  ; 先把新程序释放成临时文件：复制失败（目录不可写、磁盘满）时旧程序仍能启动。
  CreateDirectory "$INSTDIR"
  SetOutPath "$INSTDIR"
  Delete "$INSTDIR\${APP_EXE}.new"
  ClearErrors
  File /oname=${APP_EXE}.new "${BUILD_DIR}\${APP_EXE}"
  ${If} ${Errors}
    Call AbortWithOldApp
  ${EndIf}

  nsExec::Exec 'taskkill /F /IM ${APP_EXE}'
  Pop $0
  Sleep 800

  Delete "$INSTDIR\${APP_EXE}.old"
  ClearErrors
  Rename "$INSTDIR\${APP_EXE}" "$INSTDIR\${APP_EXE}.old"
  ${If} ${Errors}
    Sleep 700
    ClearErrors
    Rename "$INSTDIR\${APP_EXE}" "$INSTDIR\${APP_EXE}.old"
  ${EndIf}
  ${If} ${Errors}
    Call AbortWithOldApp
  ${EndIf}

  ClearErrors
  Rename "$INSTDIR\${APP_EXE}.new" "$INSTDIR\${APP_EXE}"
  ${If} ${Errors}
    Rename "$INSTDIR\${APP_EXE}.old" "$INSTDIR\${APP_EXE}"
    Call AbortWithOldApp
  ${EndIf}
  Delete "$INSTDIR\${APP_EXE}.old"

  File "/oname=LICENSE" "..\..\LICENSE"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${REG_KEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" '"$INSTDIR\${APP_EXE}",0'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  CreateShortCut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
SectionEnd

Function .onInstSuccess
  ; 静默 / 被动更新（/S、/P、/UPDATE）由安装包自己拉起程序：
  ; 调用方进程此时已被结束，不拉起用户就会以为程序没装好。
  ${If} $PassiveMode == 1
    Exec '"$INSTDIR\${APP_EXE}"'
  ${ElseIf} $Relaunch == 1
    Exec '"$INSTDIR\${APP_EXE}"'
  ${EndIf}
FunctionEnd

Function AbortWithOldApp
  FileOpen $9 "$INSTDIR\hylki-update-error.log" a
  ${IfNot} ${Errors}
    FileWrite $9 "[${VERSION}] 无法写入 $INSTDIR\${APP_EXE}$\r$\n"
    FileClose $9
  ${EndIf}
  Delete "$INSTDIR\${APP_EXE}.new"
  ${If} $PassiveMode != 1
    MessageBox MB_ICONSTOP "无法写入 $\"$INSTDIR$\"，安装已取消。请换一个可写目录，或以管理员身份运行安装包。"
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\${APP_EXE}"
    Exec '"$INSTDIR\${APP_EXE}"'
  ${EndIf}
  SetErrorLevel 2
  Quit
FunctionEnd

Section "Uninstall"
  SetShellVarContext current

  nsExec::Exec 'taskkill /F /IM ${APP_EXE}'
  Sleep 500

  Delete "$SMPROGRAMS\${PRODUCT_NAME}.lnk"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$INSTDIR\LICENSE"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${REG_KEY}"

  RMDir "$INSTDIR"
SectionEnd
