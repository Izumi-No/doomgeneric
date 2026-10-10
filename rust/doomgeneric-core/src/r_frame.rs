//! The per-frame renderer facade (`R_RenderPlayerView` from `r_main.c`).
//!
//! Vanilla assembles the frame from globals: setup the view, walk the BSP
//! (walls + visplanes + vissprites), draw the planes, then the masked
//! columns. The port bundles the per-level render state ([`FrameRenderer`])
//! so the host needs no globals at all — it passes the [`World`] and an
//! indexed screen, and gets a finished frame.

use crate::level::Level;
use crate::p_world::World;
use crate::r_bsp::SegManager;
use crate::r_data::{RDataError, RenderData};
use crate::r_plane::PlaneManager;
use crate::r_setup::TextureNumbers;
use crate::r_things::SpriteManager;
use crate::v_video::{SCREENWIDTH, SCREENHEIGHT, ST_HEIGHT};
use crate::wad::WadManager;

/// Everything `R_RenderPlayerView` needs that lives per level.
pub struct FrameRenderer {
    pub data: RenderData,
    pub tn: TextureNumbers,
    pub planes: PlaneManager,
    pub seg: SegManager,
    pub sprites: SpriteManager,
    /// The player whose eyes we render (`displayplayer`).
    pub displayplayer: usize,
}

impl FrameRenderer {
    /// Build the per-level state: texture numbers, sprite definitions and
    /// the wall/plane/sprite work areas.
    pub fn new(
        manager: &WadManager,
        level: &Level,
        sprite_defs: crate::r_things::SpriteDefs,
    ) -> Result<Self, RDataError> {
        let data = RenderData::init_data(manager)?;
        let tn = TextureNumbers::resolve(level, &data, manager)?;
        let mut planes = PlaneManager::new();
        planes.init_sky_map();
        // `skyflatnum` from the resolved numbers (vanilla sets it at level
        // setup; `PlaneManager::draw_planes` compares against this).
        let sprites = SpriteManager::new(sprite_defs, level.sectors.len().max(1));
        let skytexture = tn.skytexture;
        Ok(FrameRenderer {
            data,
            tn,
            // vanilla sets the level sky in P_SetupLevel/G_DoLoadLevel.
            planes: {
                let mut p = planes;
                p.skytexture = skytexture;
                p
            },
            seg: SegManager::new(),
            sprites,
            displayplayer: 0,
        })
    }

    /// `R_RenderPlayerView`: draw the 3D view of `self.displayplayer` into
    /// `screen` (indexed colors, `SCREENWIDTH * (SCREENHEIGHT - ST_HEIGHT)`
    /// bytes at most; the status bar owns the bottom 32 rows).
    pub fn render_view(&mut self, world: &mut World, manager: &WadManager, screen: &mut [u8]) {
        let render = &mut world.render;
        let viewheight = render.viewheight().max(1) as usize;

        // R_SetupFrame from the display player.
        let (x, y, z, angle, extralight, fixedcolormap) = {
            let p = &world.players[self.displayplayer];
            let Some(mo) = p.mo else {
                return;
            };
            let Some(m) = world.mobjs.get(mo) else {
                return;
            };
            (m.x, m.y, p.viewz, m.angle, p.extralight, p.fixedcolormap)
        };
        render.setup_frame(x, y, z, angle, extralight, fixedcolormap);

        // Clear the per-frame work areas.
        self.planes.clear_planes(render);
        self.seg.clear_clip_segs(render.viewwidth().max(1));
        self.sprites.clear_sprites();
        self.planes.skyflatnum = self.tn.skyflatnum;

        // R_RenderBSPNode: walls, visplanes and pending sprite sectors.
        let bspnum = if world.level.nodes.is_empty() {
            -1
        } else {
            world.level.nodes.len() as i32 - 1
        };
        self.seg.render_bsp(
            bspnum,
            render,
            &mut self.planes,
            &world.level,
            &self.tn,
            &mut self.data,
            manager,
            screen,
            &mut self.sprites,
        );

        // R_DrawPlanes.
        self.planes
            .draw_planes(render, &mut self.data, manager, screen);

        // Project + R_DrawMasked (things and masked walls).
        self.sprites.project_sprites(
            &world.level,
            &world.mobjs,
            render,
            &self.data.sprite_width,
            &self.data.sprite_offset,
            &self.data.sprite_top_offset,
        );
        let colormaps = self.data.colormaps.clone();
        self.sprites.draw_masked(
            &mut self.seg,
            render,
            &mut self.planes,
            &world.level,
            &self.tn,
            &mut self.data,
            manager,
            screen,
            &colormaps,
        );

        // R_DrawPlayerSprites (the weapon overlay): the light row is the
        // player's own sector light plus extralight, like R_SetupFrame.
        let colormaps = colormaps.clone();
        let player = world.players[self.displayplayer].clone();
        let (sector, extralight) = match player.mo {
            Some(mo) => world
                .mobjs
                .get(mo)
                .map(|m| (m.sector, render.extralight))
                .unwrap_or((0, 0)),
            None => (0, 0),
        };
        let lightnum =
            (world.level.sectors[sector].lightlevel >> crate::r_main::LIGHTSEGSHIFT) as i32
                + extralight;
        self.sprites.draw_player_sprites(
            &player,
            lightnum,
            render,
            &mut self.data,
            manager,
            screen,
            &colormaps,
        );

        let _ = (SCREENWIDTH, SCREENHEIGHT, ST_HEIGHT, viewheight);
    }
}

#[cfg(test)]
mod tests {
    use crate::r_main::Render;
    use super::*;
    use crate::level::{Line, Sector, Subsector, Vertex};
    use crate::p_mobj::spawn_mobj;

    /// One square room with two wall textures from a synthetic WAD is too
    /// heavy for a unit test; here we validate the facade mechanics on an
    /// empty level (no nodes, one subsector, one thing).
    #[test]
    fn renders_empty_room_without_panicking() {
        let manager = WadManager::new();
        let mut level = Level::default();
        level.sectors.push(Sector {
            floorheight: 0,
            ceilingheight: 128 << 16,
            ..Sector::default()
        });
        level.subsectors.push(Subsector {
            sector: 0,
            numlines: 0,
            firstline: 0,
        });
        level.lines.push(Line {
            v1: 0,
            v2: 1,
            frontsector: Some(0),
            ..Line::default()
        });
        level.vertexes.push(Vertex { x: 0, y: 0 });

        let mut world = World::new(level, Render::new());
        world.render.set_view_size(10, 0);
        world.render.execute_set_view_size();
        world.player_in_game[0] = true;
        let mo = spawn_mobj(
            &mut world.mobjs,
            &mut world.thinkers,
            &world.level,
            &world.render,
            crate::info::MobjType::MT_PLAYER,
            0,
            0,
            crate::p_mobj::ONFLOORZ,
        );
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.player = Some(0);
        }
        world.players[0].mo = Some(mo);

        // TextureNumbers::resolve fails without a real WAD: build a bare
        // FrameRenderer by hand instead.
        let mut fr = FrameRenderer {
            data: RenderData::default(),
            tn: TextureNumbers {
                side_top: alloc::vec![0],
                side_bottom: alloc::vec![0],
                side_mid: alloc::vec![-1],
                sector_floor: alloc::vec![0],
                sector_ceiling: alloc::vec![0],
                skyflatnum: 0,
                skytexture: -1,
            },
            planes: PlaneManager::new(),
            seg: SegManager::new(),
            sprites: SpriteManager::new(crate::r_things::SpriteDefs::default(), 1),
            displayplayer: 0,
        };
        let _ = manager;
        let mut screen = [0u8; 320 * 168];
        fr.render_view(&mut world, &manager, &mut screen);
        // The renderer ran; with no visible geometry the screen stays black.
        assert!(screen.iter().all(|&p| p == 0));
    }
}
