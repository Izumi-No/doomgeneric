# doomgeneric
The purpose of doomgeneric is to make porting Doom easier.
Of course Doom is already portable but with doomgeneric it is possible with just a few functions.

To try it you will need a WAD file (game data). If you don't own the game, shareware version is freely available (doom1.wad).

# porting
Create a file named doomgeneric_yourplatform.c and just implement these functions to suit your platform.
* DG_Init
* DG_DrawFrame
* DG_SleepMs
* DG_GetTicksMs
* DG_GetKey

|Functions            |Description|
|---------------------|-----------|
|DG_Init              |Initialize your platfrom (create window, framebuffer, etc...).
|DG_DrawFrame         |Frame is ready in DG_ScreenBuffer. Copy it to your platform's screen.
|DG_SleepMs           |Sleep in milliseconds.
|DG_GetTicksMs        |The ticks passed since launch in milliseconds.
|DG_GetKey            |Provide keyboard events.
|DG_SetWindowTitle    |Not required. This is for setting the window title as Doom sets this from WAD file.

### main loop
At start, call doomgeneric_Create().

In a loop, call doomgeneric_Tick().

In simplest form:
```
int main(int argc, char **argv)
{
    doomgeneric_Create(argc, argv);

    while (1)
    {
        doomgeneric_Tick();
    }
    
    return 0;
}
```

# sound
Sound is much harder to implement! If you need sound, take a look at SDL port. It fully supports sound and music! Where to start? Define FEATURE_SOUND, assign DG_sound_module and DG_music_module.

# platforms
Ported platforms include Windows, X11, SDL, emscripten. Just look at (doomgeneric_win.c, doomgeneric_xlib.c, doomgeneric_sdl.c).
Makefiles provided for each platform.

## emscripten
You can try it directly here:
https://ozkl.github.io/doomgeneric/

emscripten port is based on SDL port, so it supports sound and music! For music, timidity backend is used.

## Windows
![Windows](screenshots/windows.png)

## X11 - Ubuntu
![Ubuntu](screenshots/ubuntu.png)

## X11 - FreeBSD
![FreeBSD](screenshots/freebsd.png)

## SDL
![SDL](screenshots/sdl.png)

## rust bootstrap (no_std + musl)

The repository contains a Rust workspace for an incremental, libc-free port
of the Doom engine core. It is organised so that every host concern (IO) is a
seam that can be stubbed, replaced or dropped per environment:

```text
rust/
  doomgeneric-core/           # #![no_std], depends only on core+alloc (no libc)
    src/fixed.rs              # m_fixed  -> FixedMul / FixedDiv (16.16)
    src/random.rs             # m_random -> deterministic LUT RNG streams
    src/platform.rs           # host seams: Renderer, Input, Audio, Clock,
                              #              FileSystem, Args, PlatformIo
  doomgeneric-host-headless/  # reference std host; a playable headless driver
```

Design rules:

- `doomgeneric-core` is `#![no_std]` and `#![forbid(unsafe_code)]`: it can never
  touch libc or FFI. It is validated against `wasm32-unknown-unknown` (a target
  with no libc) and against the `x86_64-unknown-linux-musl` target.
- All IO goes through the traits in `platform.rs`, never through the core.
  The final binary is the only place where `std` and a libc may appear; for
  Linux it links statically against musl.
- Sound is a request queue (`s_sound.rs`): the core attenuates and enqueues,
  the host drains and mixes through its `Audio` seam.
- The renderer produces indexed-color screens (`v_video.rs` presents them
  through the `PLAYPAL` palette into `Frame` pixels); the host never
  interprets WAD data itself.

Quick checks:

```bash
rustup target add x86_64-unknown-linux-musl wasm32-unknown-unknown

# core, no libc, no std
cargo check -p doomgeneric-core --target wasm32-unknown-unknown

# core + tests on musl
cargo test    -p doomgeneric-core --target x86_64-unknown-linux-musl

# playable headless driver against a retail IWAD (DOOM2.WAD / Ultimate DOOM)
cargo run     -p doomgeneric-host-headless
cargo run     -p doomgeneric-host-headless -- "DOOM (2).WAD" -warp1,1

# statically linked musl binary
cargo build   -p doomgeneric-host-headless --target x86_64-unknown-linux-musl --release
ldd target/x86_64-unknown-linux-musl/release/doomgeneric-host-headless  # -> "statically linked"
```

Porting order follows dependency from the leaves up: fixed point and RNG first,
then the WAD loader (over `FileSystem`), then the play simulation, and finally
the software renderer writing into a `Frame` for the host to present.

## TODO / porting status

Each phase compiles and passes tests on `x86_64-unknown-linux-musl` and is
checked against `wasm32-unknown-unknown` (no libc) before moving on.

### Fase 1 — data layer
- [x] `doomdef.rs` — enums/constants (`TICRATE`, `ML_*`, linedef flags, …)
- [x] `fixed.rs` — `FixedMul` / `FixedDiv` (16.16)
- [x] `random.rs` — deterministic LUT RNG streams
- [x] `tables.rs` + `build.rs` — `finesine`, `finetangent`, `tantoangle`, `gammatable`
- [x] `wad.rs` — WAD parser + `WadManager` (`W_AddFile` merge semantics)
- [x] `level.rs` — map structs + `P_SetupLevel` loaders (vertexes, sectors,
      sides, linedefs, subsectors, nodes, segs, blockmap, reject, things),
      `P_GroupLines`, `PadRejectArray`
- [x] `info.rs` — mobjinfo / states / weapons (port of `info.c` ~4700 lines)
- [x] `sounds.rs` — sound/music LUTs
### Fase 2 — world setup + software renderer
- [x] `p_setup` world data loading (`Level::load`): vertexes, sectors, sides,
      lines, subsectors, nodes, segs, blockmap, reject, things
- [x] renderer core: `r_main`, `r_bsp`, `r_segs`, `r_draw`, `r_plane`,
      `r_sky`, `r_setup`
- [x] `r_data` — flats/textures/sprites lookup, colormaps, composite columns
      (`RenderData`, WAD synthetic tests)
- [x] `p_setup` runtime init: `P_SpawnMapThing`, `P_SpawnSpecials` (structural:
      secret counters, scroll lines, timer; light/door sector specials are
      Fase 3), `P_InitThinkers`, `P_LoadThings` placement
- [x] `r_things` — sprite defs (`R_InitSpriteDefs`/`R_InstallSpriteLump`),
      `R_AddSprites`/`R_ProjectSprite`/`R_DrawSprite`/`R_DrawMasked`
      (psprites deferred with the player state)

### Fase 3 — play simulation + game loop
- [x] `p_tick` — thinker arena (`P_AddThinker`/`P_RemoveThinker`/`P_Ticker`),
      plus `World::run_thinkers` dispatch (mobj/door/ceiling/floor/plat/light)
- [x] `p_mobj` — mobj info/state layer, sector+blockmap thing linking
      (`P_SetThingPosition`/`P_UnsetThingPosition`), `P_SpawnMobj`,
      `P_MobjThinker`, `P_XYMovement`/`P_ZMovement`, `P_SlideMove`,
      `P_ExplodeMissile` (`World::explode_missile`), `P_RadiusAttack`,
      `P_SpawnMissile`/`P_CheckMissileSpawn`, `P_SpawnPlayerMissile`,
      nightmare respawn (`P_NightmareRespawn`)
- [x] `p_map` — `P_CheckPosition`/`P_TryMove`/`P_ThingHeightClip`/
      `P_ChangeSector` (crunch: gibs, dropped items, damage+blood),
      `P_AimLineAttack` (com `linetarget`), `P_LineAttack` completo
      (puff/sangue, `P_ShootSpecialLine`, céu), `P_UseLines`
- [x] `p_maputl` — divlines, `P_LineOpening`, `P_BlockLinesIterator`
- [x] `p_inter` — `P_Give*`, `P_TouchSpecialThing`, `P_KillMobj`
      (tallies `MF_COUNTKILL`), `P_DamageMobj`
- [x] `p_enemy` — AI: `A_Look`/`A_Chase`/`A_FaceTarget`, melee/missile attacks,
      `A_Tracer`, death/pain actions, `P_NoiseAlert`/`P_RecursiveSound`,
      e ações DOOM II (`A_Vile*`, `A_Fat*`, `A_PainAttack/Die`, `A_BossDeath`,
      `A_KeenDie`, `A_Brain*`, `A_SpawnFly`, `A_PlayerScream`)
- [x] sector actions: `p_doors` (`T_VerticalDoor`, `EV_DoDoor`,
      `EV_DoLockedDoor`/`EV_VerticalDoor`, `P_SpawnDoorCloseIn30`/
      `P_SpawnDoorRaiseIn5Mins`), `p_floor` (`T_MovePlane`, `T_MoveFloor`,
      `EV_DoFloor`, `EV_BuildStairs`), `p_ceilng` (`T_MoveCeiling`,
      `EV_DoCeiling`, `EV_CeilingCrushStop`, in-stasis), `p_plats`
      (`T_PlatRaise`, `EV_DoPlat`, `EV_StopPlat`, `P_ActivateInStasis`) —
      sobre as arenas `Specials` em `p_spec`
- [x] `p_pspr` — `P_SetPsprite`/`P_MovePsprites`/`P_SetupPsprites`,
      `A_WeaponReady`/`A_Lower`/`A_Raise`/`A_ReFire`/`A_GunFlash`,
      todos os `A_Fire*` (pistol/shotgun/SSG/chaingun/missile/plasma/BFG,
      chainsaw/punch), `A_Light*`, `A_BFGSpray`, `P_CheckAmmo`/`P_FireWeapon`
- [x] `p_user` — `P_PlayerThink`/`P_MovePlayer`/`P_Thrust`/`P_CalcHeight`/
      `P_DeathThink`, mudança de arma, power-ups/colormaps
- [x] `p_switch` — `alphSwitchList` (por episódio), `P_ChangeSwitchTexture`,
      botões (`P_StartButton`/timers em `P_UpdateSpecials`)
- [x] `p_lights` — `T_FireFlicker`/`T_LightFlash`/`T_StrobeFlash`/`T_Glow`,
      spawns dos sector specials, `EV_LightTurnOn`/`EV_TurnTagLightsOff`/
      `EV_StartLightStrobing`
- [x] `p_telept` — `EV_Teleport`/`P_TeleportMove`
- [x] `p_spec` ativação de linhas — `P_CrossSpecialLine` (triggers +
      retrigger 72..129), `P_ShootSpecialLine`, `P_UseSpecialLine`
      (manuals/switches/buttons/locks), `EV_DoDonut`,
      `P_PlayerInSpecialSector`, `P_UpdateSpecials` (scrolls, timer, botões)
- [x] `g_game` — `G_Ticker`/`G_DoLoadLevel` (`P_SetupLevel` glue)/
      `G_InitNew`/`G_DoReborn`/`G_PlayerReborn`/`G_CheckSpot`/`P_SpawnPlayer`/
      `G_DeathMatchSpawnPlayer`
- [x] `d_items` (tabela em `info::WEAPONINFO`), `d_main` (modo de jogo,
      resolução ExMy/MAPxx), `d_net` (paridade single-player, FIFO de ticcmds)
- [x] `d_ticcmd`, `d_player` (psprites, `pendingweapon`/`message`)

### Fase 4 — sound, status, intermission, menu, playable hosts
- [x] `s_sound` — fila de requests (`S_StartSound` com atenuação posicional
      `S_AdjustSoundParams`, música por nível `S_LevelMusic`), drenada pelo
      host via seam `Audio`; volumes via `m_config`
- [x] `st_stuff` — status bar completa: widgets (ammo/health/armor/arms/
      keys/frags), animação de rosto (`ST_FaceCalc` com todas as prioridades
      vanilla), desenho via patches reais (`STBAR`/`STTNUM`/`STF*`)
- [x] `wi_stuff` — máquina de estados do intermission: contadores com
      aceleração (`accelstage`), par times (`pars`/`cpars`), `WI_Ticker`
- [x] `hu_stuff` — fila de mensagens por jogador (`HU_Ticker`)
- [x] `m_menu` — árvore de menus como dados; input emite `MenuAction`s
      (start game/volumes/quit) sem IO nem telas
- [x] `m_misc`/`m_config` — helpers de texto; config key/value com
      parse/serialize (o host persiste via seam `FileSystem`)
- [x] game flow (`g_game`) — `gamestate_t` level↔intermission,
      `G_DoCompleted` (próximo mapa, vitória E?M8, saídas secretas
      DOOM/DOOM II), `G_WorldDone`/finale maps, `G_RespawnSpecials`
      (respawn de itens em deathmatch 2), fila `itemrespawnque`
- [x] `v_video` — primitivas de vídeo: patches (`V_DrawPatch` com clipping),
      paleta `PLAYPAL`, apresentação indexed→`Frame` (pixels ARGB)
- [x] `r_frame` — fachada `R_RenderPlayerView`: setup do view + BSP walls +
      visplanes (com os pads `pad1`/`pad2` do vanilla) + sprites mascarados
- [x] gameplay fixes: pickups e `P_CrossSpecialLine` ligados ao movimento
      (`post_move_hooks`, o `spechit` do `P_TryMove`), `P_TeleportMove`,
      segs com `side` fora de 0/1 mascarado (WADs re-construídos)
- [x] host jogável (`doomgeneric-host-headless`): identifica o IWAD
      (`d_main`), carrega DOOM2.WAD/Ultimate DOOM, roda `G_Ticker` com
      ticcmds scriptados, renderiza 3D + status bar com paleta real, drena
      a fila de sons e escreve PPMs por segundo

### Fase 5 — host/UI extras
- [x] sprites de arma em primeira pessoa — `R_DrawPSprite` no `r_things`
      (`draw_player_sprites`: escala/flip/light row do `R_SetupFrame`,
      invisibilidade→fuzz, fullbright), ligado à fachada `r_frame`
- [x] finale/text screens — `f_finale`: stages text/art/cast com a tabela
      `textscreens` (E1-E4/DOOM II), digitação `TEXTSPEED`/`TEXTWAIT`,
      `F_CastTicker` com a tabela de sons, desenho via flats/`STCFN`/
      sprites do WAD; tela de título no host (`TITLEPIC`)
- [x] demos — `m_demo`: header vanilla 1.9 (109) + **longtics** (221,
     Chocolate): `-record demo.lmp` / `-playdemo demo.lmp` no host regravam
      bit-a-bit (playback exato validado end-to-end no MAP01)
- [x] savegames — `g_save`: snapshot binário do estado dinâmico completo
      (players, mobjs, thinkers, specials, luzes, setores/linhas/sides
      alterados, filas, RNG) com restore em lockstep testado
- [x] multiplayer — seam `Network` (`platform.rs`) + `d_net::NetSession`
      (um tic roda quando todos os comandos chegaram; slot local conta
      na hora) + `LoopbackNet` testável; `Rc<RefCell>` sem `std`
- [x] input modular — `d_loop`: fila de eventos (`D_PostEvent`/
      `D_ProcessEvents`), `InputState` (teclas/mouse/joystick, sensibilidade
      vanilla), `G_BuildTiccmd` com as tabelas `angleturn`/`forwardmove`/
      `sidemove`, turn acelerado (`SLOWTURNTICS`), strafe, armas 1-7;
      seams `poll_mouse`/`poll_joystick` no trait `Input`
- [x] host jogável: cmds construídos via `G_BuildTiccmd` (o mesmo caminho de
      um host com janela), demo record/playback, finale e título renderizados

### Fora de escopo (host de plataforma)
- [ ] janela real (SDL/X11/wayland): os seams existem (`Renderer`, `Input`
      com mouse/joystick, `Audio`, `Network`); falta o crate de host com
      display, impossível de validar neste ambiente headless

Legend: `[x]` done · `[ ]` pending
