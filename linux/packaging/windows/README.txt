BJTU campus for Windows 10/11 x64

Double-click campus.exe (or open PowerShell in this folder):
  .\campus.exe
  .\campus.exe auth login
  .\campus.exe mail list
  .\campus.exe mail download <ID>
  .\campus.exe homework download <ID>
  .\campus.exe homework submit <ID> --file report.pdf

Default download folder for courseware, homework attachments, and mail
attachments is your Windows Downloads folder (override with -o <DIR>).

If Windows SmartScreen shows "Windows protected your PC" (unsigned open-source
binary), click "More info" -> "Run anyway", or run in PowerShell once:
  Unblock-File .\campus.exe

After interactive login on Windows, you will be prompted to create a
"交大自由行 CLI" desktop shortcut using campus.ico. Initial install
automatically synchronizes all tabs once login completes.

campus.ps1 unblocks campus.exe and launches it.

Local state: %LOCALAPPDATA%\campus-portal
Default credential store: Windows Credential Manager
Use "campus.exe sync" for synchronization. The Linux systemd
timer command is only available on Linux.
