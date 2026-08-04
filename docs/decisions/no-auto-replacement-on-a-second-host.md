# Tool results are replaced on one host only

Status: accepted

The optimizer replaces an oversized tool result on the host whose hook protocol
defines an output-replacement channel. On the other supported host it observes
and records, but never substitutes a result.

**Why.** Replacement is only safe when the protocol says what a replacement
must look like and what happens when the shape is wrong. Without that contract,
a substituted payload either is silently dropped or reaches the model in a shape
the host never promised — and neither failure is visible from inside the hook.
Parity here would be a claim, not a capability.

**Evidence.** [PR #1810](https://github.com/cyberlife-coder/VelesDB/pull/1810);
the host-by-host wiring is in `integrations/agent-hooks/`.
