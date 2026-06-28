# vhs script for the why()-across-sessions demo gif.
#   brew install vhs && vhs examples/agent_memory/why_across_sessions.tape
# Regenerates examples/agent_memory/why_across_sessions.gif (real, deterministic run).

Output examples/agent_memory/why_across_sessions.gif

Set Shell "bash"
Set FontSize 19
Set Width 1180
Set Height 760
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms

# Hidden setup: a clean prompt, and (best-effort) a velesdb venv so the visible
# command is just `python …`. Run vhs from the repo root; requires `velesdb`
# importable in the active environment (`pip install velesdb`).
Hide
Type "source /tmp/velesp2-venv/bin/activate 2>/dev/null || true; export VIRTUAL_ENV_DISABLE_PROMPT=1; export PS1='$ '; clear"
Enter
Sleep 1s
Show

Type "python examples/agent_memory/why_across_sessions.py"
Sleep 900ms
Enter
Sleep 6s
