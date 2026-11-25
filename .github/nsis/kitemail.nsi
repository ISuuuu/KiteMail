; KiteMail Windows x64 NSIS 安装脚本
; 支持 /S（静默）、/P（被动）、/UPDATE、/R（安装后启动）。
; 安装目录默认 %LOCALAPPDATA%\KiteMail；升级时沿用注册表里记录的目录。

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
  !define BUILD_DIR "..\..\target\x86_64-pc-windows-msvc\release"
!endif

!ifndef OUT_DIR
  !define OUT_DIR "..\..\dist"
!endif

!define PRODUCT_NAME "KiteMail"
!define PUBLISHER "KiteMail"
!define APP_EXE "kitemail.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\KiteMail"
!define REG_KEY "Software\KiteMail"

Name "${PRODUCT_NAME}"
OutFile "${OUT_DIR}\KiteMail_${VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\KiteMail"
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
Var CallerPid

!macro LockSetup
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\KiteMail.Setup") p .r1 ?e'
  Pop $2
  ${If} $1 == 0
  ${OrIf} $2 == 183
    MessageBox MB_OK|MB_ICONSTOP "另一个 KiteMail 安装或卸载程序正在运行。" /SD IDOK
    SetErrorLevel 2
    Quit
  ${EndIf}
!macroend

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
  StrCpy $PassiveMode 0
  StrCpy $Relaunch 0
  StrCpy $Elevated 0
  StrCpy $CallerPid ""

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

  ${GetOptions} $R0 "/PID=" $CallerPid

  ${GetOptions} $R0 "/ELEVATED" $R1
  ${IfNot} ${Errors}
    StrCpy $Elevated 1
  ${EndIf}

  ${If} ${Silent}
    StrCpy $PassiveMode 1
  ${EndIf}

FunctionEnd

; 安装目录不可写时尝试提权一次，始终使用原目标目录。
Function EnsureWritableInstallDir
  CreateDirectory "$INSTDIR"
  ClearErrors
  GetTempFileName $9 "$INSTDIR"
  ${IfNot} ${Errors}
    Delete "$9"
    Return
  ${EndIf}

  ${If} $Elevated == 1
    SetErrorLevel 2
    Quit
  ${EndIf}

  StrCpy $0 "/ELEVATED"
  ${If} $PassiveMode == 1
    StrCpy $0 "$0 /S"
  ${EndIf}
  ${If} $Relaunch == 1
    StrCpy $0 "$0 /R"
  ${EndIf}
  ${If} $CallerPid != ""
    StrCpy $0 "$0 /PID=$CallerPid"
  ${EndIf}
  ClearErrors
  ExecShell "runas" "$EXEPATH" '$0 /D=$INSTDIR'
  ${If} ${Errors}
    SetErrorLevel 2
  ${EndIf}
  Quit
FunctionEnd

Section "MainSection" SecMain
  SetShellVarContext current
  Call EnsureWritableInstallDir
  !insertmacro LockSetup

  ; 先把新程序释放成临时文件：复制失败（目录不可写、磁盘满）时旧程序仍能启动。
  CreateDirectory "$INSTDIR"
  SetOutPath "$INSTDIR"
  Delete "$INSTDIR\${APP_EXE}.new"
  ClearErrors
  File /oname=${APP_EXE}.new "${BUILD_DIR}\${APP_EXE}"
  ${If} ${Errors}
    Call AbortWithOldApp
  ${EndIf}

  ; 仅等待发起更新的进程；超时或无法打开时停止更新。
  ${If} $CallerPid != ""
    System::Call 'kernel32::OpenProcess(i 0x100000, i 0, i $CallerPid) p .r1 ?e'
    Pop $2
    ${If} $1 == 0
      ${If} $2 != 87
        Call AbortWithOldApp
      ${EndIf}
    ${Else}
      System::Call 'kernel32::WaitForSingleObject(p r1, i 60000) i .r2'
      System::Call 'kernel32::CloseHandle(p r1)'
      ${If} $2 != 0
        Call AbortWithOldApp
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ${If} ${FileExists} "$INSTDIR\${APP_EXE}"
    System::Call 'kernel32::CreateFileW(w "$INSTDIR\${APP_EXE}", i 0xC0000000, i 0, p 0, i 3, i 0, p 0) p .r1'
    ${If} $1 == -1
      Call AbortWithOldApp
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r1)'
    Delete "$INSTDIR\${APP_EXE}.old"
    ClearErrors
    Rename "$INSTDIR\${APP_EXE}" "$INSTDIR\${APP_EXE}.old"
    ${If} ${Errors}
      Call AbortWithOldApp
    ${EndIf}
  ${EndIf}

  ClearErrors
  Rename "$INSTDIR\${APP_EXE}.new" "$INSTDIR\${APP_EXE}"
  ${If} ${Errors}
    Rename "$INSTDIR\${APP_EXE}.old" "$INSTDIR\${APP_EXE}"
    Call AbortWithOldApp
  ${EndIf}
  Delete "$INSTDIR\${APP_EXE}.old"

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
  FileOpen $9 "$INSTDIR\kitemail-update-error.log" a
  ${IfNot} ${Errors}
    FileWrite $9 "[${VERSION}] 无法写入 $INSTDIR\${APP_EXE}$\r$\n"
    FileClose $9
  ${EndIf}
  Delete "$INSTDIR\${APP_EXE}.new"
  ${If} $PassiveMode != 1
    MessageBox MB_ICONSTOP "无法写入 $\"$INSTDIR$\"，安装已取消。请换一个可写目录，或以管理员身份运行安装包。"
  ${EndIf}
  SetErrorLevel 2
  Quit
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro LockSetup

  ClearErrors
  Delete "$INSTDIR\${APP_EXE}"
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "请先退出 KiteMail，然后重新卸载。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}

  Delete "$SMPROGRAMS\${PRODUCT_NAME}.lnk"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  Delete "$INSTDIR\uninstall.exe"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${REG_KEY}"

  RMDir "$INSTDIR"
SectionEnd
