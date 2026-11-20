; Flimmer installer hooks.
; The web UI's file names change with every build (content hashes). An update installs the new ones next to
; the old ones and the uninstaller only knows the newest list, so old assets would pile up. Start clean.

!macro NSIS_HOOK_PREINSTALL
  RMDir /r "$INSTDIR\web\dist"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    RMDir /r "$INSTDIR\web"
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
