# Tests run single-threaded

Status: accepted

The suites run with `--test-threads=1`.

**Why.** The tests share filesystem state — they open, write and reopen stores
under temporary directories that the same fixtures reuse. Run in parallel they
interleave on that state and fail for reasons unrelated to the code under test,
which is the most expensive kind of red: one that teaches nothing.

**Evidence.** The workspace test job in `.github/workflows/ci.yml` passes
`-- --test-threads=1` and sets `RUST_TEST_THREADS=1`.
