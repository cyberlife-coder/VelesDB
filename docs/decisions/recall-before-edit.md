# A repository edit requires a successful causal recall first

Status: accepted

In an opted-in repository, the edit guard refuses a modification until a recall
has succeeded, bound to the exact host session and the exact physical checkout.
The sentinel is written only after a successful MCP result — never at the start
of the call, never after a timeout or an error.

**Why.** The recurring failure is not forgetting to recall; it is believing a
recall happened. Binding to the session and the checkout is what stops a
sentinel from one worktree authorising an edit in another, and stops a timeout
from reading as a success.

**Evidence.** [PR #1810](https://github.com/cyberlife-coder/VelesDB/pull/1810).
The identity pinning it relies on is itself guarded, and that guard was
corrected in [PR #1811](https://github.com/cyberlife-coder/VelesDB/pull/1811)
after it was found to pass only by an accident of directory capitalisation.
