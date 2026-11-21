BJTU campus for Windows 10/11 x64

Double-click campus.exe (or open PowerShell in this folder):
  .\campus.exe
  .\campus.exe auth login
  .\campus.exe homework download <ID> -o ./homework
  .\campus.exe homework submit <ID> --file report.pdf

After interactive login on Windows, you will be prompted to create a
"交大自由行 CLI" desktop shortcut using campus.ico. Initial install
automatically synchronizes all tabs once login completes.

campus.ps1 is an optional launcher. If local policy restricts scripts,
use campus.exe directly.

Local state: %LOCALAPPDATA%\campus-portal
Default credential store: Windows Credential Manager
Use "campus.exe sync" for synchronization. The Linux systemd
timer command is only available on Linux.
