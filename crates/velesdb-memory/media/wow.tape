# VelesDB-memory — "wow" clip (source of truth, regenerable).
# Render from this directory:
#   cd crates/velesdb-memory/media && vhs wow.tape
# Produces wow.gif (committed, used in the README) and wow.mp4 (not committed —
# a launch/social asset, regenerate when needed). Fully offline, reproducible.
# Needs: vhs + the JetBrains Mono font (brew install vhs --cask font-jetbrains-mono).

Output wow.gif
Output wow.mp4

Require cargo

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 22
Set Width 1280
Set Height 760
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms
Set Margin 24
Set Padding 24
Set BorderRadius 10

# Pre-build hidden so the run lands instantly on camera.
Hide
Type "cargo build -q -p velesdb-memory --example wow_offline 2>/dev/null"
Enter
Sleep 2s
Type "clear"
Enter
Show

Sleep 600ms
Type "# Your AI agent forgets WHY — and a similarity search can't connect the dots."
Sleep 1500ms
Enter
Sleep 1500ms

Type "cargo run -q -p velesdb-memory --example wow_offline"
Sleep 800ms
Enter
# Let the recall (miss) → why (graph reaches the ticket) contrast land and read.
Sleep 7s

Type "# The vector was blind. The graph connected it. Offline. One binary.  → velesdb.com"
Sleep 1200ms
Enter
Sleep 3s
