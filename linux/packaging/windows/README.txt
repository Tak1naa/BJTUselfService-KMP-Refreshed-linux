BJTU campus for Windows 10/11 x64

Extract the archive and open PowerShell in its folder.
  .\campus.exe --version
  .\campus.exe auth login
  .\campus.exe tui
  .\campus.exe grades gpa --offline

campus.ps1 is an optional launcher. If local policy restricts scripts,
use campus.exe directly.

Local state: %LOCALAPPDATA%\campus-portal
Default credential store: Windows Credential Manager
Use "campus.exe sync" for synchronization. The Linux systemd
timer command is only available on Linux.
