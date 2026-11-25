; Optional components (NSIS installer only).
;
; clipx-core and clipx-app are always installed. The template installs every
; externalBin unconditionally, so an unwanted component is deleted right after
; the file copy. Choices are remembered in HKCU so an update does not ask again
; (silent installs: Send = yes, CLI = no).
;
; CLI: clipx.exe stays in $INSTDIR. Only $INSTDIR\cli (holding the clipx.cmd
; shim and nothing else) goes on the user PATH, so clipx-core, clipx-send and
; clipx-app are not reachable from PATH.

Var ClipxWantSend
Var ClipxWantCli

!define CLIPX_REG_KEY "Software\godiegh\Clipx"

!macro NSIS_HOOK_POSTINSTALL

  ; ---- ask (or recall) ----
  ReadRegStr $ClipxWantSend HKCU "${CLIPX_REG_KEY}" "SendWithClipx"
  ${If} $ClipxWantSend == ""
    MessageBox MB_YESNO|MB_ICONQUESTION "Add $\"Send with ClipX$\" to the Explorer right-click menu?$\r$\n$\r$\n(Installs clipx-send.exe)" /SD IDYES IDYES clipx_send_yes IDNO clipx_send_no
    clipx_send_yes:
      StrCpy $ClipxWantSend "1"
      Goto clipx_send_done
    clipx_send_no:
      StrCpy $ClipxWantSend "0"
    clipx_send_done:
    WriteRegStr HKCU "${CLIPX_REG_KEY}" "SendWithClipx" "$ClipxWantSend"
  ${EndIf}

  ReadRegStr $ClipxWantCli HKCU "${CLIPX_REG_KEY}" "Cli"
  ${If} $ClipxWantCli == ""
    MessageBox MB_YESNO|MB_ICONQUESTION "Install the clipx command-line tool and add it to your PATH?$\r$\n$\r$\n(Only the clipx command is exposed; the other ClipX programs stay off PATH.)" /SD IDNO IDYES clipx_cli_yes IDNO clipx_cli_no
    clipx_cli_yes:
      StrCpy $ClipxWantCli "1"
      Goto clipx_cli_done
    clipx_cli_no:
      StrCpy $ClipxWantCli "0"
    clipx_cli_done:
    WriteRegStr HKCU "${CLIPX_REG_KEY}" "Cli" "$ClipxWantCli"
  ${EndIf}

  ; ---- Send with ClipX ----
  ${If} $ClipxWantSend == "1"
    WriteRegStr HKCU "Software\Classes\*\shell\ClipX" "" "Send with ClipX"
    WriteRegStr HKCU "Software\Classes\*\shell\ClipX" "Icon" "$INSTDIR\clipx-app.exe,0"
    WriteRegStr HKCU "Software\Classes\*\shell\ClipX\command" "" '"$INSTDIR\clipx-send.exe" "%1"'
  ${Else}
    Delete "$INSTDIR\clipx-send.exe"
    DeleteRegKey HKCU "Software\Classes\*\shell\ClipX"
  ${EndIf}

  ; ---- clipx CLI + PATH shim ----
  ${If} $ClipxWantCli == "1"
    CreateDirectory "$INSTDIR\cli"
    FileOpen $0 "$INSTDIR\cli\clipx.cmd" w
    FileWrite $0 "@echo off$\r$\n"
    FileWrite $0 '"%~dp0..\clipx.exe" %*$\r$\n'
    FileClose $0
    ; Add $INSTDIR\cli to the user PATH (registry, unexpanded, kept as REG_EXPAND_SZ).
    nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "$$d='$INSTDIR\cli'; $$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment',$$true); $$p=[string]$$k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $$a=@($$p -split ';' | Where-Object { $$_ -and ($$_ -ne $$d) }); $$k.SetValue('Path',(($$a+$$d) -join ';'),[Microsoft.Win32.RegistryValueKind]::ExpandString); $$k.Close()"`
    Pop $0
    System::Call 'user32::SendMessageTimeoutW(p 0xffff, i 0x1A, p 0, w "Environment", i 2, i 5000, *p .r0)'
  ${Else}
    Delete "$INSTDIR\clipx.exe"
  ${EndIf}

!macroend


!macro NSIS_HOOK_PREUNINSTALL

  ; Explorer integration
  DeleteRegKey HKCU "Software\Classes\*\shell\ClipX"

  ; Remove the shim directory from the user PATH, then delete it.
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "$$d='$INSTDIR\cli'; $$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment',$$true); $$p=[string]$$k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $$a=@($$p -split ';' | Where-Object { $$_ -and ($$_ -ne $$d) }); $$k.SetValue('Path',($$a -join ';'),[Microsoft.Win32.RegistryValueKind]::ExpandString); $$k.Close()"`
  Pop $0
  System::Call 'user32::SendMessageTimeoutW(p 0xffff, i 0x1A, p 0, w "Environment", i 2, i 5000, *p .r0)'
  Delete "$INSTDIR\cli\clipx.cmd"
  RMDir "$INSTDIR\cli"

  ; Forget remembered component choices
  DeleteRegKey HKCU "${CLIPX_REG_KEY}"

!macroend
