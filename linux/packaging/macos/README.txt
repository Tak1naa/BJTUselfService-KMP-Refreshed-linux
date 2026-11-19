交大自由行 CLI (campus) for macOS (Apple Silicon & Intel Universal)

Extract the archive and run in Terminal:
  ./campus --version
  ./campus auth login
  ./campus tui
  ./campus grades gpa --offline

If macOS Gatekeeper blocks an unnotarized binary downloaded from the browser:
  xattr -d com.apple.quarantine ./campus

To install onto PATH (e.g. Homebrew prefix on Apple Silicon):
  install -m 755 campus /opt/homebrew/bin/campus

Optional shell completions are included under completions/ (bash, zsh, fish).
Local state: ~/.local/share/campus-portal (or $XDG_DATA_HOME/campus-portal)
Default credential store: macOS Keychain
Periodic background sync:
  campus sync timer install --every 1h   # uses macOS launchd (~/Library/LaunchAgents)
  campus sync timer remove
