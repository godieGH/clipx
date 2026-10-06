!macro NSIS_HOOK_POSTINSTALL

  ; Register "Send with ClipX" for files.
  WriteRegStr HKCU "Software\Classes\*\shell\ClipX" "" "Send with ClipX"

  ; Use the main ClipX application's icon.
  WriteRegStr HKCU "Software\Classes\*\shell\ClipX" "Icon" "$INSTDIR\clipx-app.exe,0"

  ; Register the command.
  WriteRegStr HKCU "Software\Classes\*\shell\ClipX\command" "" '"$INSTDIR\clipx-send.exe" "%1"'

!macroend


!macro NSIS_HOOK_PREUNINSTALL

  ; Remove the Explorer integration when ClipX is uninstalled.
  DeleteRegKey HKCU "Software\Classes\*\shell\ClipX"

!macroend