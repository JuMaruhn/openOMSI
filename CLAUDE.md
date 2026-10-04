# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

openOMSI is a from-scratch Rust recreation of the bus simulator **OMSI 2** (Delphi/Direct3D 9,
Windows 32-bit). It ships **no game content**: it loads the maps, vehicles, scenery, scripts and
timetables of an installed OMSI 2 and must behave like `Omsi.exe` **2.2.032** while doing it.

Two rules govern almost every decision in this repository:

1. **1:1 behaviour, not "better" behaviour.** When openOMSI and the original differ, the original
   is right — even when it is a bug (a NULL texture sampling as alpha 1, a volume clamp that keeps
   the last value, a mask scaled the wrong way). Behaviour is reverse-engineered from `Omsi.exe`,
   and commits/comments cite the original unit (`mc_sound`, `mc_exprcalc`) or the address
   (`0x750340`). Deliberate improvements only ever go where the original is *silent*, and they are
   documented in `docs/MODDING.md` (extra interior lights, bigger textures) — a file using them must
   still load in OMSI 2.
2. **No original code or assets in the repo.** Never copy a texture, mesh, sound, map or script out
   of an OMSI 2 installation. Tests that need content read it from `$OMSI_ROOT` and skip themselves
   when there is none.

## Build, run, test

```sh
cargo build --release -p omsi-app        # → target/release/openomsi (plain cargo works fine)
scripts/build-linux.sh                   # → dist/linux/  (also build-macos.sh, build-windows.cmd,
                                         #    build-android.sh, build-server.sh, build-plugin-host.sh)
cargo test --workspace                   # must pass before any PR (CONTRIBUTING.md)
cargo test -p omsi-app --lib <name>      # one test; tests live inline, not in tests/
cargo run --release -p omsi-check -- "/path/to/OMSI 2"   # load every content file, report failures
```

Requires Rust 1.85+. `dist/<platform>/` is also the game's **content folder** (mods live beside the
binary), so build scripts replace only binaries and never clear it.

**Tests are `#[cfg(test)] mod tests` inside `src/*.rs`** (only `omsi-plugin` has a `tests/` dir).
Many need a real installation: `OMSI_ROOT="/path/to/OMSI 2" cargo test …`; without it they print
`skipped:` or are `#[ignore]`d. LAN tests bind real UDP ports — a new one must pick a port no other
test uses, or the whole-suite run flakes.

CI (`.github/workflows/release.yml`) builds all platforms per push and publishes a release, but runs
only a few guard tests (`-p omsi-texture --lib`, the scripted-lamp test, and omsi-render's
`shaders_validate_and_match_the_uniforms`). Run the workspace suite locally; CI will not catch it.

### Verifying a change in the real game

The binary is scriptable, which is how graphics/sim changes are checked without a human at the keys:

```sh
openomsi --root "/path/to/OMSI 2" --map maps/Grundorf/global.cfg \
         --bus Vehicles/MAN_SD200/MAN_SD80.bus --offscreen /tmp/shot.png --cam x,y,z,yaw,pitch \
         --drive 8 --snapshots 3,6 --autostart        # render frames to PNG and exit
OMSI_PROFILE=1 openomsi … --exit-after 60             # frame split, >50 ms frames, GPU/CPU memory
OMSI_INPUT="t=3 move 400,300; t=3.2 press; t=4 key F3" openomsi …   # drive the real window handlers
```

Dozens of `OMSI_*` switches isolate subsystems (`OMSI_DEBUG_TRAFFIC`, `OMSI_DEBUG_PAX`,
`OMSI_NO_SHADOWS`, `OMSI_TEXTURE_MEMORY`, `OMSI_BACKEND=vulkan|dx12|gl`, …). The useful ones are
tabulated in `docs/USER_GUIDE.md` ("Debug and test switches"); the rest: `grep -r OMSI_ crates`.
Read env vars through `omsi_cfg::env::var_os` (process-wide cached) rather than `std::env`, because
some are read per frame. Logs: `~/.openomsi/game.log`, `launcher.log`.

## Architecture

`docs/ARCHITECTURE.md` is the authority (crate ↔ original Delphi unit table, threading, memory,
coordinate frames, roadmap); `docs/FORMATS.md` documents every content format. Read the relevant
section before touching a loader or the renderer — the hard-won details are there, not in the code.

The layering, roughly bottom-up:

- **`omsi-cfg`** — OMSI's text-file grammar (keyword line `[mesh]` + N fixed parameter lines),
  code-page detection, and the **content-root overlay + VFS**: several roots are searched in order
  (LAN session content → mods/content folder → the OMSI 2 installation), lookups are
  case-insensitive like Windows, and a `.zip` is *mounted* as a folder so a 15 GB map pack needs no
  unpacking. **The original installation is never written to**; the game writes copies into the
  content folder instead (the object editor does exactly this). Go through `resolve_path`,
  `find_in_roots`, `content_dirs`, `read_dir_merged` — never bare `std::fs` on content paths.
- **Format crates** — `omsi-script` (`.osc` compiler + VM), `omsi-o3d` (`.o3d`/`.x` meshes),
  `omsi-model` (`model.cfg`/materials), `omsi-scenery` (`.sco`/`.sli`), `omsi-map` (tiles are
  300 m × 300 m, 61×61 terrain samples), `omsi-vehicle`, `omsi-timetable`, `omsi-content` (weather,
  fonts, money, humans, situations), `omsi-texture` (OMSI's name-based search order, seasonal and
  `_LOW` variants, BC1/BC3 compression), `omsi-geometry` (spline/terrain tessellation).
- **`omsi-render`** — the wgpu renderer (Metal/Vulkan/DX12/GL). Two renderers share it: the
  *vanilla* path reproduces Direct3D 9 draw order and material state, and `--enhanced` is a separate
  physically based path. Shaders are WGSL in `src/*.wgsl`; the test
  `shaders_validate_and_match_the_uniforms` parses them with naga **and asserts every WGSL struct
  matches the size of its Rust uniform** — if you add a uniform field, update both sides and that
  table. The `Scene` is slot-based: per-draw data is rewritten only for instances that changed, so
  mutating scene state has to go through the change-tracking API or the frame cost returns.
- **`omsi-sim`** — runtime: script hosts (`Program` compiled per object *type*, `State` per
  *instance*, executed against a `Host` that supplies system variables/triggers), animations,
  AI traffic (`traffic`, `ai_motion`), people (`human`, `crowd`), physics (`rigid`, `physics`),
  IBIS, script/text/HTML textures.
- **`omsi-app`** (~100k lines, the game binary `openomsi`) — main loop, window and offscreen modes,
  tile streaming, HUD, schedule runtime, the launcher window, LAN glue, dedicated server, VR.
  It uses a **crate-wide glob prelude**: `lib.rs` declares the modules and re-exports them
  (`use app::*; use world_load::*; …`), and each module starts with `use super::*;`. Adding a module
  means adding both lines; a new shared type is visible everywhere once it is `pub(crate)`.
- **`omsi-ui`** (text/icons/atlas/paint + wgpu pipeline) and **`omsi-launcher-core`** (launcher data
  side as plain functions, plus the `openomsi-launcher --cli` terminal surface; the launcher *window*
  lives in `omsi-app::launcher`). **`omsi-net`** (UDP LAN star topology, host owns the world),
  **`omsi-plugin`** (OMSI `.opl` DLLs via a 32-bit host process, plus Lua plugins),
  **`omsi-audio`** (cpal mixer, volume curves, 3D).

Threading to respect: tile parsing/tessellation and texture decode+compression run on the rayon
pool, tile streaming has its own thread, AI scripts and people are `par_iter_mut`, and **wgpu device
calls are made from worker threads** (vehicle sets are uploaded off-thread and only handed into the
scene on the main thread). Memory is budgeted deliberately (texture budget with mip demotion, fleet
read-ahead and trimming, slot reuse) — see ARCHITECTURE.md *Memory* before "simplifying" any of it.

**Coordinate frames are a frequent source of bugs** and differ per file kind: world (x east, y north,
z up), `.cfg` (x right, y forward, z up), `.o3d`/`.x` (Direct3D, converted on load), vehicle origin
at unloaded-suspension tyre contact. The exact rules, including angle units and sign conventions,
are in ARCHITECTURE.md *Coordinate frames*.

## Conventions

- **Commit subject: `Subsystem: what the player notices`**, lower-case after the colon, one change
  per PR, often with the original's reasoning in parentheses and the issue number:
  `Render: a [matl_transmap] whose file is missing keeps the slot opaque (Omsi.exe samples a NULL
  texture there, alpha 1), so traffic bodies … (#211)`. Subjects are long and explanatory on purpose.
  `CHANGELOG.md` is updated in its own commit, grouped by area, with issue links.
- **Comments explain *why*** — usually which `Omsi.exe` behaviour is being matched, with the unit or
  address. Comment density here is high and prose-like; match it rather than stripping it.
- `rustfmt` defaults. No clippy gate in CI.
- **UI strings** go in `crates/omsi-app/locales/app.yml` (`rust_i18n`), where **the English text is
  the key** and every language is listed under it; `omsi_ui::tr` / `t!` look them up. The file is
  `include_str!`-ed so edits force a recompile.
- **English first.** A string in a source file is always its English text, because that text is the
  lookup key. German and the other 24 languages only ever appear in `app.yml`, under that key, with
  every language filled in - never a translated literal in Rust.
- **Comments, doc comments and commit messages are English**, whatever language the conversation
  that led to them was held in.
- **Changes to the assistant's own files go in their own commit.** `CLAUDE.md`, `.claude/` and
  anything else that only steers an AI assistant are committed separately from program changes:
  mixed together they make a pull request hard to read.
- **Do not bump `VERSION`** (maintainers do). The release version is `MAJOR.MINOR.COMMIT` computed by
  `scripts/version.sh` from commit count; `docs/VERSIONING.md` has the scheme.
- Docs in `docs/*.md` are also the published website (`.github/workflows/pages.yml` renders them), so
  a user-visible change usually means an edit there too.
- Before larger changes, run `omsi-check` against a real installation and compare before/after: it is
  the compatibility regression net (`CONTRIBUTING.md`).
