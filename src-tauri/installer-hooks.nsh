; UsageDeck uninstall cleanup, wired in by bundle.windows.nsis.installerHooks (tauri.conf.json).
;
; Tauri's own uninstall section already, when not updating (no /UPDATE):
;   - deletes HKCU\Software\Microsoft\Windows\CurrentVersion\Run "${PRODUCTNAME}";
;   - with "Delete the application data" ticked, removes %APPDATA%\${BUNDLEID} and
;     %LOCALAPPDATA%\${BUNDLEID} (settings, saved position, logs, crash files).
; This hook runs after that and adds:
;   - the Task Manager startup entry tauri-plugin-autostart leaves behind;
;   - with "Delete the application data" ticked, the API keys in Windows Credential Manager.
; The keys follow the app data checkbox, not every uninstall: an upgrade through the installer's
; "uninstall before installing" choice runs this uninstaller without /UPDATE, and deleting the keys
; while the settings that name their accounts stay would lose them on every manual upgrade.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; tauri-plugin-autostart names the value after the product name (package_info().name), and
    ; auto-launch writes it under Run and, on enable, under StartupApproved\Run (never removed on
    ; disable). Run is also deleted by Tauri's template; repeated here so it never depends on it.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"
  ${EndIf}

  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; The keyring crate stores each key as a generic credential with target "<user>.<service>"
    ; (keyring windows.rs: format!("{user}.{service}")); the service is KEYRING_SERVICE =
    ; "UsageDeck" (store.rs). cmdkey /list prints them as "LegacyGeneric:target=<user>.UsageDeck".
    ; The match keys on the text after the first ':' so a localized "Target:" label still works.
    nsExec::Exec `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "foreach ($$l in (cmdkey /list)) { if ($$l -match ':\s*(\S+\.UsageDeck)\s*$$') { $$t = $$Matches[1]; cmdkey ('/delete:' + $$t) | Out-Null; if ($$LASTEXITCODE -ne 0 -and $$t -match 'target=(.+)$$') { cmdkey ('/delete:' + $$Matches[1]) | Out-Null } } }"`
    Pop $0
  ${EndIf}
!macroend
