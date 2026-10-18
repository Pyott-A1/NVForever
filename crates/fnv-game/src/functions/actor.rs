//! Actor functions: actor values, death, factions, voice, sex, race, flags.
//!
//! Actor value codes are the indices of `ActorValue` in `schemas/fnv/common.json`
//! [documented: xEdit; scripts.md §2.4]. What a value starts at (its record field or
//! formula) is `crate::actor_values`; docs/spec-notes/actor-values.md has the whole
//! model.
//!
//! A value is its base plus three pools [documented: GECK wiki ActorValue]:
//!
//! - the **base**: from the record or formula, or the number `SetActorValue` put in its
//!   place (`GetBaseActorValue` reads it);
//! - the **permanent modifier**: `ModActorValue` adds to it and `ForceActorValue` moves
//!   it so that the current value is the one asked for (`GetPermanentActorValue` reads
//!   base + this);
//! - the **damage**: `DamageActorValue` adds to it, `RestoreActorValue` takes from it and
//!   never past zero [documented: both pages; observed: Contreras' supplies restoring
//!   100 of a head condition damaged by 40 left it at 100, `L5.quest.effect_start`,
//!   layer5-captures.md §9.3 b]; combat's health and limb damage goes here;
//! - the **temporary modifier**: the Value Modifier effects with the Recover flag active
//!   on the actor (aid items, spells; `crate::aid`, docs/spec-notes/aid-items.md §3).
//!   Worn enchantments do not feed it yet (the Pip-Boy adds them itself).
//!
//! The permanent value (base + permanent modifier) of S.P.E.C.I.A.L. lives in the Layer
//! 4 world, where the carry weight rule and the menus read and write it; the override
//! and the permanent modifier of everything else, and of the attributes of actors the
//! world holds none for (creatures), live here.

use super::{base_of, none, yes, Args, Ctx};
use crate::game::Game;
use fnv_script::vm::Value;
use fnv_sim::forms::SPECIAL;
use fnv_sim::FormId;

fn special(code: u16) -> Option<&'static str> {
    (5..=11).contains(&code).then(|| SPECIAL[(code - 5) as usize])
}

/// The names of the actor value codes 0–76 [documented: xEdit `ActorValue`,
/// `schemas/fnv/common.json`].
pub const AV_NAMES: [&str; 77] = [
    "Aggression", "Confidence", "Energy", "Responsibility", "Mood", "Strength", "Perception", "Endurance",
    "Charisma", "Intelligence", "Agility", "Luck", "Action Points", "Carry Weight", "Critical Chance",
    "Heal Rate", "Health", "Melee Damage", "Damage Resistance", "Poison Resistance", "Rad Resistance",
    "Speed Multiplier", "Fatigue", "Karma", "XP", "Perception Condition", "Endurance Condition",
    "Left Attack Condition", "Right Attack Condition", "Left Mobility Condition", "Right Mobility Condition",
    "Brain Condition", "Barter", "Big Guns", "Energy Weapons", "Explosives", "Lockpick", "Medicine",
    "Melee Weapons", "Repair", "Science", "Guns", "Sneak", "Speech", "Survival", "Unarmed",
    "Inventory Weight", "Paralysis", "Invisibility", "Chameleon", "Night Eye", "Turbo", "Fire Resistance",
    "Water Breathing", "Rad Level", "Bloody Mess", "Unarmed Damage", "Assistance", "Electric Resistance",
    "Frost Resistance", "Energy Resistance", "EMP Resistance", "Variable01", "Variable02", "Variable03",
    "Variable04", "Variable05", "Variable06", "Variable07", "Variable08", "Variable09", "Variable10",
    "Ignore Crippled Limbs", "Dehydration", "Hunger", "Sleep Deprivation", "Damage Threshold",
];

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '_').flat_map(char::to_lowercase).collect()
}

/// An actor value code from its xEdit name (`Perception Condition`), without regard to
/// case and spaces.
pub fn av_by_name(name: &str) -> Option<u16> {
    let want = squash(name);
    AV_NAMES.iter().position(|n| squash(n) == want).map(|i| i as u16)
}

/// Script names of actor values that shipped source never spells (so `encodings.json`
/// has none) and whose xEdit name squashes to something else. Each is its AVIF record's
/// Editor ID without the `AV` prefix [observed: FalloutNV.esm AVIF `AVCritChance`
/// 0x44E, `AVPoisonResist` 0x453, `AVDetectLifeRange` 0x5E1, `AVFireResist` 0x5E2,
/// `AVEnergyResist` 0x5EA; the masters' AVIF FormIDs run in code order]. The original
/// compiler takes that name [observed: `GetAV PoisonResist` compiled and answered 125 in
/// the game, capture abilities-1; hypothesis for the others: every spelling of shipped
/// source is its AVIF Editor ID without `AV`, except Guns (AVSmallGuns) and Survival
/// (AVThrowing)]. Electric, Frost and EMP resistance have no AVIF record in the masters;
/// their names follow the pattern [hypothesis].
const SCRIPT_AV_NAMES: [(u16, &str); 8] = [
    (14, "CritChance"),
    (19, "PoisonResist"),
    (51, "DetectLifeRange"),
    (52, "FireResist"),
    (58, "ElectricResist"),
    (59, "FrostResist"),
    (60, "EnergyResist"),
    (61, "EMPResist"),
];

/// An actor value code from its name: the script name (the decoder's, `encodings.json`,
/// e.g. `NightEye`, `PerceptionCondition`, else [`SCRIPT_AV_NAMES`], e.g. `PoisonResist`)
/// or the xEdit name, either without regard to case and spaces (`Nighteye`, `Night
/// Eye`), or a number.
pub fn av_code(g: &Game, name: &str) -> Result<u16, String> {
    if let Ok(n) = name.parse::<u16>() {
        return Ok(n);
    }
    let want = squash(name);
    if let Some(m) = g.data.table.enums.get("actor_value") {
        if let Some((&c, _)) = m.iter().find(|(_, n)| squash(n) == want) {
            return Ok(c);
        }
    }
    if let Some((c, _)) = SCRIPT_AV_NAMES.iter().find(|(_, n)| squash(n) == want) {
        return Ok(*c);
    }
    AV_NAMES.iter().position(|n| squash(n) == want).map(|i| i as u16).ok_or_else(|| format!("no actor value named {name:?}"))
}

/// The permanent modifier of an actor value (ModActorValue, ForceActorValue).
fn modifier(g: &Game, actor: FormId, code: u16) -> f64 {
    g.extra.get(&actor).and_then(|e| e.av_permanent.get(&code)).copied().unwrap_or(0.0)
}

/// The world's run-time value of an attribute: its permanent value, when it holds one.
fn world_special(g: &Game, actor: FormId, code: u16) -> Option<f64> {
    g.world.peek(actor)?.actor_values.get(special(code)?).map(|v| *v as f64)
}

/// The base value: SetActorValue's number, else the record's or the formula's.
fn base(g: &mut Game, actor: FormId, code: u16) -> Result<f64, String> {
    if let Some(v) = world_special(g, actor, code) {
        return Ok(v - modifier(g, actor, code));
    }
    match g.extra.get(&actor).and_then(|e| e.actor_values.get(&code)).copied() {
        Some(v) => Ok(v),
        None => g.base_actor_value(actor, code),
    }
}

/// The permanent value: the base plus the permanent modifier.
fn permanent(g: &mut Game, actor: FormId, code: u16) -> Result<f64, String> {
    Ok(base(g, actor, code)? + modifier(g, actor, code))
}

/// The permanent value of an actor value, 0 when it cannot be read.
pub(crate) fn actor_value_base(g: &mut Game, actor: FormId, code: u16) -> f64 {
    permanent(g, actor, code).unwrap_or(0.0)
}

/// Add `d` to an actor value's damage part (as DamageActorValue).
pub fn add_damage(g: &mut Game, actor: FormId, code: u16, d: f64) {
    let total = damage(g, actor, code) + d.max(0.0);
    set_damage(g, actor, code, total);
    if code == crate::actor_values::FATIGUE && d > 0.0 {
        g.fatigue_changed(actor);
    }
}

/// The damage an actor value has taken (0 or more).
pub fn damage(g: &Game, actor: FormId, code: u16) -> f64 {
    g.extra.get(&actor).and_then(|e| e.av_damage.get(&code)).copied().unwrap_or(0.0)
}

fn set_damage(g: &mut Game, actor: FormId, code: u16, d: f64) {
    let e = g.extra.entry(actor).or_default();
    if d > 0.0 {
        e.av_damage.insert(code, d);
    } else {
        e.av_damage.remove(&code);
    }
    sync_carry(g, actor, code);
}

/// The world's carry capacity (the menus' weight line, `fnv_sim::World::encumbrance`)
/// follows the actor's current Carry Weight: its formula reads the current Strength,
/// temporary modifiers included (Buffout's +2), and Carry Weight's own modifiers add to
/// it [documented: GECK wiki GetCalculatedMaxCarryWeight, "CarryWeight … Actor Values";
/// hypothesis: the current, not the permanent, Strength; aid-items.md §12]. Called when
/// Strength or Carry Weight changes.
fn sync_carry(g: &mut Game, actor: FormId, code: u16) {
    use crate::actor_values::{CARRY_WEIGHT, STRENGTH};
    if code != STRENGTH && code != CARRY_WEIGHT {
        return;
    }
    let Ok(formula) = g.world.formula_capacity(actor) else { return };
    let Ok(capacity) = actor_value(g, actor, CARRY_WEIGHT) else { return };
    g.world.set_carry_bonus(actor, capacity - formula as f64);
}

/// The temporary modifier of an actor value (active effects).
pub fn temporary(g: &Game, actor: FormId, code: u16) -> f64 {
    g.extra.get(&actor).and_then(|e| e.av_temporary.get(&code)).copied().unwrap_or(0.0)
}

/// Add `d` to the temporary modifier.
pub(crate) fn add_temporary(g: &mut Game, actor: FormId, code: u16, d: f64) {
    let e = g.extra.entry(actor).or_default();
    let m = e.av_temporary.get(&code).copied().unwrap_or(0.0) + d;
    if m.abs() > 1e-9 {
        e.av_temporary.insert(code, m);
    } else {
        e.av_temporary.remove(&code);
    }
    sync_carry(g, actor, code);
}

/// Take up to `amount` (0 or more) off an actor value's damage, as RestoreActorValue.
pub fn restore(g: &mut Game, actor: FormId, code: u16, amount: f64) {
    let d = damage(g, actor, code) - amount.max(0.0);
    set_damage(g, actor, code, d);
}

/// The current value: the permanent value and the temporary modifier, minus the damage.
pub fn actor_value(g: &mut Game, actor: FormId, code: u16) -> Result<f64, String> {
    Ok(permanent(g, actor, code)? + temporary(g, actor, code) - damage(g, actor, code))
}

/// The current value within the range the game calculates with (S.P.E.C.I.A.L. 1–10,
/// skills 0–100), 0 when it cannot be read [documented: GECK wiki
/// GetThresholdedActorValue].
pub fn actor_value_capped(g: &mut Game, actor: FormId, code: u16) -> f64 {
    crate::actor_values::capped(code, actor_value(g, actor, code).unwrap_or(0.0))
}

/// The value's parts: base, permanent modifier, damage (as GetActorValueInfo lists them).
pub fn actor_value_parts(g: &mut Game, actor: FormId, code: u16) -> Result<[f64; 3], String> {
    Ok([base(g, actor, code)?, modifier(g, actor, code), damage(g, actor, code)])
}

/// Put `v` in the base's place (as SetActorValue). An attribute's permanent value goes
/// to the world when the world holds attributes for the actor.
fn set_base(g: &mut Game, actor: FormId, code: u16, v: f64) -> Result<(), String> {
    if let Some(name) = special(code) {
        // Something that has no attributes takes none.
        base(g, actor, code)?;
        if g.world.set_actor_value(actor, name, (v + modifier(g, actor, code)) as f32).is_ok() {
            sync_carry(g, actor, code);
            return Ok(());
        }
    }
    g.extra.entry(actor).or_default().actor_values.insert(code, v);
    sync_carry(g, actor, code);
    Ok(())
}

/// Add `d` to the permanent modifier (as ModActorValue).
pub(crate) fn add_modifier(g: &mut Game, actor: FormId, code: u16, d: f64) -> Result<(), String> {
    let before = permanent(g, actor, code)?;
    let e = g.extra.entry(actor).or_default();
    let m = e.av_permanent.get(&code).copied().unwrap_or(0.0) + d;
    if m != 0.0 {
        e.av_permanent.insert(code, m);
    } else {
        e.av_permanent.remove(&code);
    }
    if let Some(name) = special(code) {
        // The world's number is the permanent value; without one the base stays the
        // record's and the modifier is added on reading.
        let _ = g.world.set_actor_value(actor, name, (before + d) as f32);
    }
    sync_carry(g, actor, code);
    Ok(())
}

/// Move the base by `d`, leaving the modifier and the damage alone.
pub(crate) fn shift_base(g: &mut Game, actor: FormId, code: u16, d: f64) -> Result<(), String> {
    let v = base(g, actor, code)? + d;
    set_base(g, actor, code, v)
}

/// Make the current value exactly `v` with no modifier and no damage (the state a
/// capture save reports: the runner's `given` of kind `actor_value`).
pub fn set_current(g: &mut Game, actor: FormId, code: u16, v: f64) -> Result<(), String> {
    set_damage(g, actor, code, 0.0);
    if let Some(e) = g.extra.get_mut(&actor) {
        e.av_permanent.remove(&code);
    }
    set_base(g, actor, code, v)
}

pub fn get_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    // Authored abilities and worn enchantments are in the modifiers (`crate::abilities`).
    g.sync_actor_effects(r);
    Ok(Value::Num(actor_value(g, r, a.code(0)?)?))
}

/// `GetBaseActorValue`: the base value, without the modifiers and the damage
/// [documented: GECK wiki ActorValue, GetBaseActorValue].
pub fn get_base_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    Ok(Value::Num(base(g, r, a.code(0)?)?))
}

/// `GetPermanentActorValue`: the base value plus the permanent modifier [documented:
/// GECK wiki ActorValue, "the combined base value + the permanent modifier"].
pub fn get_permanent_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    g.sync_actor_effects(r);
    Ok(Value::Num(permanent(g, r, a.code(0)?)?))
}

/// `SetActorValue`: the base value becomes the given number; the modifiers and the
/// damage stay [documented: GECK wiki ActorValue, "this base value can be overridden by
/// the SetActorValue command"].
pub fn set_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    set_base(g, r, a.code(0)?, a.num(1)?)?;
    none()
}

/// `ForceActorValue`: the current value becomes the given number, by changing the
/// permanent modifier by the difference [documented: GECK wiki ForceActorValue, the
/// equivalent ModActorValue; ActorValue, "will alter the permanent modifier"].
pub fn force_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let code = a.code(0)?;
    let diff = a.num(1)? - actor_value(g, r, code)?;
    add_modifier(g, r, code, diff)?;
    none()
}

/// `ModActorValue`: the permanent modifier changes by the amount [documented: GECK wiki
/// ActorValue, the permanent pool].
pub fn mod_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    add_modifier(g, r, a.code(0)?, a.num(1)?)?;
    none()
}

/// `DamageActorValue`: the damage grows by the amount [documented: catalogue summary; a
/// negative amount heals at most the damage taken, hypothesis].
///
/// Rads and Hardcore's needs grow the other way: damage raises them, restoring lowers
/// them, never below 0 [hypothesis: as their aid effects, aid-items.md §3; the Cannibal
/// perk's script restores Hunger 50 with the health it gives].
pub fn damage_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let code = a.code(0)?;
    if crate::aid::grows(code) {
        shift_growing(g, r, code, a.num(1)?)?;
        return none();
    }
    let d = damage(g, r, code) + a.num(1)?;
    set_damage(g, r, code, d);
    // Out of fatigue: knocked out (docs/spec-notes/unconscious.md §2.3).
    if code == crate::actor_values::FATIGUE {
        g.fatigue_changed(r);
    }
    none()
}

/// `RestoreActorValue`: the damage shrinks by the amount, never below zero, so the value
/// never rises past its permanent part [documented: catalogue notes; observed:
/// `L5.quest.effect_start`].
pub fn restore_actor_value(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let code = a.code(0)?;
    if crate::aid::grows(code) {
        shift_growing(g, r, code, -a.num(1)?.max(0.0))?;
        return none();
    }
    let d = damage(g, r, code) - a.num(1)?.max(0.0);
    set_damage(g, r, code, d);
    none()
}

/// Move a value that grows (rads, needs) by `by`, never below 0.
fn shift_growing(g: &mut Game, r: FormId, code: u16, by: f64) -> Result<(), String> {
    let now = actor_value(g, r, code)?;
    let to = (now + by).max(0.0);
    shift_base(g, r, code, to - now)
}

pub fn get_dead(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(g.extra.get(&r).is_some_and(|e| e.dead))
}

/// `actor.KillActor [killer] [limb] [cause]`: the actor dies; its OnDeath blocks run
/// [hypothesis: at once], and the kill counts for GetDeadCount of its base (`Game::kill`,
/// shared with combat deaths). The cause is what GetCauseofDeath then reads [documented:
/// GECK wiki KillActor, "See GetCauseofDeath for death values"]. The limb to dismember is
/// not acted on (no dismemberment yet).
pub fn kill_actor(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    // An essential actor is knocked out "for a few seconds" instead [documented: GECK
    // wiki KillActor notes] (docs/spec-notes/unconscious.md §2.2).
    if essential(g, r) && r != FormId::PLAYER_REF {
        g.knock_out(r, crate::unconscious::Cause::Essential, a.opt_form(0));
        return none();
    }
    if g.kill(r, a.opt_form(0)) {
        if let Ok(cause) = a.int(2) {
            g.extra.entry(r).or_default().death_cause = Some(cause);
        }
    }
    none()
}

/// `actor.DetonatePlacedExplosives`: the actor's placed explosives that detonate (C-4)
/// go off [documented: GECK wiki Projectile, "Detonates"]. The projectiles are the
/// engine's: the request waits in `Game::combat_requests`.
pub fn detonate_placed_explosives(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.on.or(ctx.self_ref).unwrap_or(FormId::PLAYER_REF);
    g.combat_requests.push(crate::combat::CombatRequest::Detonate(r));
    none()
}

/// The body location of the actor's last hit, −1 for none [documented: catalogue notes;
/// explosions give −1].
pub fn get_hit_location(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    Ok(Value::Num(g.extra.get(&r).and_then(|e| e.last_hit).map_or(-1, |h| h.location) as f64))
}

/// 1 while the actor has a weapon out [documented: catalogue summary and notes of
/// IsWeaponOut: the value changes when the weapon is attached to the weapon node]. The
/// state is the animating side's ([`Game::set_weapon_out`]); an actor nobody reported
/// has its weapon away.
///
/// An actor the engine has not told about in the last second (not drawn: unloaded, far,
/// or just gone) reads the rule the engine draws by ([`Game::wants_weapon_out`]), as does
/// a dead one: 0 [hypothesis: a corpse holds nothing out]. [observed: Sunny Smiles at
/// rest read 0 drawn and undrawn, capture `fa_batch2_loaded_npcs`.]
pub fn is_weapon_out(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    if g.is_dead(r) {
        return yes(false);
    }
    let told = g.senses.weapon_told.get(&r).is_some_and(|&t| (g.time - t).abs() < 1.0);
    if r == FormId::PLAYER_REF || told {
        return yes(g.extra.get(&r).is_some_and(|e| e.weapon_out));
    }
    yes(g.wants_weapon_out(r))
}

/// GetWeaponAnimType's value for a WEAP DNAM animation type (the record's numbering).
///
/// [documented: GECK wiki GetWeaponAnimType — the function tells 11 animation types
/// apart and has one catch-all value, several of the 13 (Fallout 3) or 14 (New Vegas)
/// record types sharing a value. The page's table itself is a template we have no copy
/// of.] The numbering here is a [hypothesis] fitted to the shipped scripts
/// (docs/spec-notes/combat-rules.md §14):
/// - the record's types in order starting at 1, the two pistol types sharing 4 and the
///   ballistic and energy rifles sharing 5, which leaves the 11 values 1–11;
/// - Thrown (New Vegas's 14th type) and anything else: the catch-all, 0.
///
/// [observed: `GetWeaponAnimType == 4` is tested together with `GetEquipped
/// NVWeapMS22Camera`, whose record type is 3, Pistol - Ballistic (1 Hand); the weapon
/// alarm scripts test `IsWeaponOut == 1 && GetWeaponAnimType != 1` (raised fists are no
/// alarm) and the tutorial's bottles `> 3 && < 9`, which is then every gun: pistols 4,
/// rifles 5, automatic rifles 6, Handle 7, Launcher 8.] Which types share a value besides
/// the pistols is the unchecked part.
pub fn weapon_anim_code(record_type: i64) -> i64 {
    match record_type {
        0..=2 => record_type + 1,
        3 | 4 => 4,
        5 | 7 => 5,
        6 => 6,
        8..=12 => record_type - 1,
        _ => 0,
    }
}

/// `actor.GetWeaponAnimType`: the animation type of the equipped weapon as
/// [`weapon_anim_code`] numbers it; an actor with no weapon equipped fights with its
/// hands, Hand to Hand, 1 [hypothesis: scripts treat `<= 1` as unarmed and `!= 1` as
/// armed].
pub fn get_weapon_anim_type(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    Ok(Value::Num(g.weapon_anim_type(r) as f64))
}

impl Game<'_> {
    /// What `GetWeaponAnimType` answers for `actor` ([`get_weapon_anim_type`]): the
    /// equipped weapon's type, else that of the weapon the animating side draws for it
    /// (`Game::set_held_weapon`: NPCs' weapons are not equipped in the inventories here).
    pub fn weapon_anim_type(&mut self, actor: FormId) -> i64 {
        let held = self.extra.get(&actor).and_then(|e| e.held_weapon);
        let record_type = super::items::equipped_weapon(self, actor).or(held).and_then(|w| self.data.weapon_anim_types.get(&w).copied());
        record_type.map_or(1, weapon_anim_code)
    }
}

/// `actor.ResurrectActor [anim]`: the actor comes back "from the dead or unconscious
/// state" [documented: GECK wiki ResurrectActor], generated again from its record
/// (`crate::resurrect`, docs/spec-notes/resurrect.md). `anim` 1 has it stand up, 0 (or
/// none) just standing [documented: the wiki's parameter notes].
pub fn resurrect_actor(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let animate = a.int(0).is_ok_and(|v| v != 0);
    g.resurrect(r, animate);
    none()
}

pub fn get_dead_count(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let b = a.form(0)?;
    Ok(Value::Num(g.player.dead_count.get(&b).copied().unwrap_or(0) as f64))
}

/// The factions and ranks the records give an actor: its base's `SNAM` list, or its
/// template's with the template flag "Factions" (docs/spec-notes/factions.md §2).
fn base_factions<'d>(g: &'d Game, r: FormId) -> &'d [(FormId, i8)] {
    base_of(g, r).map_or(&[], |b| g.data.base_factions(b, r))
}

/// Faction rank of an actor: a run-time change, else the records' ([`base_factions`]);
/// −1 when not a member.
pub fn faction_rank(g: &Game, r: FormId, f: FormId) -> i32 {
    if let Some(&rank) = g.extra.get(&r).and_then(|e| e.factions.get(&f)) {
        return rank as i32;
    }
    base_factions(g, r).iter().find(|(x, _)| *x == f).map(|(_, rank)| *rank as i32).unwrap_or(-1)
}

pub fn get_in_faction(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(faction_rank(g, r, a.form(0)?) >= 0)
}

pub fn get_faction_rank(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    Ok(Value::Num(faction_rank(g, r, a.form(0)?) as f64))
}

/// `AddToFaction` / `SetFactionRank`: the actor reference joins at the rank.
pub fn add_to_faction(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let f = a.form(0)?;
    let rank = a.num_or(1, 0.0) as i8;
    g.extra.entry(r).or_default().factions.insert(f, rank);
    none()
}

pub fn remove_from_faction(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    g.extra.entry(r).or_default().factions.insert(a.form(0)?, -1);
    none()
}

/// The voice type can be given directly or as a form list of voice types.
pub fn get_is_voice_type(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let v = a.form(0)?;
    let Some(voice) = g.traits_def(r).and_then(|d| d.voice) else { return yes(false) };
    yes(voice == v || super::refs::list_members(g, v).contains(&voice))
}

fn is_female(g: &Game, r: FormId) -> bool {
    if r == FormId::PLAYER_REF {
        if let Some(f) = g.player.female {
            return f;
        }
    }
    g.traits_def(r).is_some_and(|a| a.female)
}

/// Sex codes: 0 male, 1 female (scripts.md §2.4).
pub fn get_is_sex(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(is_female(g, r) == (a.code(0)? == 1))
}

pub fn get_pc_is_sex(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    yes(is_female(g, FormId::PLAYER_REF) == (a.code(0)? == 1))
}

/// `SexChange [sex]`: without a calling reference it changes the player; without a sex
/// it flips [documented: catalogue summary]. Other actors: not modelled.
pub fn sex_change(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject_or_player();
    if r != FormId::PLAYER_REF {
        return Err("SexChange on an actor other than the player is not modelled".into());
    }
    let female = match a.code(0) {
        Ok(c) => c == 1,
        Err(_) => !is_female(g, r),
    };
    g.player.female = Some(female);
    none()
}

pub fn get_is_race(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    // The race chosen in the character creator stands for the record's.
    let chosen = g.player.look.as_ref().filter(|_| r == FormId::PLAYER_REF).map(|l| l.race);
    yes(chosen.or_else(|| g.traits_def(r).and_then(|d| d.race)) == Some(a.form(0)?))
}

pub fn get_is_class(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(g.traits_def(r).and_then(|d| d.class) == Some(a.form(0)?))
}

macro_rules! flag {
    ($set:ident, $get:ident, $field:ident) => {
        pub fn $set(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
            let r = ctx.subject()?;
            g.extra.entry(r).or_default().$field = a.num_or(0, 1.0) != 0.0;
            none()
        }
        #[allow(dead_code)]
        pub fn $get(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
            let r = ctx.subject()?;
            yes(g.extra.get(&r).is_some_and(|e| e.$field))
        }
    };
}

flag!(set_ghost, get_ghost, ghost);
flag!(set_restrained, get_restrained, restrained);
flag!(set_unconscious, get_unconscious, unconscious);
flag!(set_player_teammate, get_player_teammate, teammate);

/// 1 for an actor whose race has the Child flag (RACE DATA) [documented: catalogue
/// summary of IsChild].
pub fn is_child(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(g.traits_def(r).and_then(|d| d.race).is_some_and(|race| g.data.child_races.contains(&race)))
}

/// 1 for a creature of the given type (CREA DATA type) [documented: catalogue summary];
/// 0 for NPCs.
pub fn get_is_creature_type(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let t = a.code(0)? as i64;
    // The type sits in CREA DATA with the stats [hypothesis: it follows the "Stats" flag].
    yes(g.stats_def(r).and_then(|d| d.creature_type) == Some(t))
}

/// Essential: the reference's own flag (SetActorRefEssential) or its base's (ACBS,
/// changed by SetEssential). The reference flag cannot clear the base's [documented:
/// catalogue notes of SetActorRefEssential].
pub fn essential(g: &Game, r: FormId) -> bool {
    let base = base_of(g, r);
    let base_flag = base
        .and_then(|b| g.forms.essential.get(&b).copied())
        .unwrap_or_else(|| g.base_data_def(r).is_some_and(|d| d.essential));
    base_flag || g.extra.get(&r).and_then(|e| e.essential).unwrap_or(false)
}

/// `SetEssential base flag`: the base form's flag, for every reference [documented:
/// catalogue summary].
pub fn set_essential(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let b = a.form(0)?;
    let b = base_of(g, b).filter(|_| g.world.placed.get(b).is_some()).unwrap_or(b);
    g.forms.essential.insert(b, a.num_or(1, 1.0) != 0.0);
    none()
}

pub fn set_actor_ref_essential(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    g.extra.entry(r).or_default().essential = Some(a.num_or(0, 1.0) != 0.0);
    none()
}

pub fn is_essential(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(essential(g, r))
}

/// `actor.SetActorFullName message`: the base's name becomes the message's title, for
/// every reference [documented: catalogue notes]; kept for the engine's name display.
pub fn set_actor_full_name(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let m = a.form(0)?;
    let b = base_of(g, r).unwrap_or(r);
    g.forms.names.insert(b, m);
    none()
}

/// 1 when the given actor killed the caller (KillActor's killer) [documented:
/// catalogue summary].
pub fn is_killer(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let k = a.form(0)?;
    yes(g.extra.get(&r).and_then(|e| e.killer) == Some(k))
}

/// The factions an actor is in now: the records' ([`base_factions`]) unless removed, and
/// those added at run time.
pub fn factions_of(g: &Game, r: FormId) -> Vec<FormId> {
    let mut out: Vec<FormId> = base_factions(g, r).iter().map(|f| f.0).collect();
    if let Some(e) = g.extra.get(&r) {
        out.extend(e.factions.keys().copied());
    }
    out.sort();
    out.dedup();
    out.retain(|&f| faction_rank(g, r, f) >= 0);
    out
}

/// The group combat reaction of faction `a` towards `b`: a run-time SetEnemy/SetAlly,
/// else FACT XNAM, else neutral (0) [documented: catalogue notes of
/// GetFactionCombatReaction: 0 neutral for unlisted pairs, 1 enemy, 2 ally, 3 friend].
pub fn reaction(g: &Game, a: FormId, b: FormId) -> i64 {
    if let Some(&v) = g.forms.faction_reactions.get(&a).and_then(|m| m.get(&b)) {
        return v;
    }
    g.data.faction_reactions.get(&a).and_then(|l| l.iter().find(|x| x.0 == b)).map(|x| x.1).unwrap_or(0)
}

pub fn get_faction_combat_reaction(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    Ok(Value::Num(reaction(g, a.form(0)?, a.form(1)?) as f64))
}

/// How actor `r` stands towards actor `o` by their factions (0 neutral, 1 enemy, 2 ally,
/// 3 friend) [documented: GECK wiki GetFactionRelation]. One direction: the reactions
/// of `r`'s factions towards `o`'s, which need not be those of `o`'s towards `r`'s
/// [documented: GECK wiki SetEnemy, each faction has its own reaction list;
/// docs/spec-notes/factions.md §4]. How several factions combine is a [hypothesis]:
/// enemy first, then friend, then ally; the player counts as an enemy of an actor in a
/// faction the player is a crime enemy of.
pub fn faction_relation(g: &Game, r: FormId, o: FormId) -> i64 {
    factions_relation(g, &factions_of(g, r), o)
}

/// How the factions `mine` together stand towards actor `o`, combined as
/// [`faction_relation`] does (0 neutral, 1 enemy, 2 ally, 3 friend). A media location
/// controller's conditional faction asks this of the player (docs/spec-notes/audio.md §13).
pub fn factions_relation(g: &Game, mine: &[FormId], o: FormId) -> i64 {
    if o == FormId::PLAYER_REF && mine.iter().any(|f| g.player.enemy_of.contains(f)) {
        return 1;
    }
    let theirs = factions_of(g, o);
    let all: Vec<i64> = mine.iter().flat_map(|&x| theirs.iter().map(move |&y| (x, y))).map(|(x, y)| reaction(g, x, y)).collect();
    if all.contains(&1) {
        1
    } else if all.contains(&3) {
        3
    } else if all.contains(&2) {
        2
    } else {
        0
    }
}

/// `actor.GetFactionRelation other` ([`faction_relation`]).
pub fn get_faction_relation(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    Ok(Value::Num(faction_relation(g, r, a.form(0)?) as f64))
}

/// `SetEnemy f1 f2 [flag12 flag21]`: each direction becomes enemy (flag 0, the default)
/// or neutral (1) [documented: catalogue notes].
pub fn set_enemy(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let (f1, f2) = (a.form(0)?, a.form(1)?);
    let v = |x: f64| if x != 0.0 { 0 } else { 1 };
    let (r12, r21) = (v(a.num_or(2, 0.0)), v(a.num_or(3, 0.0)));
    g.forms.faction_reactions.entry(f1).or_default().insert(f2, r12);
    g.forms.faction_reactions.entry(f2).or_default().insert(f1, r21);
    none()
}

/// `SetAlly f1 f2 [flag12 flag21]`: each direction becomes ally (flag 0, the default)
/// or friend (1) [documented: catalogue notes].
pub fn set_ally(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let (f1, f2) = (a.form(0)?, a.form(1)?);
    let v = |x: f64| if x != 0.0 { 3 } else { 2 };
    let (r12, r21) = (v(a.num_or(2, 0.0)), v(a.num_or(3, 0.0)));
    g.forms.faction_reactions.entry(f1).or_default().insert(f2, r12);
    g.forms.faction_reactions.entry(f2).or_default().insert(f1, r21);
    none()
}

/// The actor leaves every faction it is in [documented: catalogue summary].
pub fn remove_from_all_factions(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    for f in factions_of(g, r) {
        g.extra.entry(r).or_default().factions.insert(f, -1);
    }
    none()
}

/// The caller's rank minus the target's in a faction [documented: catalogue summary];
/// a non-member's rank counts as −1 [hypothesis].
pub fn get_faction_rank_difference(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let f = a.form(0)?;
    let t = a.form(1)?;
    Ok(Value::Num((faction_rank(g, r, f) - faction_rank(g, t, f)) as f64))
}

/// `SetPCEnemyOfFaction faction flag` [documented: catalogue summary].
pub fn set_pc_enemy_of_faction(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let f = a.form(0)?;
    if a.num_or(1, 1.0) != 0.0 {
        g.player.enemy_of.insert(f);
    } else {
        g.player.enemy_of.remove(&f);
    }
    none()
}

pub fn get_pc_enemy_of_faction(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    yes(g.player.enemy_of.contains(&a.form(0)?))
}

pub fn clear_faction_player_enemy_flag(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    g.player.enemy_of.remove(&a.form(0)?);
    none()
}

/// 1 when the player is a crime enemy of any faction the caller is in [documented:
/// catalogue summary].
pub fn get_actor_faction_player_enemy(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(factions_of(g, r).iter().any(|f| g.player.enemy_of.contains(f)))
}

flag!(ignore_crime, get_ignore_crime, ignore_crime);
flag!(set_ignore_friendly_hits, get_ignore_friendly_hits, ignore_friendly_hits);

/// `actor.SetLevel level [levelToPC min max]`: the player's level, or an actor's
/// (with the level-to-player flag the level is an offset from the player's, kept within
/// min and max when given) [documented: catalogue summary and notes]. Re-rolling an
/// actor's inventory and the player's stats are not modelled.
pub fn set_level(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject_or_player();
    let mut level = a.int(0)?;
    if r == FormId::PLAYER_REF {
        g.player.level = Some(level);
        return none();
    }
    if a.num_or(1, 0.0) != 0.0 {
        level += g.player.level.unwrap_or(g.world.config().player_level as i32);
        if let Ok(min) = a.int(2) {
            level = level.max(min);
        }
        if let Some(max) = a.int(3).ok().filter(|&m| m > 0) {
            level = level.min(max);
        }
    }
    g.extra.entry(r).or_default().level = Some(level);
    none()
}

/// How many actors are the player's teammates now [documented: catalogue summary].
pub fn get_player_teammate_count(g: &mut Game, _ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let n = g.extra.iter().filter(|(_, e)| e.teammate && !e.dead).count();
    Ok(Value::Num(n as f64))
}

/// `ResetHealth`: the actor's health is restored and its crippled limbs cured: the
/// damage of Health and of the limb conditions goes [documented: GECK wiki ResetHealth].
pub fn reset_health(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    if let Some(e) = g.extra.get_mut(&r) {
        e.av_damage.remove(&crate::combat::HEALTH);
        for code in crate::combat::LIMB_CONDITIONS {
            e.av_damage.remove(&code);
        }
    }
    none()
}

/// Recorded on the actor and made its current package (the package runs in Layer 6);
/// also traced as a stub call.
pub fn add_script_package(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let p = a.form(0)?;
    // An actor holds one script package: a second call replaces the first
    // [documented: catalogue notes on AddScriptPackage, actor-behaviour.md §1.4].
    g.extra.entry(r).or_default().packages = vec![p];
    if g.runs_packages() {
        g.set_package(r, Some(p));
    } else {
        g.extra.entry(r).or_default().current_package = Some(p);
    }
    g.stub_note(ctx, a.0);
    none()
}

/// The script package is dropped; the actor chooses again when its package is next
/// asked for.
pub fn remove_script_package(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    if let Some(e) = g.extra.get_mut(&r) {
        e.packages.pop();
        if !g.runs_packages() {
            g.extra.entry(r).or_default().current_package = None;
        } else {
            g.evaluate_package(r);
        }
    }
    g.stub_note(ctx, a.0);
    none()
}

/// `EvaluatePackage`: see [`Game::evaluate_package`].
pub fn evaluate_package(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    g.evaluate_package(r);
    none()
}

/// 1 when the actor's current package is the given one [documented: catalogue summary].
pub fn get_is_current_package(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    let p = a.form(0)?;
    yes(g.current_package(r) == Some(p))
}

/// `GetActorValue Health` over `GetBaseActorValue Health`, 1.0 when the base is 0, with
/// no upper bound (effects raising Health give more than 1) [documented: GECK wiki
/// GetHealthPercentage notes, the code the function is equivalent to]; a dead actor is at
/// 0 [hypothesis: KillActor leaves its health as it was here].
pub fn get_health_percentage(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    if g.extra.get(&r).is_some_and(|e| e.dead) {
        return Ok(Value::Num(0.0));
    }
    let base = base(g, r, crate::combat::HEALTH)?;
    let now = actor_value(g, r, crate::combat::HEALTH)?;
    Ok(Value::Num(if base != 0.0 { (now / base).max(0.0) } else { 1.0 }))
}

/// IsFacingUp (bipeds) and IsLeftUp (quadrupeds), the idle manager's checks of a knocked
/// down actor's side [documented: GECK wiki]: 1 for a standing actor [observed, function
/// probes `fp1_queries` 2026-10-17: both read 1 on Ringo standing in the gas station];
/// for a knocked-out one the side the engine saw its body come to rest on, 1 until then
/// (docs/spec-notes/unconscious.md §2.8) [hypothesis: IsLeftUp reads the same side].
pub fn knocked_side_up(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let r = ctx.subject()?;
    yes(g.knockout(r).and_then(|k| k.facing_up).unwrap_or(true))
}

/// `GetOwnerLastTarget`: "the most recent target of whoever owns the calling reference"
/// [documented: catalogue summary]: the actor the calling item's holder (an item
/// script's instance), or the calling actor, last hit (`RefExtra::last_target`), else
/// its combat target [hypothesis: the hit target is the target meant; the weapon scripts
/// that call it run in OnHit, docs/spec-notes/unconscious.md §1].
pub fn get_owner_last_target(g: &mut Game, ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    let owner = match ctx.inst {
        Some(crate::game::InstKey::Item { holder, .. }) => Some(holder),
        _ => ctx.on.or(ctx.self_ref),
    };
    let Some(owner) = owner else { return Ok(Value::NONE) };
    let target = g.extra.get(&owner).and_then(|e| e.last_target).or_else(|| g.awareness.get(owner).and_then(|a| a.combat_target()).map(|t| t.0));
    Ok(target.map_or(Value::NONE, |t| Value::Ref(t.0)))
}

/// `actor.GetDisposition target`: the actor's record's Disposition Base (ACBS) plus
/// `fDispTargetCharMult` × (the target's Charisma − 5), kept within 0 to 100; a value
/// SetDisposition gave for that target instead [observed, function probes 2026-10-17
/// (`fp1_queries`, `fp5_actions`): every Goodsprings resident (base 35) read 35 towards a
/// player of Charisma 5, 85 at Charisma 10 and 0 at Charisma 1; Ringo towards Sunny Smiles
/// (Charisma 4) 25; Karma ±1000 changed nothing; after `SetDisposition player 20` it read
/// 20. The GMST is 10 in FalloutNV.esm and the slope observed is 10: that the GMST is the
/// factor, the anchor 5 and the upper bound 100 are hypothesis]. The wiki's "all
/// modifiers" (factions, crimes, the actor's own Charisma via `fDispActorCharMult`) are
/// not applied: the actor's Charisma (3 to 6 among those probed) made no difference.
pub fn get_disposition(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let actor = ctx.subject()?;
    let target = a.form(0)?;
    if let Some(v) = g.extra.get(&actor).and_then(|x| x.dispositions.get(&target)) {
        return Ok(Value::Num(*v));
    }
    let base = g.base_data_def(actor).map_or(0, |d| d.disposition_base) as f64;
    let cha = crate::actor_values::capped(crate::actor_values::CHARISMA, actor_value(g, target, crate::actor_values::CHARISMA).unwrap_or(5.0));
    let mult = crate::combat::setting(g, "fDispTargetCharMult", 10.0);
    Ok(Value::Num((base + mult * (cha - 5.0)).clamp(0.0, 100.0).round()))
}

/// `actor.SetDisposition target value`: GetDisposition towards `target` reads `value`
/// from now on [documented: GECK wiki SetDisposition; observed: 20 after
/// `SetDisposition player 20`]. Kept in the saved state.
pub fn set_disposition(g: &mut Game, ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let actor = ctx.subject()?;
    let target = a.form(0)?;
    let v = a.num(1)?;
    g.extra.entry(actor).or_default().dispositions.insert(target, v);
    none()
}
