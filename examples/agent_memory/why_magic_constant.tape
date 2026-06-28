# vhs script for the "why is this magic constant here?" demo gif.
#   brew install vhs && vhs examples/agent_memory/why_magic_constant.tape
# Regenerates examples/agent_memory/why_magic_constant.gif (real, deterministic,
# offline run — the default 'hash' embedder, no model, no network).

Output examples/agent_memory/why_magic_constant.gif

Set Shell "bash"
Set FontSize 19
Set Width 1180
Set Height 820
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms

Hide
Type "source /tmp/velesp2-venv/bin/activate 2>/dev/null || true; export VIRTUAL_ENV_DISABLE_PROMPT=1; export PS1='$ '; clear"
Enter
Sleep 1s
Show

Type "python examples/agent_memory/why_magic_constant.py"
Sleep 900ms
Enter
Sleep 6s
