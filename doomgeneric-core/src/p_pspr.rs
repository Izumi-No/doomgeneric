//! Weapon sprite animation and weapon objects, port of `p_pspr.c`.
//!
//! Vanilla passes `(player_t*, pspdef_t*)` to every action; the port passes
//! the player index plus the sprite slot, with the player owned by the
//! [`World`].

use crate::d_player::PspriteNum;
use crate::d_ticcmd::BT_ATTACK;
use crate::doomdef::{AmmoType, WeaponType};
use crate::fixed::{fixed_mul, Fixed, FRAC_UNIT};
use crate::info::{Action, MobjType, StateId, WEAPONINFO};
use crate::p_enemy::{MELEERANGE, MISSILERANGE};
use crate::p_map::aim_line_attack;
use crate::p_mobj::{MobjId, MF_JUSTATTACKED};
use crate::p_world::World;
use crate::r_main::point_to_angle_from;
use crate::sounds::Sfx;
use crate::tables::{
    finecosine_index, finesine_index, FINEANGLES, FINE_MASK, ANG180, ANG90,
};

/// `LOWERSPEED` / `RAISESPEED`.
const LOWERSPEED: Fixed = FRAC_UNIT * 6;
const RAISESPEED: Fixed = FRAC_UNIT * 6;
/// `WEAPONBOTTOM` / `WEAPONTOP`.
const WEAPONBOTTOM: Fixed = 128 * FRAC_UNIT;
const WEAPONTOP: Fixed = 32 * FRAC_UNIT;

/// `deh_bfg_cells_per_shot`.
const BFG_CELLS_PER_SHOT: i32 = 40;

/// `P_SetPsprite`: start the psprite state chain at `state`, running actions.
pub fn set_psprite(world: &mut World, pnum: usize, position: PspriteNum, mut stnum: StateId) {
    loop {
        if stnum == StateId::S_NULL {
            // object removed itself
            world.players[pnum].psprites[position.index()].state = None;
            break;
        }

        let st = crate::info::STATES[stnum as usize];
        {
            let psp = &mut world.players[pnum].psprites[position.index()];
            psp.state = Some(stnum);
            psp.tics = st.tics; // could be 0

            if st.misc1 != 0 {
                // coordinate set
                psp.sx = st.misc1 << crate::fixed::FRAC_BITS;
                psp.sy = st.misc2 << crate::fixed::FRAC_BITS;
            }
        }

        // Call action routine.
        if st.action != Action::Null {
            run_psprite_action(world, pnum, position, st.action);
            if world.players[pnum].psprites[position.index()].state.is_none() {
                break;
            }
        }

        stnum = world.players[pnum].psprites[position.index()]
            .state
            .map(|s| crate::info::STATES[s as usize].nextstate)
            .unwrap_or(StateId::S_NULL);

        if world.players[pnum].psprites[position.index()].tics != 0 {
            break;
        }
    }
}

/// Weapon/muzzle-flash action dispatch (`state->action.acp2`).
pub fn run_psprite_action(world: &mut World, pnum: usize, position: PspriteNum, action: Action) {
    match action {
        Action::A_Light0 => a_light(world, pnum, 0),
        Action::A_Light1 => a_light(world, pnum, 1),
        Action::A_Light2 => a_light(world, pnum, 2),
        Action::A_WeaponReady => a_weapon_ready(world, pnum),
        Action::A_Lower => a_lower(world, pnum),
        Action::A_Raise => a_raise(world, pnum),
        Action::A_GunFlash => a_gun_flash(world, pnum),
        Action::A_ReFire => a_refire(world, pnum),
        Action::A_CheckReload => {
            check_ammo(world, pnum);
        }
        Action::A_Punch => a_punch(world, pnum),
        Action::A_Saw => a_saw(world, pnum),
        Action::A_FirePistol => a_fire_pistol(world, pnum),
        Action::A_FireShotgun => a_fire_shotgun(world, pnum),
        Action::A_FireShotgun2 => a_fire_shotgun2(world, pnum),
        Action::A_OpenShotgun2 => a_open_shotgun2(world, pnum),
        Action::A_LoadShotgun2 => a_load_shotgun2(world, pnum),
        Action::A_CloseShotgun2 => a_close_shotgun2(world, pnum),
        Action::A_FireCGun => a_fire_cgun(world, pnum, position),
        Action::A_FireMissile => a_fire_missile(world, pnum),
        Action::A_FirePlasma => a_fire_plasma(world, pnum),
        Action::A_FireBFG => a_fire_bfg(world, pnum),
        Action::A_BFGsound => {
            let mo = world.players[pnum].mo;
            let _ = mo;
            world.play_sfx(Sfx::sfx_bfg);
        }
        _ => {}
    }
}

/// `P_BringUpWeapon`: start bringing the pending weapon up from the bottom.
pub fn bring_up_weapon(world: &mut World, pnum: usize) {
    if world.players[pnum].pendingweapon.is_none() {
        world.players[pnum].pendingweapon = Some(world.players[pnum].readyweapon);
    }

    if world.players[pnum].pendingweapon == Some(WeaponType::Chainsaw) {
        world.play_sfx(Sfx::sfx_sawup);
    }

    let newstate = WEAPONINFO[world.players[pnum]
        .pendingweapon
        .unwrap_or(WeaponType::Fist) as usize]
    .upstate;
    world.players[pnum].pendingweapon = None;
    world.players[pnum].psprites[PspriteNum::Weapon.index()].sy = WEAPONBOTTOM;

    set_psprite(world, pnum, PspriteNum::Weapon, newstate);
}

/// `P_CheckAmmo`: enough ammo to shoot? If not, pick a weapon to change to.
pub fn check_ammo(world: &mut World, pnum: usize) -> bool {
    let ready = world.players[pnum].readyweapon;
    let ammo = WEAPONINFO[ready as usize].ammo;

    // Minimal amount for one shot varies.
    let count = if ready == WeaponType::Bfg {
        BFG_CELLS_PER_SHOT
    } else if ready == WeaponType::SuperShotgun {
        2 // Double barrel.
    } else {
        1 // Regular.
    };

    // Some do not need ammunition anyway.
    let have = match ammo {
        None => true,
        Some(a) => world.players[pnum].ammo[a as usize] >= count,
    };
    if have {
        return true;
    }

    // Out of ammo, pick a weapon to change to. Preferences are set here.
    let (weaponowned, ammo, shareware, commercial) = {
        let p = &world.players[pnum];
        (
            p.weaponowned,
            [p.ammo[0], p.ammo[1], p.ammo[2], p.ammo[3]],
            world.shareware,
            world.commercial,
        )
    };
    let [clip, shell, cell, misl] = [ammo[0], ammo[1], ammo[2], ammo[3]];

    let pending = if weaponowned[WeaponType::Plasma as usize] && cell != 0 && !shareware {
        Some(WeaponType::Plasma)
    } else if weaponowned[WeaponType::SuperShotgun as usize] && shell > 2 && commercial {
        Some(WeaponType::SuperShotgun)
    } else if weaponowned[WeaponType::Chaingun as usize] && clip != 0 {
        Some(WeaponType::Chaingun)
    } else if weaponowned[WeaponType::Shotgun as usize] && shell != 0 {
        Some(WeaponType::Shotgun)
    } else if clip != 0 {
        Some(WeaponType::Pistol)
    } else if weaponowned[WeaponType::Chainsaw as usize] {
        Some(WeaponType::Chainsaw)
    } else if weaponowned[WeaponType::Missile as usize] && misl != 0 {
        Some(WeaponType::Missile)
    } else if weaponowned[WeaponType::Bfg as usize] && cell > 40 && !shareware {
        Some(WeaponType::Bfg)
    } else {
        // If everything fails.
        Some(WeaponType::Fist)
    };
    world.players[pnum].pendingweapon = pending;

    // Now set appropriate weapon overlay.
    let downstate = WEAPONINFO[world.players[pnum].readyweapon as usize].downstate;
    set_psprite(world, pnum, PspriteNum::Weapon, downstate);

    false
}

/// `P_FireWeapon`.
pub fn fire_weapon(world: &mut World, pnum: usize) {
    if !check_ammo(world, pnum) {
        return;
    }

    if let Some(mo) = world.players[pnum].mo {
        world.set_mobj_state(mo, StateId::S_PLAY_ATK1);
    }
    let atkstate = WEAPONINFO[world.players[pnum].readyweapon as usize].atkstate;
    set_psprite(world, pnum, PspriteNum::Weapon, atkstate);
    if let Some(mo) = world.players[pnum].mo {
        crate::p_enemy::noise_alert(world, mo, mo);
    }
}

/// `P_DropWeapon`: player died, so put the weapon away.
pub fn drop_weapon(world: &mut World, pnum: usize) {
    let downstate = WEAPONINFO[world.players[pnum].readyweapon as usize].downstate;
    set_psprite(world, pnum, PspriteNum::Weapon, downstate);
}

/// `A_WeaponReady`: the player can fire the weapon or change to another.
pub fn a_weapon_ready(world: &mut World, pnum: usize) {
    // get out of attack state
    if let Some(mo) = world.players[pnum].mo {
        let st = world.mobjs.get(mo).map(|m| m.state).unwrap_or(StateId::S_NULL);
        if st == StateId::S_PLAY_ATK1 || st == StateId::S_PLAY_ATK2 {
            world.set_mobj_state(mo, StateId::S_PLAY);
        }
    }

    if world.players[pnum].readyweapon == WeaponType::Chainsaw
        && world.players[pnum].psprites[PspriteNum::Weapon.index()].state
            == Some(StateId::S_SAW)
    {
        world.play_sfx(Sfx::sfx_sawidl);
    }

    // check for change; if player is dead, put the weapon away
    if world.players[pnum].pendingweapon.is_some() || world.players[pnum].health == 0 {
        let downstate = WEAPONINFO[world.players[pnum].readyweapon as usize].downstate;
        set_psprite(world, pnum, PspriteNum::Weapon, downstate);
        return;
    }

    // check for fire: the missile launcher and BFG do not auto fire
    if world.players[pnum].cmd.buttons & BT_ATTACK != 0 {
        let ready = world.players[pnum].readyweapon;
        if !world.players[pnum].attackdown
            || (ready != WeaponType::Missile && ready != WeaponType::Bfg)
        {
            world.players[pnum].attackdown = true;
            fire_weapon(world, pnum);
            return;
        }
    } else {
        world.players[pnum].attackdown = false;
    }

    // bob the weapon based on movement speed
    let bob = world.players[pnum].bob;
    let angle = (128 * world.leveltime as u32) & FINE_MASK as u32;
    let sx = FRAC_UNIT + fixed_mul(bob, finecosine_index(angle));
    let angle = angle & (FINEANGLES / 2 - 1) as u32;
    let sy = WEAPONTOP + fixed_mul(bob, finesine_index(angle));
    world.players[pnum].psprites[PspriteNum::Weapon.index()].sx = sx;
    world.players[pnum].psprites[PspriteNum::Weapon.index()].sy = sy;
}

/// `A_ReFire`: the player can re-fire without lowering the weapon entirely.
pub fn a_refire(world: &mut World, pnum: usize) {
    if world.players[pnum].cmd.buttons & BT_ATTACK != 0
        && world.players[pnum].pendingweapon.is_none()
        && world.players[pnum].health != 0
    {
        world.players[pnum].refire += 1;
        fire_weapon(world, pnum);
    } else {
        world.players[pnum].refire = 0;
        check_ammo(world, pnum);
    }
}

/// `A_Lower`: lowers the current weapon and changes weapon at the bottom.
pub fn a_lower(world: &mut World, pnum: usize) {
    let psp = &mut world.players[pnum].psprites[PspriteNum::Weapon.index()];
    psp.sy += LOWERSPEED;

    // Is already down?
    if world.players[pnum].psprites[PspriteNum::Weapon.index()].sy < WEAPONBOTTOM {
        return;
    }

    // Player is dead.
    if world.players[pnum].playerstate == crate::d_player::PlayerState::Dead {
        world.players[pnum].psprites[PspriteNum::Weapon.index()].sy = WEAPONBOTTOM;
        // don't bring weapon back up
        return;
    }

    // The old weapon has been lowered off the screen, so change the weapon
    // and start raising it.
    if world.players[pnum].health == 0 {
        // Player is dead, so keep the weapon off screen.
        set_psprite(world, pnum, PspriteNum::Weapon, StateId::S_NULL);
        return;
    }

    world.players[pnum].readyweapon =
        world.players[pnum].pendingweapon.unwrap_or(world.players[pnum].readyweapon);

    bring_up_weapon(world, pnum);
}

/// `A_Raise`.
pub fn a_raise(world: &mut World, pnum: usize) {
    world.players[pnum].psprites[PspriteNum::Weapon.index()].sy -= RAISESPEED;

    if world.players[pnum].psprites[PspriteNum::Weapon.index()].sy > WEAPONTOP {
        return;
    }

    world.players[pnum].psprites[PspriteNum::Weapon.index()].sy = WEAPONTOP;

    // The weapon has been raised all the way, so change to the ready state.
    let readystate = WEAPONINFO[world.players[pnum].readyweapon as usize].readystate;
    set_psprite(world, pnum, PspriteNum::Weapon, readystate);
}

/// `A_GunFlash`.
pub fn a_gun_flash(world: &mut World, pnum: usize) {
    if let Some(mo) = world.players[pnum].mo {
        world.set_mobj_state(mo, StateId::S_PLAY_ATK2);
    }
    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    set_psprite(world, pnum, PspriteNum::Flash, flashstate);
}

// ---------------------------------------------------------------------------
// WEAPON ATTACKS
// ---------------------------------------------------------------------------

fn a_light(world: &mut World, pnum: usize, level: i32) {
    world.players[pnum].extralight = level;
}

/// `A_Punch`.
pub fn a_punch(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };

    let damage = ((world.rng.p.next() as i32 % 10) + 1) << 1;
    let damage = if world.players[pnum].powers[crate::doomdef::PowerType::Strength as usize] != 0 {
        damage * 10
    } else {
        damage
    };

    let angle = world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0);
    let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 18) as u32;
    let angle = angle.wrapping_add(disp);
    let vid = world.bump_validcount();
    let (slope, linetarget) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        angle,
        MELEERANGE,
    );
    world.line_attack(mo, angle, MELEERANGE, slope, damage);

    // turn to face target
    if let Some(t) = linetarget {
        world.play_sfx(Sfx::sfx_punch);
        let (mx, my, tx, ty) = {
            let (a, b) = (world.mobjs.get(mo).unwrap(), world.mobjs.get(t).unwrap());
            (a.x, a.y, b.x, b.y)
        };
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.angle = point_to_angle_from(mx, my, tx, ty);
        }
    }
}

/// `A_Saw`.
pub fn a_saw(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };

    let damage = 2 * ((world.rng.p.next() as i32 % 10) + 1);
    let angle = world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0);
    let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 18) as u32;
    let angle = angle.wrapping_add(disp);

    // use meleerange + 1 so the puff doesn't skip the flash
    let vid = world.bump_validcount();
    let (slope, linetarget) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        angle,
        MELEERANGE + 1,
    );
    world.line_attack(mo, angle, MELEERANGE + 1, slope, damage);

    let Some(t) = linetarget else {
        world.play_sfx(Sfx::sfx_sawful);
        return;
    };
    world.play_sfx(Sfx::sfx_sawhit);

    // turn to face target
    let (mx, my, tx, ty) = {
        let (a, b) = (world.mobjs.get(mo).unwrap(), world.mobjs.get(t).unwrap());
        (a.x, a.y, b.x, b.y)
    };
    let mut angle = point_to_angle_from(mx, my, tx, ty);
    let mo_angle = world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0);
    const ANG90_20: u32 = ANG90 / 20;
    if angle.wrapping_sub(mo_angle) > ANG180 {
        if (angle.wrapping_sub(mo_angle) as i32) < -(ANG90_20 as i32) {
            angle = angle.wrapping_add(ANG90 / 21);
            if let Some(m) = world.mobjs.get_mut(mo) {
                m.angle = angle;
            }
        } else if let Some(m) = world.mobjs.get_mut(mo) {
            m.angle = m.angle.wrapping_sub(ANG90 / 20);
        }
    } else if angle.wrapping_sub(mo_angle) > ANG90 / 20 {
        angle = angle.wrapping_sub(ANG90 / 21);
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.angle = angle;
        }
    } else if let Some(m) = world.mobjs.get_mut(mo) {
        m.angle = m.angle.wrapping_add(ANG90 / 20);
    }
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.flags |= MF_JUSTATTACKED;
    }
}

/// `DecreaseAmmo` (with the vanilla ammo-array overflow emulation dropped:
/// indices are always in range in the port).
fn decrease_ammo(world: &mut World, pnum: usize, ammo: Option<AmmoType>, amount: i32) {
    if let Some(a) = ammo {
        world.players[pnum].ammo[a as usize] -= amount;
    }
}

/// `A_FireMissile`.
pub fn a_fire_missile(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, 1);
    world.spawn_player_missile(mo, MobjType::MT_ROCKET);
}

/// `A_FireBFG`.
pub fn a_fire_bfg(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, BFG_CELLS_PER_SHOT);
    world.spawn_player_missile(mo, MobjType::MT_BFG);
}

/// `A_FirePlasma`.
pub fn a_fire_plasma(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, 1);

    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    let pick = world.rng.p.next() as i32 & 1;
    set_psprite(world, pnum, PspriteNum::Flash, StateId::from_index(flashstate as i32 + pick));

    world.spawn_player_missile(mo, MobjType::MT_PLASMA);
}

/// `P_BulletSlope` result holder for the shot functions.
fn bullet_slope(world: &mut World, mo: MobjId) -> Fixed {
    const RANGE: Fixed = 16 * 64 * FRAC_UNIT;
    let an = world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0);
    let vid = world.bump_validcount();
    let (slope, target) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        an,
        RANGE,
    );
    if target.is_some() {
        return slope;
    }
    let an = an.wrapping_add(1 << 26);
    let vid = world.bump_validcount();
    let (slope, target) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        an,
        RANGE,
    );
    if target.is_some() {
        return slope;
    }
    let an = an.wrapping_sub(2 << 26);
    let vid = world.bump_validcount();
    let (slope, _) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        an,
        RANGE,
    );
    slope
}

/// `P_GunShot` with a pre-computed slope.
fn gun_shot_with(world: &mut World, mo: MobjId, accurate: bool, slope: Fixed) {
    let damage = 5 * ((world.rng.p.next() as i32 % 3) + 1);
    let mut angle = world.mobjs.get(mo).map(|m| m.angle).unwrap_or(0);

    if !accurate {
        let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 18) as u32;
        angle = angle.wrapping_add(disp);
    }

    world.line_attack(mo, angle, MISSILERANGE, slope, damage);
}

/// `A_FirePistol`.
pub fn a_fire_pistol(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    world.play_sfx(Sfx::sfx_pistol);

    world.set_mobj_state(mo, StateId::S_PLAY_ATK2);
    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, 1);

    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    set_psprite(world, pnum, PspriteNum::Flash, flashstate);

    let slope = bullet_slope(world, mo);
    gun_shot_with(world, mo, world.players[pnum].refire == 0, slope);
}

/// `A_FireShotgun`.
pub fn a_fire_shotgun(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    world.play_sfx(Sfx::sfx_shotgn);
    world.set_mobj_state(mo, StateId::S_PLAY_ATK2);

    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, 1);

    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    set_psprite(world, pnum, PspriteNum::Flash, flashstate);

    let slope = bullet_slope(world, mo);
    for _ in 0..7 {
        gun_shot_with(world, mo, false, slope);
    }
}

/// `A_FireShotgun2`.
pub fn a_fire_shotgun2(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    world.play_sfx(Sfx::sfx_dshtgn);
    world.set_mobj_state(mo, StateId::S_PLAY_ATK2);

    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    decrease_ammo(world, pnum, ammo, 2);

    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    set_psprite(world, pnum, PspriteNum::Flash, flashstate);

    let slope = bullet_slope(world, mo);

    for _ in 0..20 {
        let damage = 5 * ((world.rng.p.next() as i32 % 3) + 1);
        let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 19) as u32;
        let angle = world
            .mobjs
            .get(mo)
            .map(|m| m.angle)
            .unwrap_or(0)
            .wrapping_add(disp);
        let zdisp = (world.rng.p.next() as i32 - world.rng.p.next() as i32) << 5;
        world.line_attack(mo, angle, MISSILERANGE, slope + zdisp, damage);
    }
}

/// `A_OpenShotgun2` / `A_LoadShotgun2` / `A_CloseShotgun2` (the
/// super-shotgun reload sequence).
pub fn a_open_shotgun2(world: &mut World, pnum: usize) {
    world.play_sfx(Sfx::sfx_dbopn);
    set_psprite(world, pnum, PspriteNum::Flash, StateId::S_DSGUNFLASH1);
}

pub fn a_load_shotgun2(world: &mut World, pnum: usize) {
    world.play_sfx(Sfx::sfx_dbload);
    set_psprite(world, pnum, PspriteNum::Flash, StateId::S_DSGUNFLASH2);
}

pub fn a_close_shotgun2(world: &mut World, pnum: usize) {
    world.play_sfx(Sfx::sfx_dbcls);
    set_psprite(world, pnum, PspriteNum::Flash, StateId::S_NULL);
}

/// `A_FireCGun`.
pub fn a_fire_cgun(world: &mut World, pnum: usize, position: PspriteNum) {
    let Some(mo) = world.players[pnum].mo else { return };
    world.play_sfx(Sfx::sfx_pistol);

    let ammo = WEAPONINFO[world.players[pnum].readyweapon as usize].ammo;
    if let Some(a) = ammo {
        if world.players[pnum].ammo[a as usize] == 0 {
            return;
        }
    }

    world.set_mobj_state(mo, StateId::S_PLAY_ATK2);
    decrease_ammo(world, pnum, ammo, 1);

    let flashstate = WEAPONINFO[world.players[pnum].readyweapon as usize].flashstate;
    let offset = world.players[pnum].psprites[position.index()]
        .state
        .map(|s| s as i32 - StateId::S_CHAIN1 as i32)
        .unwrap_or(0);
    set_psprite(
        world,
        pnum,
        PspriteNum::Flash,
        StateId::from_index(flashstate as i32 + offset),
    );

    let slope = bullet_slope(world, mo);
    gun_shot_with(world, mo, world.players[pnum].refire == 0, slope);
}

/// `A_BFGSpray` — a mobj action (the BFG ball's death state); lives here with
/// the other weapon code (`p_pspr.c`).
pub fn a_bfg_spray(world: &mut World, id: MobjId) {
    let Some(target_origin) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    const RANGE: Fixed = 16 * 64 * FRAC_UNIT;
    let base_angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);

    // offset angles from its attack angle
    for i in 0..40 {
        let an = base_angle
            .wrapping_sub(ANG90 / 2)
            .wrapping_add(ANG90 / 40 * i);

        // mo->target is the originator (player) of the missile
        let vid = world.bump_validcount();
        let (_, linetarget) = aim_line_attack(
            &world.level,
            &world.render,
            &world.mobjs,
            &mut world.line_vc,
            vid,
            target_origin,
            an,
            RANGE,
        );

        let Some(t) = linetarget else {
            continue;
        };

        let (tx, ty, tz) = {
            let tm = world.mobjs.get(t).unwrap();
            (tm.x, tm.y, tm.z + (tm.height >> 2))
        };
        crate::p_mobj::spawn_mobj(
            &mut world.mobjs,
            &mut world.thinkers,
            &world.level,
            &world.render,
            MobjType::MT_EXTRABFG,
            tx,
            ty,
            tz,
        );

        let mut damage = 0;
        for _ in 0..15 {
            damage += (world.rng.p.next() as i32 & 7) + 1;
        }

        world.damage_mobj(t, Some(target_origin), Some(target_origin), damage);
    }
}

/// `P_SetupPsprites`: called at the start of the level for each player.
pub fn setup_psprites(world: &mut World, pnum: usize) {
    // remove all psprites
    for psp in world.players[pnum].psprites.iter_mut() {
        psp.state = None;
    }

    // spawn the gun
    world.players[pnum].pendingweapon = Some(world.players[pnum].readyweapon);
    bring_up_weapon(world, pnum);
}

/// `P_MovePsprites`: called every tic by the player thinking routine.
pub fn move_psprites(world: &mut World, pnum: usize) {
    for i in 0..crate::d_player::NUM_PSPRITES {
        // a null state means not active
        if let Some(state) = world.players[pnum].psprites[i].state {
            // drop tic count and possibly change state; a -1 tic never changes
            if world.players[pnum].psprites[i].tics != -1 {
                world.players[pnum].psprites[i].tics -= 1;
                if world.players[pnum].psprites[i].tics == 0 {
                    let nextstate = crate::info::STATES[state as usize].nextstate;
                    set_psprite(
                        world,
                        pnum,
                        match i {
                            0 => PspriteNum::Weapon,
                            _ => PspriteNum::Flash,
                        },
                        nextstate,
                    );
                }
            }
        }
    }

    // The flash follows the weapon.
    let (sx, sy) = {
        let w = &world.players[pnum].psprites[PspriteNum::Weapon.index()];
        (w.sx, w.sy)
    };
    let f = &mut world.players[pnum].psprites[PspriteNum::Flash.index()];
    f.sx = sx;
    f.sy = sy;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Sector, Subsector};
    use crate::p_mobj::spawn_mobj;

    fn world_with_player() -> (World, usize, MobjId) {
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
        (w, 0, mo)
    }

    #[test]
    fn setup_and_cycle_psprites() {
        let (mut w, pnum, _mo) = world_with_player();
        setup_psprites(&mut w, pnum);
        // The pistol up-state chain settles in the ready state after the
        // ~16-tic raise (WEAPONBOTTOM -> WEAPONTOP at 6 units/tic).
        assert_eq!(w.players[pnum].readyweapon, WeaponType::Pistol);
        for _ in 0..80 {
            move_psprites(&mut w, pnum);
            if w.players[pnum].psprites[PspriteNum::Weapon.index()].state
                == Some(StateId::S_PISTOL)
            {
                break;
            }
        }
        assert_eq!(
            w.players[pnum].psprites[PspriteNum::Weapon.index()].state,
            Some(StateId::S_PISTOL)
        );

        // Firing cycles into the attack state and consumes ammo.
        let ammo0 = w.players[pnum].ammo[AmmoType::Clip as usize];
        fire_weapon(&mut w, pnum);
        // The attack state chains to S_PISTOL2, where A_FirePistol runs.
        assert_eq!(
            w.players[pnum].psprites[PspriteNum::Weapon.index()].state,
            Some(StateId::S_PISTOL1)
        );

        // After the attack tics run out the weapon returns to ready and the
        // shot has consumed ammo.
        for _ in 0..20 {
            move_psprites(&mut w, pnum);
        }
        assert!(w.players[pnum].ammo[AmmoType::Clip as usize] < ammo0);
        assert_eq!(
            w.players[pnum].psprites[PspriteNum::Weapon.index()].state,
            Some(StateId::S_PISTOL)
        );
    }

    #[test]
    fn lower_switches_to_pending_weapon() {
        let (mut w, pnum, _mo) = world_with_player();
        setup_psprites(&mut w, pnum);
        w.players[pnum].weaponowned[WeaponType::Chaingun as usize] = true;
        w.players[pnum].pendingweapon = Some(WeaponType::Chaingun);
        // A_Lower runs until the weapon is off screen, then swaps and starts
        // the chaingun up-state.
        for _ in 0..24 {
            a_lower(&mut w, pnum);
        }
        assert_eq!(w.players[pnum].readyweapon, WeaponType::Chaingun);
        assert_eq!(w.players[pnum].pendingweapon, None);
        // Tick the up-state out: A_Raise lands on the chaingun ready state.
        for _ in 0..20 {
            move_psprites(&mut w, pnum);
            a_raise(&mut w, pnum);
        }
        assert_eq!(
            w.players[pnum].psprites[PspriteNum::Weapon.index()].state,
            Some(StateId::S_CHAIN)
        );
    }

    #[test]
    fn check_ammo_switches_when_empty() {
        let (mut w, pnum, _mo) = world_with_player();
        setup_psprites(&mut w, pnum);
        w.players[pnum].ammo[AmmoType::Clip as usize] = 0;
        assert!(!check_ammo(&mut w, pnum));
        // The pending weapon is either still queued or already swapped in
        // (the down-state chains through `A_Lower` immediately).
        assert!(
            w.players[pnum].pendingweapon == Some(WeaponType::Fist)
                || w.players[pnum].readyweapon == WeaponType::Fist
        );
    }

    #[test]
    fn fire_cgun_offsets_flash_by_chain_state() {
        let (mut w, pnum, _mo) = world_with_player();
        w.players[pnum].readyweapon = WeaponType::Chaingun;
        w.players[pnum].weaponowned[WeaponType::Chaingun as usize] = true;
        w.players[pnum].ammo[AmmoType::Clip as usize] = 50;
        set_psprite(&mut w, pnum, PspriteNum::Weapon, StateId::S_CHAIN1);
        a_fire_cgun(&mut w, pnum, PspriteNum::Weapon);
        assert_eq!(
            w.players[pnum].psprites[PspriteNum::Flash.index()].state,
            Some(StateId::S_CHAINFLASH1)
        );
        assert!(w.players[pnum].ammo[AmmoType::Clip as usize] < 50);
    }
}
