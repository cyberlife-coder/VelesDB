# VelesDB-memory — "wow" clip (source of truth). Real semantic recall, shown
# with a LOCAL model so the scores are genuine — still offline, no API key.
#
# Render from the repo root:   vhs crates/velesdb-memory/media/wow.tape
# Needs: vhs + JetBrains Mono + a local Ollama with the embedding model:
#   brew install vhs && brew install --cask font-jetbrains-mono
#   ollama pull all-minilm
# (The example also runs with no Ollama at all — `cargo run -p velesdb-memory
#  --example wow_offline` uses the deterministic embedder; the scores differ but
#  the wedge is identical.)

Output crates/velesdb-memory/media/wow.gif
Output crates/velesdb-memory/media/wow.mp4

Require cargo

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 24
Set Width 1300
Set Height 880
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms
Set Margin 26
Set Padding 26
Set BorderRadius 10

# Hidden setup: build once with the real-embedder feature, point it at the local
# model, clear the screen. The commands below are then instant and clean.
Hide
Type "cargo build -q --features ollama -p velesdb-memory --example wow_offline 2>/dev/null"
Enter
Sleep 4s
Type "export VELESDB_MEMORY_EMBEDDER=ollama"
Enter
Type "clear"
Enter
Show

Sleep 700ms
Type "# memory backed by a real local model (Ollama all-minilm) — still offline"
Sleep 400ms
Enter
Sleep 1200ms

# Beat 1 — vector: the work surfaces from both projects, never the person.
Type "./target/debug/examples/wow_offline recall"
Sleep 500ms
Enter
Sleep 3800ms

# Beat 2 — ColumnStore: an exact-match metadata filter scopes it to one project.
Type "./target/debug/examples/wow_offline filter"
Sleep 500ms
Enter
Sleep 3500ms

# Beat 3 — graph: why() hops topic → PR → author and lands on the person.
Type "./target/debug/examples/wow_offline why"
Sleep 500ms
Enter
Sleep 5200ms
