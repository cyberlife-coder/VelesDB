# Decision note: native installers

**Status:** open — no decision made. This note lays out the trade-off; it does
not recommend one option over another.

## The open question

**Does the installation-friction audience include non-developers?**

This is not currently known, and it changes the answer completely:

- If the audience is developers and technical operators (the current README /
  `docs/guides/INSTALL_OPTIONS.md` audience), the existing paths — `pip
  install`, `cargo install`, `npm install`, Docker, the `curl | bash`
  one-liners, the `.mcpb` bundle — already cover it at low marginal cost.
- If the audience extends to non-developers (e.g. someone installing a
  desktop AI-agent tool who has never opened a terminal), none of the current
  paths are truly zero-friction: every one of them assumes comfort with a
  package manager or a shell command.

Nothing in this repository answers this question today. It should be asked
and answered by whoever owns the installation-friction goal before any of the
options below are built.

## The options

| Option | State | Cost | Notes |
|---|---|---|---|
| **`.mcpb` bundle** | Exists today (`.github/workflows/release-mcpb.yml`, built on every `velesdb-memory-vX.Y.Z` tag) | Marginal — already built and published per release | Covers `velesdb-memory` only, for MCP clients that support the bundle format. Does not touch the core server/CLI. |
| **Homebrew (`brew install`) / winget (`winget install`)** | Does not exist — no formula, cask, or manifest published anywhere | Low effort | No code-signing requirement from either package manager itself. Homebrew formulas can build from the existing release tarballs/source; winget manifests point at the existing `.zip`/`.msi`. Still requires maintaining a formula/manifest and (for Homebrew) either a tap or an upstream PR, plus keeping it in sync with each release. |
| **Signed `.pkg` (macOS)** | Does not exist | Recurring cost: Apple Developer Program membership ($99/year) + notarization step in the release pipeline | Removes the Gatekeeper "unidentified developer" warning on the portable `.tar.gz`/binaries. Only relevant if the audience is expected to double-click an installer rather than run a shell command. |
| **Signed `.msi` (Windows)** | Does not exist — `docs/guides/INSTALLATION.md` already documents this as "on the roadmap but not yet available," current Windows path is the portable `.zip` | Recurring cost: EV code-signing certificate. Without it, SmartScreen flags the installer as unrecognized, which is a worse first impression than the current portable `.zip` for a non-developer audience | Same audience question applies: a developer comfortable with `Expand-Archive` doesn't need this; a non-developer likely does. |
| **`.app` (Tauri GUI)** | Does not exist as a distributed artifact today. `demos/tauri-rag-app/` and `crates/tauri-plugin-velesdb/` exist in the repo, but as a demo app and a plugin, not a packaged, signed, released VelesDB GUI product | Largest cost of all options — this is a **product to build and maintain** (UI, its own release cadence, its own support surface, likely its own signing requirements on both macOS and Windows), not a one-time packaging task | Only makes sense if there is a standalone GUI product roadmap independent of this installation-friction question. Do not conflate "package the CLI/server nicely" with "build a GUI app." |

## Why this is being written down instead of decided

The five options above sit on a strictly increasing cost curve (`.mcpb` →
Homebrew/winget → signed `.pkg` → signed `.msi` → Tauri `.app`), and the right
stopping point on that curve depends entirely on the open question above. Two
of the options (Homebrew/winget) are cheap enough that they don't strictly
need the answer — see `docs/guides/INSTALL_OPTIONS.md`'s "Recommended paths"
section, which already names them as a future direction. The three that carry
real recurring cost (signed `.pkg`, signed `.msi`, Tauri `.app`) should wait
for the audience question to be answered explicitly rather than being
inferred from convenience.
