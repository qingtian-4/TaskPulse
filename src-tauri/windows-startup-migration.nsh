; The installer owns a CLI-only compatibility bridge, separately from the GUI.
!define TASKPULSE_LEGACY_BRIDGE "${__FILEDIR__}\target\taskpulse-legacy-${ARCH}.exe"

; NSIS is current-user. Keep the existing Run value name and approval bytes.
!macro NSIS_HOOK_PREINSTALL
  ; The stock template checks the new executable. The recorded old executable
  ; must also be closed before the template replaces files and shortcuts.
  ${If} ${FileExists} "$INSTDIR\agent-tray.exe"
    !insertmacro CheckIfAppIsRunning "$INSTDIR\agent-tray.exe" "${PRODUCTNAME}"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File /oname=agent-tray.exe "${TASKPULSE_LEGACY_BRIDGE}"
  ReadRegStr $R7 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  ${If} $R7 == "$\"$INSTDIR\agent-tray.exe$\""
  ${OrIf} $R7 == "$INSTDIR\agent-tray.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\agent-tray.exe"
    !insertmacro CheckIfAppIsRunning "$INSTDIR\agent-tray.exe" "${PRODUCTNAME}"
  ${EndIf}
  Delete "$INSTDIR\agent-tray.exe"
!macroend
