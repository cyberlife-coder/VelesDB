# production-config — the way you would actually ship it

The quickstart's `init()` opens `./velesdb_data`, relative to the **process's
working directory**. That path moves depending on how the app was launched, is
not writable inside an installed `.app` bundle or `Program Files`, and is not
per-user. Fine for a smoke test, wrong for a release.

This example fixes both halves: where the data lives, and how the engine is
tuned.

```text
src-tauri/src/main.rs   <- init_with_app_data + Builder + with_config_path
velesdb.toml            <- engine configuration, next to the binary or bundled
```

## The three entry points

| Entry point | Data directory | Use it when |
|---|---|---|
| `init()` | `./velesdb_data`, relative to the working directory | prototyping |
| `init_with_path("./my_data")` | whatever you pass | you own the layout |
| `init_with_app_data("MyApp")?` | `%APPDATA%\MyApp\velesdb\` · `~/Library/Application Support/MyApp/velesdb/` · `~/.local/share/MyApp/velesdb/` | **production** |

`init_with_app_data` returns a `Result`: resolving the platform directory can
fail, and the plugin surfaces that instead of guessing.

## Tuning the engine

`Builder::new(path).with_config_path("./velesdb.toml")` reads a TOML file and
**fails fast** — a missing, unparsable or out-of-range file makes the call
return an error rather than silently falling back to defaults. That is the
whole point: a typo in a config file should stop the app, not quietly halve
your recall.

Only the engine sections are read: `[search]`, `[hnsw]`, `[storage]`,
`[limits]`, `[quantization]`, `[wal_batch]`. A `[server]` table belonging to
another VelesDB component in a shared file is ignored, not rejected.

## Cargo features

`default` pulls in `velesdb-core/default`, which includes `persistence` — mmap
storage, WAL, and the streaming-insert commands. The other features (`gpu`,
`openapi`, `update-check`, `loom`, `internal-bench`, `bench-sift1m`,
`test-fault-injection`) forward to the matching `velesdb-core` feature.

**Never enable `internal-bench`, `bench-sift1m` or `test-fault-injection` in a
shipping bundle.**

## Copy it in

```bash
cp examples/production-config/src-tauri/src/main.rs src-tauri/src/main.rs
cp examples/production-config/velesdb.toml          src-tauri/velesdb.toml
```

You still need the capability file from the quickstart — it is what unlocks the
commands:

```bash
mkdir -p src-tauri/capabilities
cp examples/quickstart/src-tauri/capabilities/velesdb.json src-tauri/capabilities/velesdb.json
```

## Where to put velesdb.toml in a real build

A relative `"./velesdb.toml"` has the same working-directory problem as
`./velesdb_data`. Two robust options:

- **bundle it as a resource** and resolve the path at runtime through Tauri's
  path API, then pass the resolved absolute path to `with_config_path`;
- **skip the file entirely** and keep the defaults, which are what
  `init_with_app_data` alone gives you. Most apps never need to tune the
  engine.

`main.rs` shows the first shape, with the fallback spelled out.

## Verify where the data actually landed

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\VelesDBExample\velesdb\` |
| macOS | `~/Library/Application Support/VelesDBExample/velesdb/` |
| Linux | `~/.local/share/VelesDBExample/velesdb/` |

The directory is created on first write. If it stays empty, the app is still
running on the `init()` default somewhere else — check that the `.plugin(...)`
line you edited is the one actually compiled.
