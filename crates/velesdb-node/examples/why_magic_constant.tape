# vhs script for the Node "magic constant" demo gif.
#   (from crates/velesdb-node, after `npm install && npm run build`)
#   vhs examples/why_magic_constant.tape
# Real, offline run of the @wiscale/velesdb-memory-node binding (hash embedder).

Output examples/why_magic_constant.gif

Set Shell "bash"
Set FontSize 19
Set Width 1180
Set Height 760
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms

Hide
Type "export PS1='$ '; clear"
Enter
Sleep 500ms
Show

Type "node examples/why_magic_constant.mjs"
Sleep 900ms
Enter
Sleep 5s
