!include "MUI2.nsh"

Name "odx"
OutFile "odx-installer.exe"
InstallDir "$PROGRAMFILES64\odx"
RequestExecutionLevel admin

!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "odx" SEC_MAIN
  SetOutPath "$INSTDIR"
  File "odx.exe"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\odx"
  CreateShortcut "$SMPROGRAMS\odx\odx.lnk" "$INSTDIR\odx.exe"
  CreateShortcut "$SMPROGRAMS\odx\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\odx.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\odx\odx.lnk"
  Delete "$SMPROGRAMS\odx\Uninstall.lnk"
  RMDir "$SMPROGRAMS\odx"
SectionEnd

