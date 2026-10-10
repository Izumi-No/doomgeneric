//! Player interaction: pickups, damage and death, port of `p_inter.c`.
//!
//! All references are arena handles: `MobjId` for things, `usize` for player
//! indexes into the world's player array. Vanilla global state (`gamemode`,
//! `gamemap`, `wminfo`, `deathmatch`, `netgame`) comes in as explicit context
//! arguments.

use crate::d_player::Player;
use crate::doomdef::{AmmoType, Card, PowerType, WeaponType};
use crate::fixed::{fixed_mul, Fixed, FRAC_UNIT};
use crate::info::{MobjType, MOBJINFO};
use crate::p_mobj::{
    Mobjs, MobjId, MF_COUNTKILL, MF_CORPSE, MF_DROPPED, MF_FLOAT, MF_NOCLIP,
    MF_NOGRAVITY, MF_SKULLFLY, MF_SOLID, MF_SHOOTABLE, ONFLOORZ,
};
use crate::p_tick::Thinkers;
use crate::random::Random;
use crate::r_main::{point_to_angle_from, Render};
use crate::level::Level;

/// `BONUSADD` (bonus screen flash duration).
pub const BONUSADD: i32 = 6;

/// `clipammo` per ammo type (`d_items.c`).
pub const CLIP_AMMO: [i32; 4] = [10, 4, 20, 1];

/// Rank ordering of pickups, for weapon switching.
pub struct Interaction {
    pub netgame: bool,
    pub deathmatch: i32,
    /// `skill`: 0 = baby, 4 = nightmare (mirrors `gameskill`).
    pub skill: i32,
    pub commercial: bool,
    /// The player index acting (vanilla `consoleplayer` for sound effects).
    pub consoleplayer: usize,
    pub wminfo_frags: i32,
    /// Totals used for intermission stats.
    pub totalkills: i32,
    pub totalitems: i32,
    pub leveltime: i32,
}

impl Default for Interaction {
    fn default() -> Self {
        Interaction {
            netgame: false,
            deathmatch: 0,
            skill: 2,
            commercial: false,
            consoleplayer: 0,
            wminfo_frags: 0,
            totalkills: 0,
            totalitems: 0,
            leveltime: 0,
        }
    }
}

/// `P_GiveBody`.
pub fn give_body(player: &mut Player, mobjs: &mut Mobjs, num: i32) -> bool {
    if player.health >= 100 {
        return false;
    }
    player.health = (player.health + num).min(100);
    if let Some(moid) = player.mo {
        if let Some(m) = mobjs.get_mut(moid) {
            m.health = player.health;
        }
    }
    true
}

/// `P_GiveArmor` (vanilla behavior; `deh_green_armor_class` = 1,
/// `deh_blue_armor_class` = 2).
pub fn give_armor(player: &mut Player, armortype: i32) -> bool {
    let hits = armortype * 100;
    if player.armorpoints >= hits {
        return false;
    }
    player.armortype = armortype;
    player.armorpoints = hits;
    true
}

/// `P_GiveCard`.
pub fn give_card(player: &mut Player, card: Card) {
    if player.has_card(card) {
        return;
    }
    player.bonuscount = BONUSADD;
    player.cards[card as usize] = true;
}

/// `P_GivePower`. `power` indexes `PowerType`.
pub fn give_power(mobjs: &mut Mobjs, player: &mut Player, power: PowerType) -> bool {
    match power {
        PowerType::Invulnerability => {
            player.powers[power as usize] = crate::doomdef::INVULN_TICS;
            true
        }
        PowerType::Invisibility => {
            player.powers[power as usize] = crate::doomdef::INVIS_TICS;
            if let Some(mo) = player.mo {
                if let Some(m) = mobjs.get_mut(mo) {
                    m.flags |= crate::p_mobj::MF_SHADOW;
                }
            }
            true
        }
        PowerType::InfraRed => {
            player.powers[power as usize] = crate::doomdef::INFRARED_TICS;
            true
        }
        PowerType::IronFeet => {
            player.powers[power as usize] = crate::doomdef::IRON_TICS;
            true
        }
        PowerType::Strength => {
            give_body(player, mobjs, 100);
            player.powers[power as usize] = 1;
            true
        }
        PowerType::AllMap => {
            if player.powers[power as usize] != 0 {
                return false;
            }
            player.powers[power as usize] = 1;
            true
        }
    }
}

/// `P_GiveAmmo`. `skill==0` (baby) and `skill==4` (nightmare) double it, like
/// vanilla.
pub fn give_ammo(player: &mut Player, ammo: AmmoType, num: i32, skill: i32) -> bool {
    let ai = ammo as usize;
    if ai >= crate::doomdef::NUM_AMMO {
        return false;
    }
    if player.ammo[ai] == player.maxammo[ai] {
        return false;
    }

    let amount = if num != 0 { num * CLIP_AMMO[ai] } else { CLIP_AMMO[ai] / 2 };
    let amount = if skill == 0 || skill == 4 { amount * 2 } else { amount };

    let oldammo = player.ammo[ai];
    player.ammo[ai] += amount;
    if player.ammo[ai] > player.maxammo[ai] {
        player.ammo[ai] = player.maxammo[ai];
    }

    if oldammo > 0 {
        return true;
    }

    // out-of-ammo weapon switch: like the original switch statement.
    match ammo {
        AmmoType::Clip => {
            if player.readyweapon == WeaponType::Fist {
                player.pendingweapon = Some(if player.weaponowned[WeaponType::Chaingun as usize] {
                    WeaponType::Chaingun
                } else {
                    WeaponType::Pistol
                });
            }
        }
        AmmoType::Shell => {
            if (player.readyweapon == WeaponType::Fist || player.readyweapon == WeaponType::Pistol)
                && player.weaponowned[WeaponType::Shotgun as usize]
            {
                player.pendingweapon = Some(WeaponType::Shotgun);
            }
        }
        AmmoType::Cell => {
            if (player.readyweapon == WeaponType::Fist || player.readyweapon == WeaponType::Pistol)
                && player.weaponowned[WeaponType::Plasma as usize]
            {
                player.pendingweapon = Some(WeaponType::Plasma);
            }
        }
        AmmoType::Missile => {
            if player.readyweapon == WeaponType::Fist
                && player.weaponowned[WeaponType::Missile as usize]
            {
                player.pendingweapon = Some(WeaponType::Missile);
            }
        }
    }
    true
}

/// `P_GiveWeapon`. `dropped` mirrors the `MF_DROPPED` flag path.
pub fn give_weapon(
    player: &mut Player,
    weapon: WeaponType,
    dropped: bool,
    skill: i32,
) -> bool {
    let wi = weapon as usize;
    let gave_ammo = if crate::info::WEAPONINFO[wi].ammo.is_some() {
        give_ammo(player, crate::info::WEAPONINFO[wi].ammo.unwrap(), 1 + dropped as i32, skill)
    } else {
        false
    };

    let gave_weapon = if player.weaponowned[wi] {
        false
    } else {
        player.weaponowned[wi] = true;
        player.pendingweapon = Some(weapon);
        true
    };
    gave_ammo || gave_weapon
}

/// `P_TouchSpecialThing`: apply the special (powerup/weapon/card/armor).
/// Returns `true` when the special was consumed.
#[allow(clippy::too_many_arguments)]
pub fn touch_special_thing(
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    players: &mut [Player; crate::doomdef::MAX_PLAYERS],
    context: &Interaction,
    special: MobjId,
    toucher: MobjId,
) -> bool {
    let Some(sp) = mobjs.get(special) else { return false };
    let sz = sp.z;
    let spflags = sp.flags;
    let spsprite = sp.sprite;
    let Some(tc) = mobjs.get(toucher) else { return false };
    let th = tc.height;
    let tz = tc.z;
    let Some(pnum) = tc.player else { return false };

    let delta = sz - tz;
    if delta > th || delta < -(8 * FRAC_UNIT) {
        return false; // out of reach
    }

    if mobjs.get(toucher).map(|m| m.health).unwrap_or(0) <= 0 {
        return false; // sliding corpse
    }

    let player = &mut players[pnum];
    use crate::info::Sprite::*;

    // Sound would go here (deferred): sfx_itemup / sfx_getpow / sfx_wpnup.
    let powermsg = false;
    let _ = powermsg;

    match spsprite {
        SPR_ARM1 => {
            if !give_armor(player, 1) {
                return false;
            }
        }
        SPR_ARM2 => {
            if !give_armor(player, 2) {
                return false;
            }
        }
        SPR_BON1 => {
            player.health = (player.health + 1).min(200);
            if let Some(mo) = player.mo {
                if let Some(m) = mobjs.get_mut(mo) {
                    m.health = player.health;
                }
            }
        }
        SPR_BON2 => {
            player.armorpoints = (player.armorpoints + 1).min(200);
            if player.armortype == 0 {
                player.armortype = 1;
            }
        }
        SPR_SOUL => {
            player.health = (player.health + 100).min(200);
            if let Some(mo) = player.mo {
                if let Some(m) = mobjs.get_mut(mo) {
                    m.health = player.health;
                }
            }
        }
        SPR_MEGA => {
            if !context.commercial {
                return false;
            }
            player.health = 200;
            give_armor(player, 2);
            if let Some(mo) = player.mo {
                if let Some(m) = mobjs.get_mut(mo) {
                    m.health = player.health;
                }
            }
        }
        SPR_BKEY => give_card(player, Card::BlueCard),
        SPR_YKEY => give_card(player, Card::YellowCard),
        SPR_RKEY => give_card(player, Card::RedCard),
        SPR_BSKU => give_card(player, Card::BlueSkull),
        SPR_YSKU => give_card(player, Card::YellowSkull),
        SPR_RSKU => give_card(player, Card::RedSkull),
        SPR_STIM => {
            if !give_body(player, mobjs, 10) {
                return false;
            }
        }
        SPR_MEDI => {
            if !give_body(player, mobjs, 25) {
                return false;
            }
        }
        SPR_PINV => {
            if !give_power(mobjs, player, PowerType::Invulnerability) {
                return false;
            }
        }
        SPR_PSTR => {
            if !give_power(mobjs, player, PowerType::Strength) {
                return false;
            }
            if player.readyweapon != WeaponType::Fist {
                player.pendingweapon = Some(WeaponType::Fist);
            }
            if let Some(mo) = player.mo {
                if let Some(m) = mobjs.get_mut(mo) {
                    m.height = m.info().height;
                }
            }
        }
        SPR_PINS => {
            if !give_power(mobjs, player, PowerType::Invisibility) {
                return false;
            }
        }
        SPR_SUIT => {
            player.powers[PowerType::IronFeet as usize] = crate::doomdef::IRON_TICS;
        }
        SPR_PMAP => {
            if !give_power(mobjs, player, PowerType::AllMap) {
                return false;
            }
        }
        SPR_PVIS => {
            if !give_power(mobjs, player, PowerType::InfraRed) {
                return false;
            }
        }

        SPR_CLIP => {
            let num = if spflags & MF_DROPPED != 0 { 0 } else { 1 };
            if !give_ammo(player, AmmoType::Clip, num, context.skill) {
                return false;
            }
        }
        SPR_AMMO => {
            if !give_ammo(player, AmmoType::Clip, 5, context.skill) {
                return false;
            }
        }
        SPR_ROCK => {
            if !give_ammo(player, AmmoType::Missile, 1, context.skill) {
                return false;
            }
        }
        SPR_BROK => {
            if !give_ammo(player, AmmoType::Missile, 5, context.skill) {
                return false;
            }
        }
        SPR_CELL => {
            if !give_ammo(player, AmmoType::Cell, 1, context.skill) {
                return false;
            }
        }
        SPR_CELP => {
            if !give_ammo(player, AmmoType::Cell, 5, context.skill) {
                return false;
            }
        }
        SPR_SHEL => {
            if !give_ammo(player, AmmoType::Shell, 1, context.skill) {
                return false;
            }
        }
        SPR_SBOX => {
            if !give_ammo(player, AmmoType::Shell, 5, context.skill) {
                return false;
            }
        }
        SPR_BPAK => {
            if !player.backpack {
                for i in 0..crate::doomdef::NUM_AMMO {
                    player.maxammo[i] *= 2;
                }
                player.backpack = true;
            }
            for i in 0..crate::doomdef::NUM_AMMO {
                let at = match i {
                    0 => AmmoType::Clip,
                    1 => AmmoType::Shell,
                    2 => AmmoType::Cell,
                    _ => AmmoType::Missile,
                };
                give_ammo(player, at, 1, context.skill);
            }
        }

        SPR_BFUG => {
            if !give_weapon(player, WeaponType::Bfg, false, context.skill) {
                return false;
            }
        }
        SPR_MGUN => {
            if !give_weapon(player, WeaponType::Chaingun, spflags & MF_DROPPED != 0, context.skill)
            {
                return false;
            }
        }
        SPR_CSAW => {
            if !give_weapon(player, WeaponType::Chainsaw, false, context.skill) {
                return false;
            }
        }
        SPR_LAUN => {
            if !give_weapon(player, WeaponType::Missile, false, context.skill) {
                return false;
            }
        }
        SPR_PLAS => {
            if !give_weapon(player, WeaponType::Plasma, false, context.skill) {
                return false;
            }
        }
        SPR_SHOT => {
            let dropped = spflags & MF_DROPPED != 0;
            if !give_weapon(player, WeaponType::Shotgun, dropped, context.skill) {
                return false;
            }
        }
        SPR_SGN2 => {
            let dropped = spflags & MF_DROPPED != 0;
            if !give_weapon(player, WeaponType::SuperShotgun, dropped, context.skill) {
                return false;
            }
        }
        _ => {
            // Unknown gettable thing; vanilla raises an error.
        }
    }

    if spflags & crate::p_mobj::MF_COUNTITEM != 0 {
        player.itemcount += 1;
    }
    player.bonuscount += BONUSADD;
    mobjs_remove_special(mobjs, thinkers, special);
    true
}

/// Remove a picked-up special thing (vanilla `P_RemoveMobj`).
fn mobjs_remove_special(mobjs: &mut Mobjs, thinkers: &mut Thinkers, special: MobjId) {
    if mobjs.is_live(special) {
        crate::p_mobj::remove_mobj(mobjs, thinkers, special);
    }
}

/// `P_KillMobj`. Drops items, updates the player death state.
#[allow(clippy::too_many_arguments)]
pub fn kill_mobj(
    level: &Level,
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    players: &mut [Player; crate::doomdef::MAX_PLAYERS],
    context: &Interaction,
    source: Option<MobjId>,
    target: MobjId,
) {
    let target_pnum = mobjs.get(target).and_then(|m| m.player);
    let target_type = mobjs.get(target).map(|m| m.ty).unwrap();
    {
        let Some(tm) = mobjs.get_mut(target) else { return };
        tm.flags &= !(MF_SHOOTABLE | MF_FLOAT | MF_SKULLFLY);
        if tm.ty != MobjType::MT_SKULL {
            tm.flags &= !MF_NOGRAVITY;
        }
        tm.flags |= MF_CORPSE | crate::p_mobj::MF_DROPOFF;
        tm.height >>= 2;
    }

    if let Some(pid) = context.source_player_index(source, mobjs) {
        if mobjs.get(target).map(|m| m.flags & MF_COUNTKILL) == Some(MF_COUNTKILL) {
            players[pid].killcount += 1;
        }
        if let Some(tp) = target_pnum {
            players[pid].frags[tp] += 1;
        }
    } else if !context.netgame && mobjs.get(target).map(|m| m.flags & MF_COUNTKILL) == Some(MF_COUNTKILL) {
        players[0].killcount += 1;
    }

    if let Some(tp) = target_pnum {
        if context.source_player_index(source, mobjs).is_none() {
            players[tp].frags[tp] += 1;
        }
        players[tp].playerstate = crate::d_player::PlayerState::Dead;
        players[tp].mo = Some(target);
        if let Some(m) = mobjs.get_mut(target) {
            m.flags &= !MF_SOLID;
        }
        // Drop the current weapon for later pickup (P_DropWeapon).
        let names = [
            None,
            Some(MobjType::MT_SHOTGUN),
            Some(MobjType::MT_CHAINGUN),
            Some(MobjType::MT_CLIP),
            None,
            None,
            None,
            None,
            None,
        ];
        if let Some(it) = names[players[tp].readyweapon as usize] {
            let (sx, sy) = mobjs.get(target).map(|m| (m.x, m.y)).unwrap_or((0, 0));
            let mi = crate::p_mobj::spawn_mobj(
                mobjs, thinkers, level, &Render::new(), it, sx, sy, ONFLOORZ,
            );
            if let Some(m) = mobjs.get_mut(mi) {
                m.flags |= MF_DROPPED;
            }
            players[tp].weaponowned[players[tp].readyweapon as usize] = false;
        }
    }

    // Choose death state.
    let (xdeath, death) = {
        let m = mobjs.get(target).unwrap();
        (m.info().xdeathstate, m.info().deathstate)
    };
    let health = mobjs.get(target).unwrap().health;
    let st = MOBJINFO[target_type as usize].spawnhealth;
    let pick = if health < -st && xdeath != crate::info::StateId::S_NULL {
        xdeath
    } else {
        death
    };
    crate::p_mobj::set_mobj_state(mobjs, thinkers, target, pick);

    let tics = mobjs.get(target).map(|m| m.tics).unwrap_or(1);
    // tics -= P_Random() & 3 is done by the caller (mobj_thinker ticks).
    let _ = tics;

    // Item drops.
    let drop: Option<MobjType> = match target_type {
        MobjType::MT_WOLFSS | MobjType::MT_POSSESSED => Some(MobjType::MT_CLIP),
        MobjType::MT_SHOTGUY => Some(MobjType::MT_SHOTGUN),
        MobjType::MT_CHAINGUY => Some(MobjType::MT_CHAINGUN),
        _ => None,
    };
    if let (Some(dt),) = (drop,) {
        let (x, y) = mobjs.get(target).map(|m| (m.x, m.y)).unwrap_or((0, 0));
        let mi = crate::p_mobj::spawn_mobj(
            mobjs, thinkers, level, &Render::new(), dt, x, y, ONFLOORZ,
        );
        if let Some(m) = mobjs.get_mut(mi) {
            m.flags |= MF_DROPPED;
        }
    }
}

/// Resolve `source`'s owning player index, if any.
impl Interaction {
    #[inline]
    pub fn source_player_index(&self, source: Option<MobjId>, mobjs: &Mobjs) -> Option<usize> {
        source
            .and_then(|s| mobjs.get(s))
            .and_then(|m| m.player)
    }
}

/// `P_DamageMobj`. Returns `true` if the target survived.
#[allow(clippy::too_many_arguments)]
pub fn damage_mobj(
    level: &Level,
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    rng: &mut Random,
    players: &mut [Player; crate::doomdef::MAX_PLAYERS],
    context: &Interaction,
    target: MobjId,
    inflictor: Option<MobjId>,
    _source_pos: Option<(Fixed, Fixed)>,
    source: Option<MobjId>,
    mut damage: i32,
) -> bool {
    let (tm_flags, tm_mass, tm_x, tm_y, tm_z, target_pnum) = {
        let Some(tm) = mobjs.get(target) else { return true };
        if tm.flags & MF_SHOOTABLE == 0 || tm.health <= 0 {
            return true;
        }
        (tm.flags, tm.info().mass.max(1), tm.x, tm.y, tm.z, tm.player)
    };

    if tm_flags & MF_SKULLFLY != 0 {
        let m = mobjs.get_mut(target).unwrap();
        m.momx = 0;
        m.momy = 0;
        m.momz = 0;
    }

    if target_pnum.is_some() && context.skill == 0 {
        damage >>= 1;
    }

    // Knockback: thrust the victim away from the inflictor, unless the source
    // is a player wielding the chainsaw (`P_DamageMobj`, thrust part).
    if let Some(inf) = inflictor {
        let chainsaw = source
            .and_then(|s| mobjs.get(s))
            .and_then(|m| m.player)
            .map(|p| players[p].readyweapon == WeaponType::Chainsaw)
            .unwrap_or(false);
        if !chainsaw && tm_flags & MF_NOCLIP == 0 {
            let (ix, iy) = mobjs.get(inf).map(|m| (m.x, m.y)).unwrap_or((0, 0));
            let mut ang = point_to_angle_from(ix, iy, tm_x, tm_y);
            let mut thrust = damage * (FRAC_UNIT >> 3) * 100 / tm_mass;

            // Occasionally the target falls forward instead of away.
            if damage < 40
                && damage > mobjs.get(target).unwrap().health
                && tm_z - mobjs.get(inf).map(|m| m.z).unwrap_or(0) > 64 * FRAC_UNIT
                && (rng.p.next() & 1) != 0
            {
                ang = ang.wrapping_add(crate::tables::ANG180);
                thrust *= 4;
            }

            let a = ang >> crate::tables::ANGLETOFINESHIFT;
            if let Some(m) = mobjs.get_mut(target) {
                m.momx += fixed_mul(thrust, crate::tables::finecosine_index(a));
                m.momy += fixed_mul(thrust, crate::tables::finesine_index(a));
            }
        }
    }

    // Player-specific damage path.
    if let Some(pnum) = target_pnum {
        // Hell-effect hack: the "end of game" sector sets damage to one less
        // than needed to keep the player alive.
        let sector = &level.sectors[mobjs.get(target).map(|m| m.sector).unwrap_or(0)];
        if sector.special == 11 && damage >= mobjs.get(target).unwrap().health {
            damage = mobjs.get(target).unwrap().health - 1;
        }

        // Ignore damage when god-mode or invulnerable (below the 1000 cap).
        if damage < 1000
            && (players[pnum].cheats & Player::CF_GODMODE != 0
                || players[pnum].has_power(PowerType::Invulnerability))
        {
            return true;
        }

        if players[pnum].armortype != 0 {
            let mut saved = damage / 3 * players[pnum].armortype; // type 1=1/3, 2=1/2
            if players[pnum].armortype == 2 {
                saved = damage / 2;
            }
            if players[pnum].armorpoints <= saved {
                saved = players[pnum].armorpoints;
                players[pnum].armortype = 0;
            }
            players[pnum].armorpoints -= saved;
            damage -= saved;
        }

        players[pnum].health -= damage;
        if players[pnum].health < 0 {
            players[pnum].health = 0;
        }

        players[pnum].attacker = source;
        players[pnum].damagecount += damage;
        if players[pnum].damagecount > 100 {
            players[pnum].damagecount = 100;
        }

        let _ = context.consoleplayer;
    }

    // Apply the final damage.
    let tl = mobjs.get_mut(target).unwrap();
    tl.health -= damage;
    if tl.health <= 0 {
        kill_mobj(level, mobjs, thinkers, players, context, source, target);
        return false;
    }

    // Pain chance (and `battle back` flag) with `MF_JUSTHIT`.
    {
        let painchance = mobjs.get(target).unwrap().info().painchance;
        let flags = mobjs.get(target).unwrap().flags;
        if (rng.p.next() as i32) < painchance && flags & MF_SKULLFLY == 0 {
            // fight back!
            let painstate = mobjs.get(target).unwrap().info().painstate;
            if let Some(m) = mobjs.get_mut(target) {
                m.flags |= crate::p_mobj::MF_JUSTHIT;
            }
            crate::p_mobj::set_mobj_state(mobjs, thinkers, target, painstate);
        }
    }

    {
        let Some(m) = mobjs.get_mut(target) else { return true };
        m.reactiontime = 0; // we're awake now...
    }

    // Target the attacker if this thing was idle (or a vile).
    let tm_tgt = mobjs.get(target).map(|m| (m.threshold, m.ty, m.state));
    if let (Some((threshold, ty, state)), Some(src)) = (tm_tgt, source) {
        if threshold == 0 && ty != MobjType::MT_VILE && Some(src) != Some(target) {
            let src_is_vile = mobjs.get(src).map(|m| m.ty) == Some(MobjType::MT_VILE);
            if !src_is_vile {
                let want_see;
                {
                    let m = mobjs.get_mut(target).unwrap();
                    m.target = Some(src);
                    m.threshold = 100; // BASETHRESHOLD
                    want_see = if state == m.info().spawnstate
                        && m.info().seestate != crate::info::StateId::S_NULL
                    {
                        Some(m.info().seestate)
                    } else {
                        None
                    };
                }
                if let Some(seestate) = want_see {
                    crate::p_mobj::set_mobj_state(mobjs, thinkers, target, seestate);
                }
            }
        }
    }

    true
}
