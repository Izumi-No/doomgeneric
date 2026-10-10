//! Player thinking: bobbing POV/weapon, movement, pending weapon, port of
//! `p_user.c`.

use crate::d_player::PlayerState;
use crate::d_ticcmd::{BT_CHANGE, BT_SPECIAL, BT_USE, BT_WEAPONMASK, BT_WEAPONSHIFT};
use crate::doomdef::{PowerType, WeaponType};
use crate::fixed::{fixed_mul, Fixed, FRAC_UNIT};
use crate::p_pspr::move_psprites;
use crate::p_world::World;
use crate::tables::{
    finecosine_index, finesine_index, FINEANGLES, FINE_MASK, ANGLETOFINESHIFT, ANG180, ANG90,
};

/// `VIEWHEIGHT` (`p_local.h`).
pub const VIEWHEIGHT: Fixed = 41 * FRAC_UNIT;

// 16 pixels of bob.
const MAXBOB: Fixed = 0x100000;

/// `ANG5`.
const ANG5: u32 = ANG90 / 18;

/// `P_Thrust`: move the player origin along `angle` by `move`.
fn thrust(world: &mut World, pnum: usize, angle: u32, move_: Fixed) {
    let Some(mo) = world.players[pnum].mo else { return };
    let a = angle >> ANGLETOFINESHIFT;
    let dmx = fixed_mul(move_, finecosine_index(a));
    let dmy = fixed_mul(move_, finesine_index(a));
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.momx += dmx;
        m.momy += dmy;
    }
}

/// `P_CalcHeight`: the walking / running height adjustment.
fn calc_height(world: &mut World, pnum: usize, onground: bool) {
    // Regular movement bobbing (needed for gun swing even if not on ground).
    let Some(mo) = world.players[pnum].mo else { return };
    let (momx, momy) = {
        let m = world.mobjs.get(mo).unwrap();
        (m.momx, m.momy)
    };
    let mut bob = fixed_mul(momx, momx) + fixed_mul(momy, momy);
    bob >>= 2;
    if bob > MAXBOB {
        bob = MAXBOB;
    }
    world.players[pnum].bob = bob;

    if world.players[pnum].cheats & crate::d_player::Player::CF_NOMOMENTUM != 0 || !onground {
        // (The vanilla ceiling clamp just above is dead code there too.)
        let z = world.mobjs.get(mo).unwrap().z;
        world.players[pnum].viewz = z + world.players[pnum].viewheight;
        return;
    }

    let angle = (FINEANGLES / 20 * world.leveltime as usize) & FINE_MASK;
    let bob = fixed_mul(world.players[pnum].bob / 2, finesine_index(angle as u32));

    // move viewheight
    if world.players[pnum].playerstate == PlayerState::Live {
        world.players[pnum].viewheight += world.players[pnum].deltaviewheight;

        if world.players[pnum].viewheight > VIEWHEIGHT {
            world.players[pnum].viewheight = VIEWHEIGHT;
            world.players[pnum].deltaviewheight = 0;
        }

        if world.players[pnum].viewheight < VIEWHEIGHT / 2 {
            world.players[pnum].viewheight = VIEWHEIGHT / 2;
            if world.players[pnum].deltaviewheight <= 0 {
                world.players[pnum].deltaviewheight = 1;
            }
        }

        if world.players[pnum].deltaviewheight != 0 {
            world.players[pnum].deltaviewheight += FRAC_UNIT / 4;
            if world.players[pnum].deltaviewheight == 0 {
                world.players[pnum].deltaviewheight = 1;
            }
        }
    }

    let (z, ceilingz) = {
        let m = world.mobjs.get(mo).unwrap();
        (m.z, m.ceilingz)
    };
    world.players[pnum].viewz = z + world.players[pnum].viewheight + bob;
    if world.players[pnum].viewz > ceilingz - 4 * FRAC_UNIT {
        world.players[pnum].viewz = ceilingz - 4 * FRAC_UNIT;
    }
}

/// `P_MovePlayer`.
fn move_player(world: &mut World, pnum: usize) -> bool {
    let Some(mo) = world.players[pnum].mo else { return false };
    let cmd = world.players[pnum].cmd;

    {
        let turn = (cmd.angleturn as i32) << 16;
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.angle = m.angle.wrapping_add(turn as u32);
        }
    }

    // Do not let the player control movement if not onground.
    let onground = world
        .mobjs
        .get(mo)
        .map(|m| m.z <= m.floorz)
        .unwrap_or(false);

    if cmd.forwardmove != 0 && onground {
        thrust(
            world,
            pnum,
            world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0),
            cmd.forwardmove as i32 * 2048,
        );
    }
    if cmd.sidemove != 0 && onground {
        thrust(
            world,
            pnum,
            world
                .mobjs
                .get(mo)
                .map(|m| m.angle)
                .unwrap_or(0)
                .wrapping_sub(ANG90),
            cmd.sidemove as i32 * 2048,
        );
    }

    let st = world.mobjs.get(mo).map(|m| m.state);
    if (cmd.forwardmove != 0 || cmd.sidemove != 0)
        && st == Some(crate::info::StateId::S_PLAY)
    {
        world.set_mobj_state(mo, crate::info::StateId::S_PLAY_RUN1);
    }

    onground
}

/// `P_DeathThink`: fall on your face when dying, decrease POV height to
/// floor height.
fn death_think(world: &mut World, pnum: usize) {
    move_psprites(world, pnum);

    // fall to the ground
    if world.players[pnum].viewheight > 6 * FRAC_UNIT {
        world.players[pnum].viewheight -= FRAC_UNIT;
    }
    if world.players[pnum].viewheight < 6 * FRAC_UNIT {
        world.players[pnum].viewheight = 6 * FRAC_UNIT;
    }
    world.players[pnum].deltaviewheight = 0;

    let (_mo, onground) = {
        let Some(mo) = world.players[pnum].mo else { return };
        let m = world.mobjs.get(mo).unwrap();
        (mo, m.z <= m.floorz)
    };
    calc_height(world, pnum, onground);

    let attacker = world.players[pnum].attacker;
    if let (Some(a), Some(mo)) = (attacker, world.players[pnum].mo) {
        if a != mo {
            let (mx, my, ax, ay, mo_angle) = {
                let m = world.mobjs.get(mo).unwrap();
                let t = world.mobjs.get(a);
                (
                    m.x,
                    m.y,
                    t.map(|t| t.x).unwrap_or(0),
                    t.map(|t| t.y).unwrap_or(0),
                    m.angle,
                )
            };
            let angle = crate::r_main::point_to_angle_from(mx, my, ax, ay);
            let delta = angle.wrapping_sub(mo_angle);

            if delta < ANG5 || delta > 0u32.wrapping_sub(ANG5) {
                // Looking at killer, so fade damage flash down.
                if let Some(m) = world.mobjs.get_mut(mo) {
                    m.angle = angle;
                }
                if world.players[pnum].damagecount != 0 {
                    world.players[pnum].damagecount -= 1;
                }
            } else if delta < ANG180 {
                if let Some(m) = world.mobjs.get_mut(mo) {
                    m.angle = m.angle.wrapping_add(ANG5);
                }
            } else if let Some(m) = world.mobjs.get_mut(mo) {
                m.angle = m.angle.wrapping_sub(ANG5);
            }
        }
    } else if world.players[pnum].damagecount != 0 {
        world.players[pnum].damagecount -= 1;
    }

    if world.players[pnum].cmd.buttons & BT_USE != 0 {
        world.players[pnum].playerstate = PlayerState::Reborn;
    }
}

/// `P_PlayerThink`.
pub fn player_think(world: &mut World, pnum: usize) {
    // fixme: do this in the cheat code
    if let Some(mo) = world.players[pnum].mo {
        let noclip = world.players[pnum].cheats & crate::d_player::Player::CF_NOCLIP != 0;
        if let Some(m) = world.mobjs.get_mut(mo) {
            if noclip {
                m.flags |= crate::p_mobj::MF_NOCLIP;
            } else {
                m.flags &= !crate::p_mobj::MF_NOCLIP;
            }
        }
    }

    // chain saw run forward
    if let Some(mo) = world.players[pnum].mo {
        let just_attacked = world.mobjs.get(mo).map(|m| m.flags & crate::p_mobj::MF_JUSTATTACKED != 0) == Some(true);
        if just_attacked {
            world.players[pnum].cmd.angleturn = 0;
            world.players[pnum].cmd.forwardmove = 100; // 0xc800/512 in vanilla
            world.players[pnum].cmd.sidemove = 0;
            if let Some(m) = world.mobjs.get_mut(mo) {
                m.flags &= !crate::p_mobj::MF_JUSTATTACKED;
            }
        }
    }

    if world.players[pnum].playerstate == PlayerState::Dead {
        death_think(world, pnum);
        return;
    }

    // Move around. Reactiontime prevents movement for a bit after a teleport.
    let mut onground = false;
    if let Some(mo) = world.players[pnum].mo {
        let reactiontime = world.mobjs.get(mo).map(|m| m.reactiontime).unwrap_or(0);
        if reactiontime != 0 {
            if let Some(m) = world.mobjs.get_mut(mo) {
                m.reactiontime -= 1;
            }
        } else {
            onground = move_player(world, pnum);
        }
    }

    calc_height(world, pnum, onground);

    if let Some(mo) = world.players[pnum].mo {
        let sector = world.mobjs.get(mo).map(|m| m.sector).unwrap_or(0);
        if world.level.sectors[sector].special != 0 {
            crate::p_spec::player_in_special_sector(world, pnum);
        }
    }

    // Check for weapon change. A special event has no other buttons.
    if world.players[pnum].cmd.buttons & BT_SPECIAL != 0 {
        world.players[pnum].cmd.buttons = 0;
    }

    if world.players[pnum].cmd.buttons & BT_CHANGE != 0 {
        // The actual changing of the weapon is done when the weapon psprite
        // can do it (read: not in the middle of an attack).
        let idx = (world.players[pnum].cmd.buttons & BT_WEAPONMASK) >> BT_WEAPONSHIFT;
        let mut newweapon = WeaponType::from_index(idx as i32);

        if newweapon == WeaponType::Fist
            && world.players[pnum].weaponowned[WeaponType::Chainsaw as usize]
            && !((world.players[pnum].readyweapon == WeaponType::Chainsaw)
                && world.players[pnum].powers[PowerType::Strength as usize] != 0)
        {
            newweapon = WeaponType::Chainsaw;
        }

        if world.commercial
            && newweapon == WeaponType::Shotgun
            && world.players[pnum].weaponowned[WeaponType::SuperShotgun as usize]
            && world.players[pnum].readyweapon != WeaponType::SuperShotgun
        {
            newweapon = WeaponType::SuperShotgun;
        }

        if world.players[pnum].weaponowned[newweapon as usize]
            && newweapon != world.players[pnum].readyweapon
        {
            // Do not go to plasma or BFG in shareware, even if cheated.
            if (newweapon != WeaponType::Plasma && newweapon != WeaponType::Bfg)
                || !world.shareware
            {
                world.players[pnum].pendingweapon = Some(newweapon);
            }
        }
    }

    // check for use
    if world.players[pnum].cmd.buttons & BT_USE != 0 {
        if !world.players[pnum].usedown {
            world.use_lines(pnum);
            world.players[pnum].usedown = true;
        }
    } else {
        world.players[pnum].usedown = false;
    }

    // cycle psprites
    move_psprites(world, pnum);

    // Counters, time dependend power ups.

    // Strength counts up to diminish fade.
    if world.players[pnum].powers[PowerType::Strength as usize] != 0 {
        world.players[pnum].powers[PowerType::Strength as usize] += 1;
    }
    if world.players[pnum].powers[PowerType::Invulnerability as usize] != 0 {
        world.players[pnum].powers[PowerType::Invulnerability as usize] -= 1;
    }
    if world.players[pnum].powers[PowerType::Invisibility as usize] != 0 {
        world.players[pnum].powers[PowerType::Invisibility as usize] -= 1;
        if world.players[pnum].powers[PowerType::Invisibility as usize] == 0 {
            if let Some(mo) = world.players[pnum].mo {
                if let Some(m) = world.mobjs.get_mut(mo) {
                    m.flags &= !crate::p_mobj::MF_SHADOW;
                }
            }
        }
    }
    if world.players[pnum].powers[PowerType::InfraRed as usize] != 0 {
        world.players[pnum].powers[PowerType::InfraRed as usize] -= 1;
    }
    if world.players[pnum].powers[PowerType::IronFeet as usize] != 0 {
        world.players[pnum].powers[PowerType::IronFeet as usize] -= 1;
    }
    if world.players[pnum].damagecount != 0 {
        world.players[pnum].damagecount -= 1;
    }
    if world.players[pnum].bonuscount != 0 {
        world.players[pnum].bonuscount -= 1;
    }

    // Handling colormaps.
    const INVERSECOLORMAP: i32 = 32;
    if world.players[pnum].powers[PowerType::Invulnerability as usize] != 0 {
        if world.players[pnum].powers[PowerType::Invulnerability as usize] > 4 * 32
            || (world.players[pnum].powers[PowerType::Invulnerability as usize] & 8) != 0
        {
            world.players[pnum].fixedcolormap = INVERSECOLORMAP;
        } else {
            world.players[pnum].fixedcolormap = 0;
        }
    } else if world.players[pnum].powers[PowerType::InfraRed as usize] != 0 {
        if world.players[pnum].powers[PowerType::InfraRed as usize] > 4 * 32
            || (world.players[pnum].powers[PowerType::InfraRed as usize] & 8) != 0
        {
            // almost full bright
            world.players[pnum].fixedcolormap = 1;
        } else {
            world.players[pnum].fixedcolormap = 0;
        }
    } else {
        world.players[pnum].fixedcolormap = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_ticcmd::TicCmd;
    use crate::level::{Sector, Subsector};
    use crate::p_mobj::spawn_mobj;

    fn world_with_player() -> World {
        let mut level = crate::level::Level::default();
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
        let mut w = World::new(level, crate::r_main::Render::new());
        let mo = spawn_mobj(
            &mut w.mobjs,
            &mut w.thinkers,
            &w.level,
            &w.render,
            crate::info::MobjType::MT_PLAYER,
            0,
            0,
            crate::p_mobj::ONFLOORZ,
        );
        if let Some(m) = w.mobjs.get_mut(mo) {
            m.player = Some(0);
        }
        w.players[0].mo = Some(mo);
        w.player_in_game[0] = true;
        w
    }

    #[test]
    fn forward_command_pushes_player() {
        let mut w = world_with_player();
        w.players[0].cmd = TicCmd {
            forwardmove: 50,
            ..TicCmd::default()
        };
        for _ in 0..5 {
            player_think(&mut w, 0);
            // The mobj thinkers turn momentum into movement (like G_Ticker).
            w.run_thinkers();
        }
        let m = w.mobjs.get(w.players[0].mo.unwrap()).unwrap();
        // Thrust is along +x (angle 0); the fine tables give the sideways
        // component a tiny non-zero value, like vanilla.
        assert!(m.x > m.y);
        assert!(m.x >> 16 > 0);
    }

    #[test]
    fn turning_adds_to_angle() {
        let mut w = world_with_player();
        w.players[0].cmd = TicCmd {
            angleturn: 100,
            ..TicCmd::default()
        };
        player_think(&mut w, 0);
        let m = w.mobjs.get(w.players[0].mo.unwrap()).unwrap();
        assert_eq!(m.angle, (100i32 << 16) as u32);
    }

    #[test]
    fn use_button_sets_usedown() {
        let mut w = world_with_player();
        w.players[0].cmd.buttons = BT_USE;
        player_think(&mut w, 0);
        assert!(w.players[0].usedown);
        w.players[0].cmd.buttons = 0;
        player_think(&mut w, 0);
        assert!(!w.players[0].usedown);
    }

    #[test]
    fn weapon_change_button_is_pending() {
        let mut w = world_with_player();
        w.players[0].weaponowned[WeaponType::Shotgun as usize] = true;
        // BT_CHANGE | weapon 2 (shotgun) = buttons 4 | (2<<3) = 20.
        w.players[0].cmd.buttons = BT_CHANGE | (2 << BT_WEAPONSHIFT);
        player_think(&mut w, 0);
        assert_eq!(w.players[0].pendingweapon, Some(WeaponType::Shotgun));
    }
}
