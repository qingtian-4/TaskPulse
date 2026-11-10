; NSIS is current-user. Keep the existing Run value name and approval bytes.
!macro NSIS_HOOK_PREINSTALL
  ; The stock template checks the new executable. The recorded old executable
  ; must also be closed before the template replaces files and shortcuts.
  ReadRegStr $R7 SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $R7 == "agent-tray.exe"
    !insertmacro CheckIfAppIsRunning "$INSTDIR\agent-tray.exe" "${PRODUCTNAME}"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $R7 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  ${If} $R7 == "$\"$INSTDIR\agent-tray.exe$\""
  ${OrIf} $R7 == "$INSTDIR\agent-tray.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  ${EndIf}
!macroend
