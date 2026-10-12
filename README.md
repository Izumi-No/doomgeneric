# doomgeneric (Rust port)

A from-scratch, dependency-light port of Doom to Rust, organised as a
`no_std` engine core plus thin hosts. The goal is a faithful, embeddable
engine: every host concern (rendering, input, audio, timing, filesystem,
networking) is a trait seam, so the same core drives a headless PPM dump, a
windowed desktop game, or a microcontroller.

The C source tree this was ported from has been removed; the repository is
Rust-only.

## Workspace

```text
doomgeneric-core/           # #![no_std], #![forbid(unsafe_code)], core+alloc only
  src/fixed.rs              # m_fixed   -> FixedMul / FixedDiv (16.16)
  src/random.rs             # m_random  -> deterministic LUT RNG streams
  src/tables.rs             # tables.c  -> exact historical fine-sine / tangent /
                            #              tantoangle/gammatable values
  src/wad.rs                # w_wad     -> WAD parser + WadManager merge
  src/level.rs, p_*.rs      # map loading and the play simulation
  src/r_*.rs, v_video.rs    # BSP software renderer and video primitives
  src/platform.rs           # host seams: Renderer, Input, Audio, Clock,
                            #              FileSystem, Args, Network, PlatformIo
doomgeneric-host-headless/  # reference host: scripted demo, writes PPM frames
doomgeneric-host-wgpu/      # playable windowed host: winit window + wgpu
```

Design rules:

- `doomgeneric-core` is `#![no_std]` and `#![forbid(unsafe_code)]`; it never
  touches libc or FFI. It is checked against `wasm32-unknown-unknown` and
  `x86_64-unknown-linux-musl`.
- All IO goes through `platform.rs`; only the host binaries use `std`.
- Sound is a request queue (`s_sound.rs`): the core attenuates and enqueues,
  the host drains and mixes through its `Audio` seam.
- The renderer writes indexed-color screens; the host presents them through
  the `PLAYPAL` palette into `Frame` (0xAARRGGBB) pixels.

## Build and run

```bash
# unit tests for the engine core
cargo test -p doomgeneric-core

# headless host: scripted demo, dumps one PPM per second
cargo run -p doomgeneric-host-headless -- DOOM2.WAD -warp 1

# playable windowed host (winit + wgpu)
cargo run -p doomgeneric-host-wgpu -- DOOM2.WAD -warp 1
```

The headless host also supports `-record FILE` / `-playdemo FILE` (vanilla
1.9 and Chocolate longtics) and `-finale`.

### WADs

Any IWAD works: `DOOM2.WAD` / `DOOM (2).WAD` (commercial), `doom1.wad`
(shareware), or `DOOM.WAD` (registered). Pass the path explicitly or leave it
out and the host probes the common names in the working directory. PWADs are
not merged from the command line yet; the `WadManager` seam exists.

### wgpu host controls

| Key | Action |
|-----|--------|
| Arrows / W A S D | move / strafe |
| Mouse | turn (raw device motion) |
| Shift | run |
| Ctrl / left mouse | fire |
| Space / E / middle mouse | open / use |
| 1-7 | select weapon |
| Esc | quit |

## Testing and fidelity

The engine core has 170 unit tests. Bit-exactness matters for demo
compatibility, so the trigonometric and gamma tables are the historical Doom
values embedded verbatim in `doomgeneric-core/src/tables_data.rs`
(`tables_match_the_original_c_source` guards a few entries). RNG streams are
checked against the original LUT.

There is no automated end-to-end demo test yet: the recommended next step is
to run a stock `.lmp` and compare a framebuffer/state hash against a known
reference.

## Porting status

Implemented: data layer (fixed point, RNG, WAD, map loaders, `info`), world
setup, BSP software renderer (walls, planes, sky, masked sprites, PSprite),
the full play simulation (`p_*`), sector actions, weapons, player, status bar,
intermission, menus (as data), HUD, game flow, demos, savegames, sound
requests, and single-player plus a loopback net seam.

Not yet ported (present in upstream doomgeneric/Chocolate Doom):

- automap (`am_map`)
- screen wipe transitions (`f_wipe`)
- cheat codes (`m_cheat`)
- DeHackEd patch loading (`deh_*`)
- multi-WAD command-line merge (`w_main`) and checksums (`w_checksum`/`sha1`)
- music conversion (`mus2mid`)
- `statdump`
- a real audio backend and socket-backed netgame (the `Audio`/`Network`
  seams are ready; only the seams and loopback transport exist)

## License

The engine descends from the Doom source release and is distributed under the
GNU GPL v2; see `LICENSE`.
