# vhs script for the "the memory builds its own graph" demo gif.
#   vhs examples/agent_memory/memory_builds_its_own_graph.tape
# Real run: a local LLM (qwen3.6) extracts the facts + auto-wires the graph, then
# why() walks it. Requires `ollama serve` with qwen3.6:27b-mlx and all-minilm, and
# velesdb built with the extract feature. Extraction takes ~50s — that wait is the
# model genuinely building the graph, on your machine.

Output examples/agent_memory/memory_builds_its_own_graph.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 900
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms

Hide
Type "source /tmp/velesp2-venv/bin/activate 2>/dev/null || true; export VIRTUAL_ENV_DISABLE_PROMPT=1; export PS1='$ '; clear"
Enter
Sleep 1s
Show

Type "python examples/agent_memory/memory_builds_its_own_graph.py"
Sleep 900ms
Enter
Sleep 70s
