# @wiscale/tauri-plugin-velesdb

Typed frontend wrapper for [`tauri-plugin-velesdb`](https://crates.io/crates/tauri-plugin-velesdb),
the Tauri 2 plugin that embeds VelesDB (vector search, knowledge graph and
agent memory) in a desktop app. Each function calls one plugin command through
`invoke` and returns its typed result.

The wrapper is optional: the plugin's commands can be called with
`invoke('plugin:velesdb|<command>', { request })` and no extra dependency.

## Install

The Rust plugin must be registered and its commands granted first; see the
[plugin README](https://github.com/cyberlife-coder/VelesDB/tree/main/crates/tauri-plugin-velesdb#readme)
(`tauri_plugin_velesdb::init()` and a capability with `velesdb:default`).

```bash
npm install @wiscale/tauri-plugin-velesdb
```

Use the same version as the `tauri-plugin-velesdb` crate.

## Usage

```typescript
import { createCollection, upsert, search } from '@wiscale/tauri-plugin-velesdb';

await createCollection({ name: 'demo', dimension: 4, metric: 'cosine' });

await upsert({
  collection: 'demo',
  points: [
    { id: 1, vector: [1, 0, 0, 0], payload: { title: 'north' } },
    { id: 2, vector: [0, 1, 0, 0], payload: { title: 'east' } },
  ],
});

const { results } = await search({ collection: 'demo', vector: [1, 0, 0, 0], topK: 2 });
console.log(results[0].payload); // { title: 'north' }
```

A rejected command rejects with a `CommandError`: core's `VELES-###` code, a
message and a `recoverable` flag (see the
[plugin reference](https://github.com/cyberlife-coder/VelesDB/blob/main/docs/guides/TAURI_PLUGIN_REFERENCE.md)).

## API

The exported functions and request types are declared in
[`index.ts`](https://github.com/cyberlife-coder/VelesDB/blob/main/crates/tauri-plugin-velesdb/guest-js/index.ts),
one per plugin command. The
[plugin reference](https://github.com/cyberlife-coder/VelesDB/blob/main/docs/guides/TAURI_PLUGIN_REFERENCE.md)
documents every command.

## License

VelesDB Core License 1.0, see `LICENSE`.
