; -*- coding: utf-8 -*-
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "StrFunc.nsh"
${StrRep}

!ifndef PACKAGE_INCLUDE
  !error "Use scripts/package-installer.ps1 to build this installer."
!endif
!include "${PACKAGE_INCLUDE}"

Name "PopChat ${APP_VERSION} — LOCAL BUILD"
OutFile "${SETUP_PATH}"
InstallDir "$LOCALAPPDATA\Programs\PopChat"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
CRCCheck force
ManifestDPIAware true
BrandingText "PopChat · Evis Lai · LOCAL BUILD"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "PopChat Windows Setup"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "FileDescription" "PopChat NSIS installer — LOCAL BUILD"
VIAddVersionKey /LANG=1033 "CompanyName" "Evis Lai"
VIAddVersionKey /LANG=1033 "LegalCopyright" "2026 Le Chen; Windows port 2026 Evis Lai"

!ifdef TEST_INSTALLER
  !define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\PopChatWindowsInstallerTest"
  !define SHORTCUT_NAME "PopChat Installer Test"
!else
  !define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\PopChatWindows"
  !define SHORTCUT_NAME "PopChat"
!endif

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "Install PopChat for the current Windows user. No administrator access is required.$\r$\n$\r$\nClose PopChat before installing. Settings, services, credentials and chat history are preserved. This installer does not enable startup or launch the app automatically."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE_DIR}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!define MUI_UNCONFIRMPAGE_TEXT_TOP "Remove PopChat program files and shortcuts? Settings, credentials, chat history, unknown files and update backups will be kept."
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "TradChinese"

Var Snapshot
Var Entry
Var ProcessName
Var ProcessHandle
Var ProcessPath
Var ManifestText
Var InstallMutex

; No taskkill, automatic launch, reboot replacement or recursive deletion.
; Inspect target-path processes (including hidden/tray-only apps). A different
; portable/test installation does not block an isolated installer smoke test.
!macro ProcessGuard PREFIX
Function ${PREFIX}CheckProcesses
  System::Call 'kernel32::CreateToolhelp32Snapshot(i 2, i 0) p.r0'
  StrCpy $Snapshot $0
  StrCmp $Snapshot -1 guard_failed
  System::Alloc 556
  Pop $Entry
  System::Call '*$Entry(i 556)'
  System::Call 'kernel32::Process32FirstW(p $Snapshot, p $Entry) i.r0'
  StrCmp $0 0 guard_failed_close
  process_loop:
    IntOp $0 $Entry + 36
    System::Call '*$0(&w260 .r1)'
    StrCpy $ProcessName $1
    StrCmp $ProcessName "popchat_windows.exe" inspect_process
    ; An updater might be executing from a temporary directory. Fail closed.
    StrCmp $ProcessName "popchat_updater.exe" guard_busy
    Goto process_next
  inspect_process:
    IntOp $0 $Entry + 8
    System::Call '*$0(i .r1)'
    System::Call 'kernel32::OpenProcess(i 0x1000, i 0, i r1) p.r0'
    StrCpy $ProcessHandle $0
    StrCmp $ProcessHandle 0 guard_failed_close
    System::Call 'kernel32::QueryFullProcessImageNameW(p $ProcessHandle, i 0, w .r1, *i ${NSIS_MAX_STRLEN}) i.r0'
    StrCpy $ProcessPath $1
    StrCpy $2 $0
    System::Call 'kernel32::CloseHandle(p $ProcessHandle)'
    StrCmp $2 0 guard_failed_close
    StrCmp $ProcessPath "$INSTDIR\popchat_windows.exe" guard_busy
  process_next:
    System::Call 'kernel32::Process32NextW(p $Snapshot, p $Entry) i.r0 ?e'
    Pop $1
    StrCmp $0 0 0 process_loop
    StrCmp $1 18 process_done guard_failed_close
  guard_busy:
    System::Free $Entry
    System::Call 'kernel32::CloseHandle(p $Snapshot)'
    MessageBox MB_OK|MB_ICONEXCLAMATION "Close PopChat (including its tray icon) and any update operation, then run Setup again. No process was terminated." /SD IDOK
    SetErrorLevel 2
    Abort
  guard_failed_close:
    System::Free $Entry
    System::Call 'kernel32::CloseHandle(p $Snapshot)'
  guard_failed:
    MessageBox MB_OK|MB_ICONSTOP "Could not safely inspect running PopChat processes. Installation was stopped." /SD IDOK
    SetErrorLevel 3
    Abort
  process_done:
    System::Free $Entry
    System::Call 'kernel32::CloseHandle(p $Snapshot)'
FunctionEnd
!macroend
!insertmacro ProcessGuard ""
!insertmacro ProcessGuard "un."

!macro InstallerLock
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\PopChat.NSIS.Install") p.r0 ?e'
  Pop $1
  StrCpy $InstallMutex $0
  ${If} $0 == 0
  ${OrIf} $1 == 183
    MessageBox MB_OK|MB_ICONEXCLAMATION "Another PopChat installer is running. Close it and retry." /SD IDOK
    SetErrorLevel 7
    Abort
  ${EndIf}
!macroend

Function .onInit
  SetShellVarContext current
  SetRegView 64
  !insertmacro InstallerLock
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "PopChat requires 64-bit Windows." /SD IDOK
    SetErrorLevel 4
    Abort
  ${EndIf}
FunctionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 64
  !insertmacro InstallerLock
FunctionEnd

; Write BOM-free UTF-8 JSON using Win32, not ANSI FileWrite or UTF-16.
; The updater verifies this manifest, including Unicode installation paths.
Function WriteManifest
  ${StrRep} $0 "$INSTDIR" "\" "\\"
  StrCpy $ManifestText '{"app":"PopChatWindows","version":"${APP_VERSION}","install_dir":"$0","executable":"popchat_windows.exe","sha256":"${APP_SHA256}","updater_sha256":"${HELPER_SHA256}","author":"Evis Lai","build_channel":"LOCAL BUILD","installer":"NSIS"}'
  StrCpy $0 $ManifestText
  System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0, w r0, i -1, p 0, i 0, p 0, p 0) i.r1'
  IntCmp $1 0 manifest_failed
  System::Alloc $1
  Pop $2
  System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0, w r0, i -1, p r2, i r1, p 0, p 0) i.r3'
  IntCmp $3 0 manifest_free_failed
  IntOp $1 $1 - 1
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\install-manifest.json.new", i 0x40000000, i 0, p 0, i 2, i 0x80, p 0) p.r4'
  StrCmp $4 -1 manifest_free_failed
  System::Call 'kernel32::WriteFile(p r4, p r2, i r1, *i .r5, p 0) i.r6'
  System::Call 'kernel32::CloseHandle(p r4)'
  System::Free $2
  StrCmp $6 0 manifest_failed
  StrCmp $5 $1 0 manifest_failed
  System::Call 'kernel32::MoveFileExW(w "$INSTDIR\install-manifest.json.new", w "$INSTDIR\install-manifest.json", i 9) i.r0'
  StrCmp $0 0 manifest_failed
  Return
  manifest_free_failed:
    System::Free $2
  manifest_failed:
    SetErrorLevel 5
    MessageBox MB_OK|MB_ICONSTOP "Could not write the update manifest. Installation did not complete." /SD IDOK
    Abort
FunctionEnd

Section "PopChat" SEC_APP
  GetFullPathName $INSTDIR "$INSTDIR"
  Call CheckProcesses
  SetOutPath "$INSTDIR"
  ; Preserve the previous binaries/manifest for manual recovery if a later
  ; extraction fails. Never touch application data or credential storage.
  IfFileExists "$INSTDIR\popchat_windows.exe" 0 backup_done
    System::Call 'kernel32::GetTickCount() i.r0'
    CreateDirectory "$INSTDIR\.popchat-before-nsis-$0"
    ClearErrors
    CopyFiles /SILENT "$INSTDIR\popchat_windows.exe" "$INSTDIR\.popchat-before-nsis-$0"
    IfErrors install_failed
    IfFileExists "$INSTDIR\popchat_updater.exe" 0 backup_manifest
    CopyFiles /SILENT "$INSTDIR\popchat_updater.exe" "$INSTDIR\.popchat-before-nsis-$0"
    IfErrors install_failed
  backup_manifest:
    IfFileExists "$INSTDIR\install-manifest.json" 0 backup_done
    CopyFiles /SILENT "$INSTDIR\install-manifest.json" "$INSTDIR\.popchat-before-nsis-$0"
    IfErrors install_failed
  backup_done:
  ClearErrors
  SetOverwrite on
  !include "${INSTALL_FILES}"
  IfErrors install_failed
  SetOutPath "$INSTDIR\licenses\nsis"
  File /oname=COPYING "${NSIS_LICENSE}"
  IfErrors install_failed
  Call WriteManifest
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${SHORTCUT_NAME}.lnk" "$INSTDIR\popchat_windows.exe"
  CreateShortcut "$DESKTOP\${SHORTCUT_NAME}.lnk" "$INSTDIR\popchat_windows.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "PopChat — LOCAL BUILD"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Evis Lai"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\popchat_windows.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/lsyuan36/popchat_windows"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" ${SIZE_KB}
  IfErrors install_failed
  Goto install_done
  install_failed:
    SetErrorLevel 6
    MessageBox MB_OK|MB_ICONSTOP "Installation could not complete. Existing backups and user data were kept. Close PopChat and retry." /SD IDOK
    Abort
  install_done:
SectionEnd

Section "Uninstall"
  Call un.CheckProcesses
  Delete "$INSTDIR\licenses\nsis\COPYING"
  RMDir "$INSTDIR\licenses\nsis"
  !include "${UNINSTALL_FILES}"
  IfFileExists "$INSTDIR\popchat_windows.exe" uninstall_failed
  IfFileExists "$INSTDIR\popchat_updater.exe" uninstall_failed
  Delete "$INSTDIR\install-manifest.json"
  Delete "$INSTDIR\Uninstall.exe"
  ; Do not remove another installation's shared shortcuts/registration.
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 == $INSTDIR
    Delete "$SMPROGRAMS\${SHORTCUT_NAME}.lnk"
    Delete "$DESKTOP\${SHORTCUT_NAME}.lnk"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ${EndIf}
  RMDir "$INSTDIR"
  Goto uninstall_done
  uninstall_failed:
    SetErrorLevel 8
    MessageBox MB_OK|MB_ICONSTOP "Program files are locked. Close PopChat and retry uninstalling. User data was kept." /SD IDOK
    Abort
  uninstall_done:
SectionEnd
