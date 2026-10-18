//! The player's attacks with ranged weapons (docs/design/combat.md step C4), on the
//! headless rules of `fnv-combat` (docs/spec-notes/combat-rules.md):
//!
//! - **Weapon and ammo.** The equipped weapon (simulation) and its loaded ammo: the
//!   first of the weapon's ammo list the player carries (`CombatForms::ammo_for`). The
//!   magazine is part of the carried count, as rounds in the inventory; a shot removes
//!   `ammo_use` rounds from the inventory. A weapon with no ammo of its own but a
//!   projectile (grenades, thrown weapons) spends itself. A newly equipped weapon starts
//!   loaded with what is carried [design].
//! - **The cycle** (`fnv_combat::attack::Cycle`): Use is the trigger, a Ready Item tap
//!   reloads, Ammo Swap moves down the ammo list and reloads [design]. It runs only once
//!   the player body says the weapon is out ([`WeaponAnim::ready`]); the body plays the
//!   attack and reload clips when [`WeaponAnim::attacks`] / [`WeaponAnim::reloads`] count
//!   up and gives their timing back.
//! - **Shots** leave the eye along the view, inside the spread cone (the documented New
//!   Vegas spread plus bloom); recoil kicks the view up and recovers [design]. They are
//!   drawn from the muzzle of the held weapon ([`WeaponAnim::muzzle`], the model's
//!   `ProjectileNode`), closing on the view ray by the point aimed at; in third person a
//!   shot with something between the muzzle and that point leaves the muzzle instead
//!   (`fnv_combat::shot::leaving`; docs/design/combat.md §5.1).
//! - **Projectiles** fly through the physics world (`fnv_phys` rays; actor hit volumes,
//!   combat C3).
//! - **Melee and unarmed** (step C6, `fnv_combat::melee`): a tap of Use swings, and taps
//!   during a swing chain the next at the clip's `a:` key (right, left, right… by the
//!   keys); holding it [`CombatTuning::power_hold`] makes a power attack whose clip
//!   follows the movement keys (standing, forward, back, left, right) and whose damage is
//!   that direction's `fDamagePowerAttack…Bonus`. The body plays [`WeaponAnim::swing`]
//!   and times it ([`WeaponAnim::timed`]); the swing strikes once at the clip's `Hit` key
//!   [documented: NiTextKeyExtraData] with a fan of rays from the eye out to the reach
//!   (`damage::melee_reach`), and at any actor whose centre is within that reach across
//!   the ground, low or high (`projectile::swing_reaching`: a gecko at the player's feet). With no weapon equipped the player fights with the
//!   Fists (WEAP `Fists`) [hypothesis: the game's unarmed weapon form]. Knockback force
//!   is computed (documented formula) and logged; living actors are not pushed until they
//!   move physically. Every shot and blow that lands on an actor also sends an
//!   [`ActorPush`], which shoves a dead actor's ragdoll [design].
//! - **Hits** (step C5): the damage of `fnv_combat::damage` against the target's worn
//!   armour (`CombatForms::worn_armor` over its inventory) goes to the script runtime's
//!   `Game::hit` (Health's damage part, OnHit/OnHitWith, death and OnDeath, kill XP), so
//!   scripts see what combat did. Explosions hit with location −1. Each hit sends an
//!   [`ActorHit`] (nvforever plays a hit reaction, when the hit cripples a body part or
//!   lands on a crippled one, or the death pose). Without the script runtime
//!   (`NVF_PLAY_SCRIPTS=0`) hits are only logged.
//! - **Limb hits** (step C8): an actor whose hit volume has body parts
//!   (`crate::hitvol::ActorParts`) is hit on a part of its BPTD: the part's damage
//!   multiplier, its location for `GetHitLocation`, and limb damage (the damage dealt
//!   times the weapon's Limb Dmg Mult) on its condition (`Game::hit`), which cripples the
//!   limb at 0. Explosions hit no part (location −1) [design]. The player's crippled limbs
//!   ([`PlayerLimbs`]) widen the spread (the documented crippled-arm terms), weaken melee
//!   (`Attacker::arm_condition`) and slow movement [design]; the HUD lists them.
//! - **Explosives and destructibles** (step C7): grenades and other thrown weapons are
//!   lobbed projectiles; mines arm and wait for an actor; C-4 waits for
//!   `DetonatePlacedExplosives`, which the detonator's OnFire script calls (weapons' OnFire
//!   blocks run on each shot). Explosions hurt actors (armour, knockdown by the EXPL flags
//!   and the documented formula, logged), damage destructible objects in their radius and
//!   push loose objects. Bullets damage destructibles they hit. Destruction stages
//!   (`Game::damage_object`, as `DamageObject`) disable, destroy, burn down by their
//!   self-damage and set off their explosions, which can set off others.
//! - **Shots on objects** (combat-rules.md §14): a shot that lands on a placed object
//!   that is not an actor runs the object's OnHit and OnHitWith blocks
//!   (`Game::hit_object`: the tutorial's bottles count the player's shots there) and
//!   strikes its loose body with the projectile's impact force ([`OBJECT_PUSH`]), so
//!   bottles and cans are knocked away [design]. The scripts are told when the player's
//!   weapon is out (`IsWeaponOut`): [`WeaponAnim::ready`].
//! - **What is drawn of a shot** (docs/spec-notes/combat-rules.md §15): each shot sends
//!   a [`ShotFired`] (the renderer shows the PROJ's muzzle flash) and every projectile
//!   in flight is listed in [`ShotVisuals`] (the renderer draws the PROJ's model there).
//!   A hitscan bullet, whose hit is found at once, is drawn flying from the muzzle to
//!   where it lands at the PROJ's speed; a beam is drawn from the muzzle to its end for
//!   the PROJ's fade duration [design].
//! - **Sounds and impacts** (docs/spec-notes/combat-rules.md §16): each shot plays the
//!   weapon's Shoot 2D sound (else its Shoot 3D), an empty trigger its No Ammo sound, and
//!   drawing and putting it away its Equip and Unequip sounds (`crate::sfx`). A shot
//!   that lands sends an [`ImpactFx`] with the IPCT its weapon's impact data set names
//!   for the surface's material, and plays that impact's sounds there; an explosion
//!   sends an [`ExplosionFx`] and plays the EXPL's sounds.
//! - **Item condition** (docs/spec-notes/combat-rules.md §17, `fnv_combat::condition`):
//!   each shot, and each swing that lands, wears the player's weapon; a hit wears the
//!   armour the target wears where it lands (head or body; an explosion both); a hit on
//!   the weapon an actor holds ([`crate::hitvol::WEAPON_PART`]) takes its damage from
//!   that weapon's condition and none from the actor. The player's weapon at zero is
//!   unequipped and stays in the inventory, broken; another actor's falls to the ground
//!   ([`WeaponBroken`]). A worn weapon hits for less and worn armour protects less.
//! - **Other actors' attacks** (combat AI, docs/design/combat-ai.md): nvforever's
//!   `npc_combat` decides when an actor fires or strikes and sends [`ActorFire`] or
//!   [`ActorStrike`]; [`actor_attacks`] turns them into projectiles and hits through the
//!   same rules as the player's. Every projectile knows who fired it, with which weapon
//!   and ammunition ([`By`]), so its hit is that actor's. Shots fired by other actors
//!   can hit the player's character [design: the player has no hit volume of its own;
//!   a hit counts as the torso]. Actors fire without spending ammunition [design: until
//!   looting balance is looked at].
//! - **The player's death**: at 0 health the player dies (`Game::hit`); every control
//!   is held from then on (`ScriptState::locks`) and the screen says so; the death
//!   camera (`nvforever` `death_cam.rs`) drops the body, slows the clock and blurs the
//!   picture [design: reloading the last save is not built]. God mode ([`god_mode`]:
//!   `--god`, the settings page's Developer switch, test aid `NVF_PLAY_TEST_GOD=1`): the
//!   player takes no damage, so a long unattended run is not ended by a fight.
//! - **Feedback** [design]: a hit marker at the
//!   crosshair (red on a kill), the last target's health for a few seconds, the player's
//!   crippled limbs, and a line for kills with their XP. Health, condition and ammo are
//!   on the HUD proper (`crate::hud`, [`Combat::hud_weapon`]). Developer markers, off
//!   unless [`HitMarks`] says so: a cross at each impact and a line from the muzzle
//!   along each hitscan shot.

use bevy::prelude::*;
use fnv_combat::attack::{ClipTiming, Cycle, Event, Input, Timing};
use fnv_combat::condition::{self, Wear, Zone};
use fnv_combat::damage::{self, Attacker, AttackKind, HitInput, HitPart, NoPerks, Target};
use fnv_combat::melee::{Melee, MeleeEvent, MeleeInput, SwingTiming};
use fnv_combat::data::AmmoEffect;
use fnv_combat::projectile::{blast, ActorId, CombatWorld, Motion, ProjEvent, Projectile, WorldHit};
use fnv_combat::rng::SplitMix;
use fnv_combat::shot::Seen;
use fnv_combat::spread::{self, Kick, Stance};
use fnv_combat::vec::{self, V3};
use fnv_combat::{shot, CombatForms, CombatTuning, Rules};
use fnv_phys::{layers, Filter, Key, Owner, Queries};
use fnv_sim::FormId;
use std::collections::HashMap;

use crate::activate::HudMessage;
use crate::sfx::{LoopSound, PlaySound, Sfx};
use crate::hitvol::{ActorParts, WEAPON_PART};
use fnv_combat::data::{BodyPart, WeaponStats};
use fnv_game::combat::LimbHit;
use crate::control::PlayerControl;
use crate::script::{with_world, ScriptState};
use fnv_game::combat::{CombatRequest, DestructionChange, HitInfo, HitResult};
use crate::phys::PhysState;
use crate::player::{MoveIntent, Player, MAX_PITCH};
use crate::sim::SimState;
use crate::{space, PlaySet, PlayState, PlayWorld};

mod hit_effects;

/// The shooter's id for projectiles (the player has no hit volume).
const PLAYER: ActorId = u64::MAX;

/// What the player body and the combat systems tell each other.
#[derive(Resource, Default, Debug, Clone)]
pub struct WeaponAnim {
    /// Set by the body: the weapon is drawn and in hand.
    pub ready: bool,
    /// Set by combat: the weapon's attack and reload animation names (`AttackRight`,
    /// `ReloadL`), for the body's clip choice.
    pub attack: Option<String>,
    pub reload: Option<String>,
    /// Set by the body: the attack and reload clips' timing in the rig in view.
    pub timing: ClipTiming,
    /// Counted up by combat when an attack or a reload starts; the body restarts the
    /// clip when it sees a new count.
    pub attacks: u32,
    pub reloads: u32,
    /// Counted up by combat when a round-by-round reload starts on a round: the body
    /// plays the reload clip from its start (a reload count plays the opening clip).
    pub reload_rounds: u32,
    /// Counted up by combat when a weapon starts to spin up: the body plays the attack
    /// clip from its start, and the first shot follows at the clip's `Fire` key.
    pub spin_ups: u32,
    /// Set by combat: the weapon's jam animation (`JamA` for reload animation
    /// `ReloadA`: the game's `1hpjama.kf` beside `1hpreloada.kf` [observed: file lists]),
    /// and counted up when a shot jams the weapon; the body plays the jam clip.
    pub jam: Option<String>,
    pub jams: u32,
    /// Set by combat: the clip stem of the melee swing playing (`attackleft_b`,
    /// `attackforwardpower`; `fnv_combat::melee`), for the body's clip choice; `None`:
    /// the weapon's attack animation.
    pub swing: Option<String>,
    /// Set by the body: the attack stem whose clip [`Self::timing`] was read from.
    pub timed: Option<String>,
    /// Set by the body each frame: where the muzzle of the weapon in hand is, in the rig
    /// in view (game space): the weapon model's `ProjectileNode` [hypothesis: that the
    /// node marks where projectiles and the muzzle flash start, from its name and its
    /// place at the end of the barrel; docs/spec-notes/animation.md §9.9]. `None`
    /// without a body, a weapon in hand or such a node: shots are drawn from the eye.
    pub muzzle: Option<[f32; 3]>,
    /// Set by the body each frame: where the weapon's `ShellCasingNode` is in the rig
    /// in view and the way a casing leaves it (unit), game space.
    pub casing: Option<([f32; 3], [f32; 3])>,
}

/// The player fired a shot this frame: the PROJ it fires. The renderer shows the
/// PROJ's muzzle flash at the weapon's muzzle (nvforever `shot_fx`).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShotFired {
    pub projectile: FormId,
    /// The weapon that fired it (nvforever `detection` hears its sound level).
    pub weapon: FormId,
    /// Who fired it.
    pub shooter: FormId,
    /// Where it left, for a shot of an actor other than the player (the player's
    /// flash is at the held weapon's muzzle).
    pub at: Option<V3>,
}

/// An actor other than the player fires its weapon once (combat AI, nvforever
/// `npc_combat`): aimed from its eyes (`origin`) along `aim` (a unit vector), inside a
/// cone of `cone` degrees (its spread), drawn from its weapon's `muzzle` when it has one
/// (as the player's first-person shots, `shot::leaving` without cover: the shot hits
/// what the eyes see, so an actor fires over low cover its barrel is behind) [design].
#[derive(Message, Clone, Copy, Debug)]
pub struct ActorFire {
    pub shooter: FormId,
    pub weapon: FormId,
    pub origin: V3,
    pub aim: V3,
    pub cone: f32,
    pub muzzle: Option<V3>,
    /// The shot does no damage (a weapon use's Do No Damage, nvforever `use_weapon`):
    /// it flies, strikes and is seen and heard as any other.
    pub harmless: bool,
}

/// An actor other than the player strikes `target` within its reach (combat AI): with
/// its melee weapon, else unarmed (the Fists), or, a creature, with its own attack
/// damage (CREA DATA Damage).
#[derive(Message, Clone, Copy, Debug)]
pub struct ActorStrike {
    pub attacker: FormId,
    pub target: FormId,
    pub weapon: Option<FormId>,
    pub creature_damage: Option<f32>,
    /// Where the blow lands and which way it goes (game space).
    pub point: V3,
    pub dir: V3,
    /// A power attack's attack group ("AttackForwardPower"), for an unarmed attack effect
    /// that names it; `None`: an ordinary blow.
    pub anim: Option<&'static str>,
}

/// A melee or unarmed blow a blocker stopped part of (docs/design/combat-ai.md §14.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockedBlow {
    pub attacker: FormId,
    pub blocker: FormId,
    /// Damage before and after the block.
    pub damage: f32,
    pub dealt: f32,
}

/// Who fired a projectile: the actor, its weapon and the ammunition it was loaded with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct By {
    pub actor: FormId,
    pub weapon: FormId,
    pub ammo: Option<FormId>,
    /// It does no damage ([`ActorFire::harmless`]).
    pub harmless: bool,
    /// A marked shot of VATS mode (`crate::vats_mode`): its hit counts as a VATS attack
    /// (the critical chance bonus, docs/design/vats-mode.md §9.2).
    pub vats: bool,
}

/// One projectile to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotVisual {
    /// The same number for as long as the projectile is drawn.
    pub id: u64,
    /// Its PROJ.
    pub projectile: FormId,
    /// Where it is drawn (game space) and the way it points (unit).
    pub pos: V3,
    pub dir: V3,
    /// A beam: its length from `pos` along `dir`.
    pub beam: Option<f32>,
    /// How far it has flown since it left.
    pub travelled: f32,
}

/// The player's projectiles to draw this frame (nvforever `shot_fx` draws their models).
#[derive(Resource, Default)]
pub struct ShotVisuals {
    pub flying: Vec<ShotVisual>,
}

/// A shot threw out a shell casing: the weapon, where the casing starts and its
/// velocity (game space, units per second). The renderer draws the WEAP's casing model.
#[derive(Message, Clone, Copy, Debug)]
pub struct CasingFx {
    pub weapon: FormId,
    pub point: V3,
    pub velocity: V3,
    /// Seconds the casing stays (`fGunShellLifetime`).
    pub life: f32,
}

/// A shot landed: the IPCT of the weapon's impact data set for what it hit, where, the
/// surface's normal and the way the projectile was flying (game space). The renderer
/// shows the impact's effect (nvforever `shot_fx`).
#[derive(Message, Clone, Copy, Debug)]
pub struct ImpactFx {
    pub impact: FormId,
    /// Who fired the projectile.
    pub shooter: FormId,
    pub point: V3,
    pub normal: V3,
    pub dir: V3,
}

/// An explosion went off: its EXPL and where.
#[derive(Message, Clone, Copy, Debug)]
pub struct ExplosionFx {
    pub explosion: FormId,
    pub point: V3,
}

/// Every projectile is drawn, not only the PROJ's tracer chance of them [design: the
/// owner asked for bullets that are not invisible, 2026-10-10; the game draws a
/// projectile at its tracer chance, documented: GECK wiki Projectile].
pub const DRAW_EVERY_PROJECTILE: bool = true;

/// An actor was hit (and maybe died) by combat.
#[derive(Message, Clone, Copy, Debug)]
pub struct ActorHit {
    pub form_id: u32,
    pub died: bool,
    pub knocked_out: bool,
    /// Where it was hit (`GetHitLocation`'s numbering; −1 none, 0 torso).
    pub location: i32,
    /// The hit crippled a limb.
    pub crippled: bool,
    /// The body part hit is crippled (by this hit or an earlier one).
    pub part_crippled: bool,
    /// Who hit it.
    pub by: Option<FormId>,
    /// The share of its maximum health the hit took (0 to 1; `crate::rumble`).
    pub share: f32,
    /// A power attack broke through its block (`damage::against_block`): it staggers.
    pub guard_broken: bool,
}

/// An actor's weapon reached zero condition in its hand and fell (§17): the renderer
/// takes it out of the hand (nvforever `weapons`); `dropped` is the reference lying on
/// the ground, drawn like any dropped item.
#[derive(Message, Clone, Copy, Debug)]
pub struct WeaponBroken {
    pub form_id: u32,
    pub weapon: FormId,
    pub dropped: Option<FormId>,
}

/// A hit shoves an actor's body at a point (game space): the ragdoll of a dead actor is
/// pushed by it (nvforever `ragdoll.rs`). `dv` is the change of velocity of the body
/// part struck, game units per second [design].
#[derive(Message, Clone, Copy, Debug)]
pub struct ActorPush {
    pub form_id: u32,
    pub point: [f32; 3],
    pub dv: [f32; 3],
}

/// How fast a shot and a melee blow shove the body part they strike, game units per
/// second, and the share of a melee shove that goes upwards [design].
pub const SHOT_PUSH: f32 = 140.0;
pub const MELEE_PUSH: f32 = 220.0;
pub const POWER_PUSH: f32 = 1.6;

/// A shot's push on a loose object [design]: the projectile's Impact Force (PROJ DATA)
/// read as an impulse in the stored collision's units, mass × metres per second
/// [hypothesis: the field's unit is not documented; the masters' bullets carry 3–6, which
/// on a bottle of mass 1 is 3–6 m/s], times this.
pub const OBJECT_PUSH: f32 = 1.0;
/// The most speed one shot gives an object, game units per second [design].
pub const OBJECT_PUSH_MAX_SPEED: f32 = 700.0;

/// The player's limb conditions (codes 25–31, 100 healthy, 0 or less crippled), read
/// from the script runtime each frame (step C8).
#[derive(Resource, Debug, Clone, Copy)]
pub struct PlayerLimbs {
    pub condition: [f32; 7],
}

impl Default for PlayerLimbs {
    fn default() -> Self {
        PlayerLimbs { condition: [100.0; 7] }
    }
}

/// The limbs by condition code 25–31, for messages.
const LIMB_NAMES: [&str; 7] = ["head", "torso", "left arm", "right arm", "left leg", "right leg", "brain"];

/// Movement with one or both legs crippled, as a share of normal speed [design].
const ONE_LEG_SPEED: f32 = 0.7;
const BOTH_LEGS_SPEED: f32 = 0.5;

impl PlayerLimbs {
    pub fn crippled(&self, code: u16) -> bool {
        code.checked_sub(25).and_then(|i| self.condition.get(i as usize)).is_some_and(|c| *c <= 0.0)
    }

    /// Left and right arm crippled.
    pub fn arms(&self) -> (bool, bool) {
        (self.crippled(27), self.crippled(28))
    }

    /// Arm condition as `Attacker::arm_condition` takes it: 1 healthy, 0.5 with one arm
    /// crippled, 0 with both [design].
    pub fn arm_condition(&self) -> f32 {
        match self.arms() {
            (false, false) => 1.0,
            (true, true) => 0.0,
            _ => 0.5,
        }
    }

    /// The share of normal movement speed the legs allow.
    pub fn speed_mult(&self) -> f32 {
        match (self.crippled(29), self.crippled(30)) {
            (false, false) => 1.0,
            (true, true) => BOTH_LEGS_SPEED,
            _ => ONE_LEG_SPEED,
        }
    }

    /// A leg is crippled: the body limps (`locomotion\hurt\` clips) [design: either leg,
    /// owner's request 2026-10-16].
    pub fn limping(&self) -> bool {
        self.crippled(29) || self.crippled(30)
    }

    /// The crippled limbs, named.
    pub fn crippled_names(&self) -> Vec<&'static str> {
        (25u16..=31).filter(|&c| self.crippled(c)).map(|c| LIMB_NAMES[(c - 25) as usize]).collect()
    }
}

/// The body part an actor was hit on (step C8): its BPTD entry, when its volume has parts.
fn body_part(c: &Combat, parts: &ActorParts, actor: ActorId, part: Option<u8>) -> Option<BodyPart> {
    let data = parts.data_of(actor as u32)?;
    c.forms.body_parts.get(&data)?.parts.get(part? as usize).cloned()
}

/// The limb damage of a hit dealing `dealt` on part `bp` with weapon `w`: none when the
/// part has no limb condition or the weapon's Limb Dmg Mult is 0 [documented: WEAP field;
/// hypothesis: 0 means no limb damage].
fn limb_hit(bp: Option<&BodyPart>, dealt: f32, w: &WeaponStats) -> Option<LimbHit> {
    let bp = bp?;
    let condition = fnv_game::combat::limb_condition(bp.actor_value.as_deref()?)?;
    let damage = (dealt * w.limb_dmg_mult.max(0.0)) as f64;
    (damage > 0.0).then_some(LimbHit { condition, health_percent: bp.health_percent as f64, damage })
}

/// Whether the script runtime has the actor dead.
pub fn is_dead(script: Option<&ScriptState>, form_id: u32) -> bool {
    script.and_then(|s| s.rt.as_ref()).is_some_and(|rt| rt.game.is_dead(FormId(form_id)))
}

/// What the HUD shows about hits [design].
#[derive(Resource, Default, Debug, Clone)]
pub struct HitFeedback {
    /// Seconds since the last hit marker, and whether it was a kill.
    pub marker_age: Option<f32>,
    pub kill: bool,
    /// The last actor hit: name, health, maximum, seconds since.
    pub target: Option<(String, f32, f32, f32)>,
}

/// Height above a destructible's position its stage explosions go off [design].
const DESTRUCTION_BLAST_HEIGHT: f32 = 40.0;

/// Seconds the hit marker stays up [design]. (The target's health is the HUD's
/// enemy health bar, `crate::hud`.)
const MARKER_SECONDS: f32 = 0.18;

/// Counts for the exit report.
#[derive(Resource, Default, Debug, Clone)]
pub struct CombatStats {
    /// The magic items hits applied and which are still active (`hit_effects`).
    pub hit_effects: hit_effects::HitEffects,
    pub shots: u32,
    pub projectiles: u32,
    pub actor_hits: u32,
    pub world_hits: u32,
    /// Shots on objects whose OnHit / OnHitWith blocks ran, and objects knocked away.
    pub object_scripts: u32,
    pub objects_pushed: u32,
    pub explosions: u32,
    pub damage: f32,
    pub reloads: u32,
    pub dry_fires: u32,
    /// Shots that jammed the weapon (item 9).
    pub jams: u32,
    /// Melee blows that showed an impact (item 24).
    pub melee_impacts: u32,
    pub kills: u32,
    pub xp: f64,
    pub swings: u32,
    pub power_attacks: u32,
    /// Melee blows a blocker stopped part of (docs/design/combat-ai.md §14.3).
    pub blows_blocked: u32,
    /// Blocks power attacks broke through (combat-rules.md §12).
    pub blocks_broken: u32,
    /// Combat-tab lines said by anyone (Hit, Attack, PowerAttack, Death …), and tries by
    /// actors that have no line of the topic (`fnv_game::combat_lines`).
    pub combat_lines: u32,
    pub combat_lines_none: u32,
    /// Swings that went on from the last one's `a:` key (combos).
    pub chained_swings: u32,
    pub melee_hits: u32,
    pub knockdowns: u32,
    /// Shots drawn from the weapon's muzzle, and shots that left it because something
    /// stood in front of the barrel (third person).
    pub muzzle_shots: u32,
    pub covered_shots: u32,
    /// Item condition (§17): health points the player's weapons lost, hits on weapons
    /// actors hold, weapons broken, health points armour lost, armour worn to zero.
    pub weapon_wear: f32,
    pub weapon_hits: u32,
    pub weapons_broken: u32,
    pub armor_wear: f32,
    pub armor_broken: u32,
    /// Other actors' shots and blows (combat AI), their hits on the player and the
    /// damage those did, and whether the player died.
    pub actor_shots: u32,
    pub actor_strikes: u32,
    pub player_hits: u32,
    pub player_damage: f32,
    pub player_died: bool,
    /// Hits of harmless shots ([`ActorFire::harmless`]): no damage dealt.
    pub harmless_hits: u32,
}

/// A projectile in flight with where it left from, where it is drawn and whether it
/// draws a tracer.
struct Flying {
    p: Projectile,
    /// Its number among the drawn projectiles ([`ShotVisual::id`]) and its PROJ.
    id: u64,
    proj: FormId,
    /// Its model is drawn.
    shown: bool,
    /// The way it last flew.
    dir: V3,
    /// It is drawn from the weapon's muzzle (not from the eye for want of one).
    from_muzzle: bool,
    origin: V3,
    seen: Seen,
    tracer: bool,
    projectiles_per_shot: u32,
    /// The projectile's Impact Force (PROJ DATA), for [`OBJECT_PUSH`].
    force: f32,
    by: By,
}

/// A hit waiting for its delivery time (hitscan: distance ÷ speed).
struct Pending {
    due: f32,
    actor: Option<ActorId>,
    /// The body part hit (step C8).
    part: Option<u8>,
    /// The placed reference hit, for destructible objects.
    object: Option<u64>,
    point: V3,
    /// The way the projectile was flying.
    dir: V3,
    /// The surface's normal and collision material, for the impact effect.
    normal: V3,
    material: Option<u32>,
    projectiles: u32,
    force: f32,
    by: By,
}

/// What is drawn of a projectile whose hit was found at once: a bullet flying from the
/// muzzle to where it landed, or a beam between the two [design].
struct Ghost {
    id: u64,
    proj: FormId,
    from: V3,
    to: V3,
    /// Units per second (a bullet); a beam stays for `left` seconds instead.
    speed: f32,
    travelled: f32,
    beam: bool,
    left: f32,
    /// Made this frame: drawn at its start before it moves.
    fresh: bool,
    /// A beam from the muzzle: its start stays on the muzzle while it shows.
    follow: bool,
}

/// What a [`Mark`] shows.
#[derive(Clone, Copy, PartialEq)]
enum MarkKind {
    Tracer,
    Impact,
    /// A shot left the muzzle at `to`.
    Flash,
}

/// Whether the developer markers are drawn: a cross at each impact, a line along each
/// hitscan shot (settings page `hit-markers`, `--hit-markers`) [design]. Off by default:
/// the shots' own impact effects stand for them in play. The test aid
/// `NVF_PLAY_TEST_TRACERS` draws them whatever this says.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct HitMarks {
    pub show: bool,
}

/// A short-lived marker [design].
struct Mark {
    from: V3,
    to: V3,
    age: f32,
    life: f32,
    kind: MarkKind,
    /// A line from the muzzle: its start stays on the muzzle while it shows.
    follow: bool,
}

impl Mark {
    fn tracer(from: V3, to: V3, life: f32) -> Mark {
        Mark { from, to, age: 0.0, life, kind: MarkKind::Tracer, follow: false }
    }

    fn at(point: V3, life: f32, kind: MarkKind) -> Mark {
        Mark { from: point, to: point, age: 0.0, life, kind, follow: false }
    }
}

/// How long a looping shoot sound goes on after the last shot, seconds [design].
const LOOP_SOUND_HOLD: f32 = 0.25;
/// The size of the test aid's marker at the muzzle, units.
const FLASH_SIZE: f32 = 2.5;

/// Whether a projectile's hit is found at once, on its first step (hitscan, beams), as
/// `Projectile::step` decides it.
fn instant(p: &Projectile) -> bool {
    (p.hitscan || p.motion == Motion::Beam) && p.motion != Motion::Lobber
}
/// The reach of the view ray that finds the aim point when the projectile has no range
/// of its own, units [design].
const AIM_REACH: f32 = 100_000.0;

/// The player's combat state.
#[derive(Resource)]
pub struct Combat {
    pub forms: CombatForms,
    pub rules: Rules,
    pub tuning: CombatTuning,
    /// The weapon in use: the equipped one, else the Fists.
    weapon: Option<FormId>,
    /// The equipped weapon as the simulation has it (None: unarmed).
    equipped: Option<Option<FormId>>,
    /// The Fists form (WEAP `Fists`), the unarmed weapon.
    fists: Option<FormId>,
    ammo: Option<FormId>,
    /// The weapon spends itself (no ammo, a projectile).
    self_ammo: bool,
    /// The weapon is melee or unarmed (no cycle; [`Swing`]s).
    melee: bool,
    /// The melee swings (step C6).
    swings: Melee,
    /// Explosions to set off this frame (impacts, detonations, destruction stages).
    boom: Vec<(FormId, V3)>,
    cycle: Option<Cycle>,
    kick: Kick,
    flying: Vec<Flying>,
    ghosts: Vec<Ghost>,
    /// The weapon's looping shoot sound while it plays, and the seconds it has left
    /// unless another shot comes.
    looping: Option<(FormId, f32)>,
    /// The next [`ShotVisual::id`].
    next_visual: u64,
    pending: Vec<Pending>,
    /// Shots scripts asked for (`FireWeapon`), fired with the actors' shots next frame.
    script_shots: Vec<ActorFire>,
    /// People blocking now and the way each faces (horizontal, game space): the engine's
    /// combat AI says each frame (docs/design/combat-ai.md §14.3).
    pub guarding: HashMap<FormId, [f32; 2]>,
    /// Blows blocked since the combat AI last took them.
    pub blocked_blows: Vec<BlockedBlow>,
    /// Until when (`time`) the player cannot block: a power attack broke its block.
    guard_down_until: f32,
    marks: Vec<Mark>,
    rng: SplitMix,
    /// The melee swings' own rolls (variants), so they leave the hit rolls as they were.
    swing_rng: SplitMix,
    time: f32,
    /// Test aid `NVF_PLAY_TEST_NO_SPREAD=1`: shots leave exactly along the view (no
    /// spread, no bloom), so an unattended check hits what it aims at.
    no_spread: bool,
    /// A save's weapon and magazine, applied when that weapon is next taken in hand
    /// (`crate::save`).
    load_magazine: Option<(u32, u32)>,
    /// Test aid `NVF_PLAY_TEST_TRACERS=<seconds>`: every projectile draws a line, and
    /// the lines and a marker at the muzzle stay that long, so one screenshot of an
    /// unattended check shows where the shots were drawn from. The muzzle flashes and
    /// the projectile models are not held: they are drawn as in play.
    tracer_hold: Option<f32>,
    /// The spread cone's half-angle in degrees a shot would leave in now, while a weapon
    /// with a firing cycle is drawn and ready; for the crosshair (`crate::reticle`).
    pub crosshair_cone: Option<f32>,
}

impl Combat {
    /// The weapon the player fights with now: the equipped one, else the Fists.
    pub fn player_weapon(&self) -> Option<FormId> {
        self.weapon
    }

    /// The Fists (WEAP `Fists`), the unarmed weapon.
    pub fn fists_id(&self) -> Option<FormId> {
        self.fists
    }

    /// The weapon in hand and the rounds in its magazine, for a save.
    pub fn magazine_for_save(&self) -> Option<(u32, u32)> {
        let cy = self.cycle.as_ref().filter(|c| !c.no_ammo)?;
        Some((self.weapon?.0, cy.magazine))
    }

    /// A save's magazine: the rounds go back in when that weapon is in hand.
    pub fn restore_magazine(&mut self, m: (u32, u32)) {
        self.load_magazine = Some(m);
        if let (Some(w), Some(cy)) = (self.weapon, self.cycle.as_mut()) {
            if w.0 == m.0 {
                cy.magazine = m.1.min(cy.clip_size.max(m.1));
                self.load_magazine = None;
            }
        }
    }

    /// The weapon in use and its Sight FOV (degrees), when it is a gun with sights (Sight
    /// FOV above 0; melee weapons block instead) (`crate::aim_zoom`).
    pub fn sight_fov(&self) -> Option<(FormId, f32)> {
        let w = self.weapon.filter(|_| !self.melee)?;
        let fov = self.forms.weapons.get(&w)?.sight_fov;
        (fov > 0.0).then_some((w, fov))
    }

    /// The PROJ the weapon in use fires with the ammunition loaded, if it fires one.
    pub fn projectile(&self) -> Option<FormId> {
        self.forms.chain(self.weapon?, self.ammo)?.projectile
    }

    /// The weapon in hand as VATS mode needs it (docs/design/vats-mode.md §3, §7), or why
    /// it cannot enter: a gun with a firing cycle that fires a projectile, not reloading or
    /// jammed, a round loaded.
    pub fn vats_weapon(&self) -> Result<VatsWeapon, &'static str> {
        const NO_GUN: &str = "VATS mode needs a gun.";
        let id = self.equipped.flatten().ok_or(NO_GUN)?;
        let w = self.forms.weapons.get(&id).ok_or(NO_GUN)?;
        if self.melee || self.self_ammo || AttackKind::of(w) != AttackKind::Gun {
            return Err(NO_GUN);
        }
        let proj = self.forms.chain(id, self.ammo).and_then(|ch| ch.projectile).ok_or(NO_GUN)?;
        let cy = self.cycle.as_ref().ok_or(NO_GUN)?;
        if cy.reloading() || cy.jammed() {
            return Err("Not while reloading.");
        }
        // God mode keeps the magazine full (the shots take nothing).
        let rounds = if cy.no_ammo {
            u32::MAX / 2
        } else if god_mode() && cy.regen.is_none() {
            cy.clip_size
        } else {
            cy.magazine
        };
        if !cy.no_ammo && rounds < cy.ammo_use {
            return Err("Out of ammo.");
        }
        let range = self.forms.projectiles.get(&proj).map_or(0.0, |p| p.range);
        Ok(VatsWeapon {
            id,
            edid: w.edid.clone().unwrap_or_else(|| id.to_string()),
            cost: fnv_combat::vats::shot_ap_cost(w, &self.rules.ap_attack),
            rounds_per_shot: if cy.no_ammo { 1 } else { cy.ammo_use },
            rounds,
            reach: if range > 0.0 { range } else { crate::vats_mode::REACH },
        })
    }

    /// The BPTD part an actor's hit volume part is (VATS mode's marks).
    pub fn vats_part(&self, parts: &ActorParts, actor: u32, part: Option<u8>) -> Option<BodyPart> {
        body_part(self, parts, actor as ActorId, part)
    }
}

/// The weapon in hand for VATS mode ([`Combat::vats_weapon`]).
#[derive(Debug, Clone)]
pub struct VatsWeapon {
    pub id: FormId,
    pub edid: String,
    /// AP a shot (`fnv_combat::vats::shot_ap_cost`).
    pub cost: f32,
    pub rounds_per_shot: u32,
    /// Rounds in the magazine now (the marks' limit).
    pub rounds: u32,
    /// How far the crosshair's ray looks for a target: the projectile's range, else
    /// `vats_mode::REACH`.
    pub reach: f32,
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("NVF_PLAY_TEST_GOD").is_some_and(|v| v != "0") {
            set_god_mode(true);
        }
        app.init_resource::<WeaponAnim>()
            .init_resource::<CombatStats>()
            .init_resource::<ShotVisuals>()
            .add_message::<ShotFired>()
            .add_message::<ImpactFx>()
            .add_message::<CasingFx>()
            .add_message::<ExplosionFx>()
            .init_resource::<HitFeedback>()
            .init_resource::<HitMarks>()
            .init_resource::<PlayerLimbs>()
            .add_message::<ActorHit>()
            .add_message::<ActorPush>()
            .add_message::<WeaponBroken>()
            .add_message::<ActorFire>()
            .add_message::<ActorStrike>()
            .add_systems(Startup, (init_combat, spawn_hit_hud))
            .add_systems(
                Update,
                (track_limbs, weapon_state, equip_sync, run_pace, weapon_sounds, fire, melee, actor_attacks, fly, log_combat_lines).chain().in_set(PlaySet::Activation).run_if(in_state(PlayState::Playing)).run_if(resource_exists::<Combat>),
            )
            .add_systems(Update, (draw_marks, update_hit_hud, watch_hit_effects).run_if(resource_exists::<Combat>))
            .add_systems(Last, report);
    }
}

/// The player runs slower with a two-handed weapon drawn, by `fMove2HBigPenalty`
/// (`crate::player::RunPace`; the owner's playtest list 5 item 61) [design: every
/// two-handed weapon; hypothesis: the setting's role, from its name].
fn run_pace(c: Res<Combat>, control: Res<PlayerControl>, pace: Option<ResMut<crate::player::RunPace>>) {
    let Some(mut pace) = pace else { return };
    let two = control.weapon_out && c.weapon.and_then(|w| c.forms.weapons.get(&w)).is_some_and(WeaponStats::two_handed);
    let want = if two { (1.0 - c.rules.move_2h_penalty).clamp(0.1, 1.0) } else { 1.0 };
    if pace.0 != want {
        pace.0 = want;
        info!("movement: running at {:.0}% of the speed ({})", want * 100.0, if two { "a two-handed weapon drawn" } else { "no two-handed weapon drawn" });
    }
}

/// Read the player's limb conditions from the script runtime (step C8).
fn track_limbs(mut limbs: ResMut<PlayerLimbs>, mut script: Option<ResMut<ScriptState>>, mut sim: ResMut<SimState>) {
    let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) else { return };
    let now = with_world(rt, &mut sim, |rt| rt.game.limb_conditions(FormId::PLAYER_REF)).map(|v| v as f32);
    if now != limbs.condition {
        let before = *limbs;
        limbs.condition = now;
        for c in 25u16..=31 {
            if limbs.crippled(c) != before.crippled(c) {
                info!("combat: player's {} {}", LIMB_NAMES[(c - 25) as usize], if limbs.crippled(c) { "crippled" } else { "healed" });
            }
        }
    }
}

/// Tell the script runtime whether the player's weapon is out (`IsWeaponOut`): drawn and
/// in hand, as the body reports it [documented: the value changes when the weapon is
/// attached to the weapon node, catalogue notes of IsWeaponOut].
fn weapon_state(anim: Res<WeaponAnim>, mut script: Option<ResMut<ScriptState>>) {
    if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
        rt.game.set_weapon_out(FormId::PLAYER_REF, anim.ready);
    }
}

fn init_combat(mut commands: Commands, d: Res<PlayWorld>) {
    let t = std::time::Instant::now();
    let tracer_hold = std::env::var("NVF_PLAY_TEST_TRACERS").ok().and_then(|v| v.trim().parse().ok()).filter(|s: &f32| *s > 0.0);
    let forms = CombatForms::build(d.lo, d.schema);
    let rules = Rules::from_forms(&forms);
    let fists = forms.by_edid("Fists");
    info!("combat: {} weapons, {} ammo, {} projectiles, {} explosions read in {:.0} ms", forms.weapons.len(), forms.ammo.len(), forms.projectiles.len(), forms.explosions.len(), t.elapsed().as_secs_f64() * 1000.0);
    commands.insert_resource(Combat {
        forms,
        rules,
        tuning: CombatTuning::default(),
        weapon: None,
        equipped: None,
        fists,
        ammo: None,
        self_ammo: false,
        melee: false,
        swings: Melee::default(),
        boom: Vec::new(),
        cycle: None,
        kick: Kick::default(),
        flying: Vec::new(),
        ghosts: Vec::new(),
        looping: None,
        next_visual: 0,
        pending: Vec::new(),
        script_shots: Vec::new(),
        guarding: HashMap::new(),
        blocked_blows: Vec::new(),
        guard_down_until: 0.0,
        marks: Vec::new(),
        rng: SplitMix(1),
        swing_rng: SplitMix(2),
        time: 0.0,
        no_spread: std::env::var_os("NVF_PLAY_TEST_NO_SPREAD").is_some_and(|v| v != "0"),
        tracer_hold,
        load_magazine: None,
        crosshair_cone: None,
    });
}

/// The player's stance for the spread: aiming, moving, running, sneaking, in the air,
/// the weapon's skill and Strength, crippled arms.
fn player_stance(control: &PlayerControl, phys: &PhysState, limbs: &PlayerLimbs, skill: f32, strength: f32) -> Stance {
    Stance {
        aiming: control.aiming,
        moving: phys.player.as_ref().is_some_and(|ch| ch.velocity[0].hypot(ch.velocity[1]) > 1.0),
        running: control.always_run,
        sneaking: control.sneak,
        airborne: phys.player.as_ref().is_some_and(|ch| !ch.grounded && !ch.swimming),
        skill,
        strength,
        aiming_move_divisor: 1.0,
        left_arm_crippled: limbs.arms().0,
        right_arm_crippled: limbs.arms().1,
    }
}

/// How many of `item` the player carries.
fn player_count(sim: &SimState, item: Option<FormId>) -> u32 {
    let inv = sim.world.peek(FormId::PLAYER_REF).and_then(|s| s.inventory.as_ref());
    item.and_then(|i| inv.map(|inv| inv.count(i))).unwrap_or(0).max(0) as u32
}

/// Follow the equipped weapon: a new weapon gets a new cycle, loaded from what is carried.
fn equip_sync(mut c: ResMut<Combat>, mut sim: ResMut<SimState>, mut anim: ResMut<WeaponAnim>) {
    let c = &mut *c;
    let equipped = sim.world.peek(FormId::PLAYER_REF).and_then(|s| s.inventory.as_ref()).map(|i| i.equipped()).unwrap_or_default();
    let mut equipped = equipped.into_iter().find(|id| c.forms.weapons.contains_key(id));
    // A weapon at zero condition does not stay in the hand, however it got there (a
    // script's SetWeaponHealthPerc) (§17).
    if let Some(id) = equipped.filter(|&id| item_condition(&sim, FormId::PLAYER_REF, id) <= 0.0) {
        let _ = sim.world.unequip(FormId::PLAYER_REF, id);
        info!("combat: the player's {} ({id}) is broken: unequipped", item_name(&sim, id));
        equipped = None;
    }
    if c.equipped == Some(equipped) {
        return;
    }
    c.equipped = Some(equipped);
    let weapon = equipped.or(c.fists);
    c.weapon = weapon;
    c.cycle = None;
    c.ammo = None;
    c.melee = false;
    c.swings.reset();
    anim.swing = None;
    let Some(id) = weapon else {
        anim.attack = None;
        anim.reload = None;
        return;
    };
    let w = &c.forms.weapons[&id];
    let ammo = c.forms.ammo_for(id, |a| player_count(&sim, Some(a)) > 0);
    let chain = c.forms.chain(id, ammo).expect("weapon present");
    let melee = AttackKind::of(w) != AttackKind::Gun;
    c.melee = melee && chain.projectile.is_none();
    c.self_ammo = !melee && chain.ammo_choices.is_empty() && chain.projectile.is_some();
    c.ammo = chain.ammo;
    let carried = player_count(&sim, c.ammo);
    let timing = Timing::of(w, &anim.timing, &c.tuning);
    // Ranged weapons without a projectile (the detonator, binoculars) use no ammo.
    let no_ammo = melee || c.self_ammo || (chain.projectile.is_none() && chain.ammo_choices.is_empty());
    c.cycle = Some(Cycle::new(w, timing, no_ammo, carried));
    // A save's magazine for this weapon (`crate::save`), no more than is carried.
    if let (Some((sw, n)), Some(cy)) = (c.load_magazine, c.cycle.as_mut()) {
        if sw == id.0 {
            cy.magazine = n.min(carried);
            c.load_magazine = None;
        }
    }
    anim.attack = w.attack_anim.clone();
    anim.reload = w.reload_anim.clone();
    anim.jam = w.reload_anim.as_deref().and_then(jam_anim);
    info!(
        "combat: weapon {} ({id}), ammo {}, magazine {}/{}, {} projectile(s) per shot",
        w.edid.as_deref().unwrap_or("?"),
        c.ammo.and_then(|a| c.forms.ammo.get(&a)).and_then(|a| a.edid.clone()).unwrap_or_else(|| if c.self_ammo { "itself".into() } else { "none".into() }),
        c.cycle.as_ref().map_or(0, |cy| cy.magazine),
        w.clip_size,
        chain.projectiles_per_shot
    );
}

/// The impact a weapon's impact data set names for what a hit struck, and the material
/// it was chosen by: an actor's is its base's Impact Material Type (NPC_ / CREA `NAM4`),
/// organic without one [design]; anything else's its collision material
/// (docs/spec-notes/audio.md §11). Shots and melee blows alike (item 24).
fn impact_for(s: &Sfx, sim: &SimState, weapon: FormId, actor: Option<ActorId>, material: Option<u32>) -> (Option<FormId>, &'static str) {
    let material = match actor {
        Some(a) => sim.base_of(a as u32).and_then(|b| s.forms.actor_material(b)).unwrap_or("organic"),
        None => fnv_combat::fx::impact_material(material),
    };
    let set = s.forms.weapons.get(&weapon).and_then(|w| w.impact_dataset);
    (set.and_then(|set| s.forms.impact_id(set, material)), material)
}

/// Show an impact (the renderer's model and debris, `ImpactFx`) and play its sounds there.
fn show_impact(s: &Sfx, fx: ImpactFx, sounds: &mut MessageWriter<PlaySound>, impacts: &mut MessageWriter<ImpactFx>) {
    impacts.write(fx);
    if let Some(i) = s.forms.impacts.get(&fx.impact) {
        for sound in i.sound_1.into_iter().chain(i.sound_2) {
            sounds.write(PlaySound { sound, at: Some(fx.point) });
        }
    }
}

/// The jam animation beside a reload animation: `ReloadA` → `JamA`, `ReloadX` → `JamX`
/// [observed: `1hpjama.kf`…`2hrjamz.kf` beside the reload clips in the meshes archive;
/// the letters' pairing is a hypothesis from the names].
pub fn jam_anim(reload: &str) -> Option<String> {
    let rest = reload.strip_prefix("Reload").or_else(|| reload.strip_prefix("reload"))?;
    (!rest.is_empty()).then(|| format!("Jam{rest}"))
}

/// The physics world as combat sees it: rays that skip the player's capsule and the
/// layers nothing solid is on, and actor hit volumes.
struct PhysView<'a> {
    q: Queries<'a>,
    player: Option<Key>,
    /// The characters of the actors another system walks (`PhysState::npcs`).
    npcs: Vec<Key>,
    actors: Vec<(ActorId, V3, f32)>,
    /// Each actor's body part capsule centres, widest first.
    aims: HashMap<ActorId, Vec<V3>>,
}

impl CombatWorld for PhysView<'_> {
    fn ray(&self, from: V3, to: V3, ignore: &[ActorId]) -> Option<WorldHit> {
        let d = vec::sub(to, from);
        let len = vec::len(d);
        if len < 1e-4 {
            return None;
        }
        // Shots pass what the character passes (triggers, zones, water, …) [design].
        let mut f = Filter { exclude_layers: layers::character_ignores(), ..Default::default() };
        // Another actor's shot can hit the player's character; the player's own and
        // the rest (blasts, picks) pass it, as before.
        let other_shoots = ignore.iter().any(|&a| a != PLAYER);
        if !other_shoots {
            f.exclude.extend(self.player);
        }
        // The shooter's own hit volume and character.
        f.exclude.extend(ignore.iter().filter(|&&a| a != PLAYER).flat_map(|&a| [Key::Actor(a as u32), Key::Character(a as u32)]));
        // An actor's character (the capsule that walks it) is not its body: its hit
        // volumes are. A shot or a swing that met the character first struck it as a
        // wall, dust and no damage, and one past a flying or low creature's capsule
        // missed its body (the owner's playtest list 5 items 16 and 25) [design].
        f.exclude.extend(self.npcs.iter().copied());
        let h = self.q.ray(from, d, len, &f)?;
        let player_hit = other_shoots && self.player == Some(h.owner.key());
        Some(WorldHit {
            point: h.point,
            normal: h.normal,
            distance: h.distance,
            actor: match h.owner {
                Owner::Actor { form_id, .. } => Some(form_id as ActorId),
                _ if player_hit => Some(PLAYER),
                _ => None,
            },
            part: match h.owner {
                Owner::Actor { part, .. } if part != fnv_phys::actors::WHOLE => Some(part),
                _ => None,
            },
            small_transparent: matches!(h.owner, Owner::Ref { layer, .. } if layer == layers::TRANSPARENT_SMALL || layer == layers::TRANSPARENT_SMALL_ANIM),
            material: h.material,
            object: match h.owner {
                Owner::Ref { form_id, .. } => Some(form_id as u64),
                _ => None,
            },
        })
    }

    fn actors_near(&self, point: V3, radius: f32) -> Vec<(ActorId, V3)> {
        self.actors.iter().filter(|(_, c, r)| vec::len(vec::sub(*c, point)) <= radius + r).map(|&(id, c, _)| (id, c)).collect()
    }

    fn aim_points(&self, actor: ActorId) -> Vec<V3> {
        self.aims.get(&actor).cloned().unwrap_or_default()
    }
}

fn view(phys: &mut PhysState) -> PhysView<'_> {
    let actors: Vec<(ActorId, V3, f32)> =
        phys.world.actor_volumes().filter_map(|id| phys.world.actor_volume(id).map(|v| (id as ActorId, v.center(), v.radius))).collect();
    let player = phys.player.as_ref().map(|c| c.key());
    let npcs = phys.npcs.values().map(|b| b.ch.key()).collect();
    let aims = actors.iter().map(|&(id, ..)| (id, phys.world.actor_part_centers(id as u32))).filter(|(_, v)| !v.is_empty()).collect();
    phys.world.update_queries();
    PhysView { q: phys.world.queries(), player, npcs, actors, aims }
}

/// An actor's current value as the formulas read it (within its range: S.P.E.C.I.A.L.
/// 1–10, skills 0–100), from the script runtime's actor values (the record, the
/// formulas and whatever scripts changed: docs/spec-notes/actor-values.md); `None`
/// without a runtime.
fn actor_av(script: Option<&mut ScriptState>, sim: &mut SimState, actor: FormId, code: u16) -> Option<f32> {
    let rt = script.and_then(|s| s.rt.as_mut())?;
    Some(with_world(rt, sim, |rt| fnv_game::functions::actor_value_capped(&mut rt.game, actor, code)) as f32)
}

/// An actor's attacker values: the weapon's skill, Strength, Luck and Unarmed, and
/// what the Melee Damage, Unarmed Damage and Critical Chance actor values hold above
/// their formulas (scripts' changes). Without a script runtime, the defaults.
fn attacker(mut script: Option<&mut ScriptState>, sim: &mut SimState, rules: &Rules, p: FormId, skill: Option<&str>, aiming_vats: bool) -> Attacker {
    use fnv_game::actor_values as avs;
    let d = Attacker::default();
    let mut av = |code: u16, or: f32| actor_av(script.as_deref_mut(), sim, p, code).unwrap_or(or);
    let (strength, luck, unarmed_skill) = (av(avs::STRENGTH, d.strength), av(avs::LUCK, d.luck), av(avs::UNARMED, d.unarmed_skill));
    let skill = skill.and_then(fnv_game::functions::av_by_name).map_or(d.skill, |code| av(code, d.skill));
    let melee = damage::melee_damage_av(rules, strength);
    let unarmed = damage::unarmed_damage_av(rules, unarmed_skill);
    let crit = rules.crit_luck_base + rules.crit_luck_mult * luck;
    Attacker {
        skill,
        strength,
        luck,
        unarmed_skill,
        melee_damage_bonus: av(avs::MELEE_DAMAGE, melee) - melee,
        unarmed_damage_bonus: av(avs::UNARMED_DAMAGE, unarmed) - unarmed,
        crit_chance_bonus: av(avs::CRITICAL_CHANCE, crit) - crit,
        in_vats: aiming_vats,
        ..d
    }
}

/// The weapon's Equip sound when the player starts to draw it and its Unequip sound
/// when they start to put it away [hypothesis: when the game plays them].
fn weapon_sounds(c: Res<Combat>, control: Res<PlayerControl>, sfx: Option<Res<Sfx>>, mut sounds: MessageWriter<PlaySound>, mut was_out: Local<bool>) {
    let out = control.weapon_out;
    if out == *was_out {
        return;
    }
    *was_out = out;
    let fx = c.weapon.zip(sfx.as_deref()).and_then(|(w, s)| s.forms.weapons.get(&w));
    if let Some(sound) = fx.and_then(|f| if out { f.equip } else { f.unequip }) {
        sounds.write(PlaySound { sound, at: None });
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fire(
    time: Res<Time>,
    mut c: ResMut<Combat>,
    mut sim: ResMut<SimState>,
    mut phys: ResMut<PhysState>,
    mut player: ResMut<Player>,
    mut control: ResMut<PlayerControl>,
    mut anim: ResMut<WeaponAnim>,
    mut stats: ResMut<CombatStats>,
    mut script: Option<ResMut<ScriptState>>,
    limbs: Res<PlayerLimbs>,
    mut fired: MessageWriter<ShotFired>,
    sfx: Option<Res<Sfx>>,
    mut sounds: MessageWriter<PlaySound>,
    mut casings: MessageWriter<CasingFx>,
    mut loops: MessageWriter<LoopSound>,
    (mut hud, mut vats): (Option<ResMut<HudMessage>>, Option<ResMut<crate::vats_mode::VatsMode>>),
) {
    let dt = time.delta_secs();
    let c = &mut *c;
    c.time += dt;
    // VATS mode's marked shot due now: the point it goes straight at (vats-mode.md §9.1).
    let vats_shot = vats.as_deref_mut().and_then(|v| v.take_shot());
    // The looping shoot sound stops a moment after the last shot.
    if let Some((sound, left)) = c.looping.as_mut() {
        *left -= dt;
        if *left <= 0.0 {
            loops.write(LoopSound { sound: *sound, on: false });
            c.looping = None;
        }
    }
    // Recoil recovers whatever happens.
    let back = c.kick.recover(&c.tuning, dt);
    player.pitch = (player.pitch - back.to_radians()).clamp(-MAX_PITCH, MAX_PITCH);
    let reload = std::mem::take(&mut control.reload);
    let swap = std::mem::take(&mut control.ammo_swap);
    c.crosshair_cone = None;
    if c.melee {
        return;
    }
    let (Some(id), Some(cycle)) = (c.weapon, c.cycle.as_mut()) else { return };
    if !anim.ready {
        if vats_shot.is_some() {
            info!("combat: the VATS mode shot could not leave (the weapon is not out)");
        }
        return;
    }
    let w = c.forms.weapons[&id].clone();
    cycle.timing = Timing::of(&w, &anim.timing, &c.tuning);
    // A VATS mode shot leaves now, whatever the weapon's cooldown or wind-up (the owner's
    // "faster than the weapon's normal rate"), unless it is reloading, jammed or empty.
    if vats_shot.is_some() {
        if cycle.reloading() || cycle.jammed() || (!cycle.no_ammo && cycle.magazine < cycle.ammo_use) {
            info!("combat: the VATS mode shot could not leave (reloading, jammed or empty)");
        } else {
            cycle.phase = fnv_combat::attack::Phase::WindUp { left: 0.0 };
        }
    }
    // Ammo Swap: the next carried type down the list, then a reload [design].
    let mut reload = reload;
    if swap {
        if let Some(cur) = c.ammo {
            let next = c.forms.next_ammo(id, cur, |a| player_count(&sim, Some(a)) > 0);
            if next.is_some_and(|n| n != cur) {
                c.ammo = next;
                cycle.magazine = 0;
                reload = true;
                info!("combat: ammo swapped to {}", next.and_then(|a| c.forms.ammo.get(&a)).and_then(|a| a.edid.clone()).unwrap_or_default());
            }
        }
    }
    // God mode: no ammo is used and nothing needs reloading (the owner's playtest list 5
    // item 49) [design]: the magazine stays full and shots take nothing from the
    // inventory.
    let god = god_mode();
    if god && !cycle.no_ammo && cycle.regen.is_none() && !c.self_ammo {
        cycle.magazine = cycle.clip_size;
    }
    let total = if c.self_ammo { player_count(&sim, Some(id)) } else { player_count(&sim, c.ammo) };
    let carried = total.saturating_sub(cycle.magazine);
    let events = cycle.step(&c.tuning, Input { trigger: control.trigger, reload: reload && !god, dt, carried });
    let Some(chain) = c.forms.chain(id, c.ammo) else { return };
    // The cone a shot would leave in now, for the crosshair (bloom included).
    {
        let effects: Vec<&AmmoEffect> = chain.ammo_effects.iter().filter_map(|e| c.forms.ammo_effects.get(e)).collect();
        let shooter = attacker(script.as_deref_mut(), &mut sim, &c.rules, FormId::PLAYER_REF, w.skill.as_deref(), false);
        let stance = player_stance(&control, &phys, &limbs, shooter.skill, shooter.strength);
        let cone = spread::cone_deg(spread::actor_spread(&c.rules, &c.tuning, &w, &stance), c.kick.bloom, &w, &effects);
        c.crosshair_cone = Some(if c.no_spread { 0.0 } else { cone });
    }
    let weapon_fx = sfx.as_deref().and_then(|s| s.forms.weapons.get(&id));
    for e in events {
        match e {
            Event::SpinUp => {
                anim.spin_ups += 1;
                debug!("combat: {} spins up, first shot in {:.2} s", w.edid.as_deref().unwrap_or("?"), cycle.timing.spin_up);
            }
            Event::Fired { rounds } => {
                anim.attacks += 1;
                stats.shots += 1;
                if rounds > 0 && !god {
                    if let Some(a) = c.ammo {
                        let _ = sim.world.remove_item(FormId::PLAYER_REF, a, rounds as i32);
                    }
                }
                if c.self_ammo && !god {
                    let _ = sim.world.remove_item(FormId::PLAYER_REF, id, 1);
                }
                // The weapon's OnFire blocks (the detonator's DetonatePlacedExplosives),
                // and the player's Attack or FireExplosive line (a thrown weapon).
                if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
                    if with_world(rt, &mut sim, |rt| rt.game.weapon_fired(FormId::PLAYER_REF, id)) {
                        info!("combat: OnFire of {} ran", w.edid.as_deref().unwrap_or("?"));
                    }
                    with_world(rt, &mut sim, |rt| rt.game.attack_line(FormId::PLAYER_REF, None, false, c.self_ammo));
                }
                let Some(proj) = chain.projectile else {
                    debug!("combat: {} fires no projectile", w.edid.as_deref().unwrap_or("?"));
                    continue;
                };
                fired.write(ShotFired { projectile: proj, weapon: id, shooter: FormId::PLAYER_REF, at: None });
                // A casing for each shot of a weapon with a casing model, from its
                // `ShellCasingNode` at the game's eject speed (`fGunShellEjectSpeed`)
                // [hypothesis: one a shot, at the shot; the game's timing is not known].
                if let (Some((point, dir)), true) = (anim.casing, weapon_fx.is_some_and(|f| f.shell_casing_model.is_some())) {
                    casings.write(CasingFx { weapon: id, point, velocity: vec::scale(dir, c.rules.shell_eject_speed), life: c.rules.shell_lifetime });
                }
                // A weapon with a looping shoot sound (the minigun has no other) plays
                // it for as long as shots keep coming [hypothesis: when the game does].
                if let Some(sound) = weapon_fx.and_then(|f| f.shoot_loop) {
                    if c.looping.is_none_or(|(s, _)| s != sound) {
                        if let Some((old, _)) = c.looping {
                            loops.write(LoopSound { sound: old, on: false });
                        }
                        loops.write(LoopSound { sound, on: true });
                    }
                    c.looping = Some((sound, LOOP_SOUND_HOLD));
                }
                // The player's own shot: the 2D sound where the weapon has one.
                if let Some(sound) = weapon_fx.and_then(|f| f.shoot_2d.or(f.shoot_3d)) {
                    sounds.write(PlaySound { sound, at: None });
                }
                let effects: Vec<&AmmoEffect> = chain.ammo_effects.iter().filter_map(|e| c.forms.ammo_effects.get(e)).collect();
                let shooter = attacker(script.as_deref_mut(), &mut sim, &c.rules, FormId::PLAYER_REF, w.skill.as_deref(), false);
                let stance = player_stance(&control, &phys, &limbs, shooter.skill, shooter.strength);
                let actor_spread = spread::actor_spread(&c.rules, &c.tuning, &w, &stance);
                let cone = if c.no_spread || vats_shot.is_some() { 0.0 } else { spread::cone_deg(actor_spread, c.kick.bloom, &w, &effects) };
                let eye = player.eye();
                let gravity = phys.world.params.gravity;
                let (tracer_chance, force, range) =
                    chain.projectile.and_then(|p| c.forms.projectiles.get(&p)).map_or((0.0, 0.0, 0.0), |p| (p.tracer_chance, p.impact_force, p.range));
                // The aim is the view ray from the eye; the shot is drawn from the
                // muzzle, and in third person leaves it when the barrel is behind cover
                // (docs/design/combat.md §5.1).
                let reach = if range > 0.0 { range } else { AIM_REACH };
                // In third person the view ray is the camera's, through the crosshair over
                // the shoulder: the shot aims from the eye at what lies under it, as the
                // eye's view ray does in first person (playtest list 4, item 12).
                let leaving = {
                    let world = view(&mut phys);
                    let aim = match control.crosshair.as_ref() {
                        // VATS mode: from the eye straight at the mark (100 % accuracy).
                        _ if vats_shot.is_some() => {
                            let d = vec::sub(vats_shot.unwrap_or(eye), eye);
                            if vec::len(d) > 1e-3 {
                                vec::normalize(d)
                            } else {
                                player.look_dir()
                            }
                        }
                        Some(cross) => {
                            let (start, dir) = crate::control::Crosshair::ray(Some(cross), eye, player.look_dir());
                            let end = vec::add(start, vec::scale(dir, reach));
                            let hit = world.ray(start, end, &[PLAYER]).map(|h| h.point);
                            crate::control::Crosshair::aim_from(eye, start, dir, reach, hit)
                        }
                        None => player.look_dir(),
                    };
                    shot::leaving(&world, eye, aim, anim.muzzle, reach, Some(PLAYER), control.third_person_view())
                };
                let rng = &mut c.rng;
                let mut rand = || rng.next_f32();
                let shots = shot::fire(&c.forms, &chain, leaving.origin, leaving.aim, cone, Some(PLAYER), gravity, &c.tuning, &mut rand);
                stats.projectiles += shots.len() as u32;
                for p in shots {
                    let rolled = c.rng.next_f32() < tracer_chance || c.tracer_hold.is_some();
                    let shown = rolled || DRAW_EVERY_PROJECTILE;
                    // A line along the shot: for what is too fast to follow (found at
                    // once), and at the tracer chance for the rest [design].
                    let tracer = rolled || (shown && instant(&p));
                    let dir = vec::normalize(p.vel);
                    c.next_visual += 1;
                    let from_muzzle = anim.muzzle.is_some() && leaving.seen.at(leaving.origin, 0.0) != eye;
                    let by = By { actor: FormId::PLAYER_REF, weapon: id, ammo: c.ammo, harmless: false, vats: vats_shot.is_some() };
                    c.flying.push(Flying { p, id: c.next_visual, proj, shown, dir, from_muzzle, origin: leaving.origin, seen: leaving.seen, tracer, projectiles_per_shot: chain.projectiles_per_shot, force, by });
                }
                let seen_from = leaving.seen.at(leaving.origin, 0.0);
                // The flash itself is the renderer's (`ShotFired`); the test aid marks
                // the point shots are drawn from.
                if let (Some(hold), true) = (c.tracer_hold, anim.muzzle.is_some()) {
                    c.marks.push(Mark::at(seen_from, hold, MarkKind::Flash));
                }
                stats.muzzle_shots += (seen_from != eye) as u32;
                stats.covered_shots += leaving.covered as u32;
                let p3 = |v: V3| format!("({:.1}, {:.1}, {:.1})", v[0], v[1], v[2]);
                debug!(
                    "combat: shot aimed from the eye {} at {} ({:.0} units), muzzle {}, drawn from {}{}",
                    p3(eye),
                    p3(leaving.aim_point),
                    leaving.aim_distance,
                    anim.muzzle.map_or("none (the eye)".to_string(), p3),
                    p3(seen_from),
                    if leaving.covered { "; cover in front of the barrel: it leaves the muzzle" } else { "" }
                );
                // VATS mode's shots kick nothing and grow no bloom [design, vats-mode.md §11.1].
                if vats_shot.is_none() {
                    let kick = c.kick.shot(&c.tuning, w.min_spread, w.base_damage as f32);
                    player.pitch = (player.pitch + kick.to_radians()).clamp(-MAX_PITCH, MAX_PITCH);
                }
                debug!("combat: shot, cone {cone:.2}°, magazine {}", cycle.magazine);
                // The shot wears the weapon (§17); a thrown weapon is spent instead.
                if !c.self_ammo {
                    // Worn below the jamming mark, the shot may jam it (item 9): the jam
                    // clip plays and nothing fires until it ends. Weapons that use ammo
                    // only; a thrown one spends itself [design].
                    let jams = vats_shot.is_none() && c.ammo.is_some() && c.rng.next_f32() < condition::jam_chance(&c.tuning, item_condition(&sim, FormId::PLAYER_REF, id));
                    let points = condition::weapon_wear(&c.rules, &w, &effects);
                    if wear_weapon(&c.forms, &mut sim, &mut stats, hud.as_deref_mut(), None, script.as_deref_mut(), FormId::PLAYER_REF, id, points, None).is_some_and(|w| w.broke) {
                        break;
                    }
                    if jams {
                        let secs = anim.timing.jam_len.unwrap_or(c.tuning.jam_fallback);
                        cycle.jam(secs);
                        anim.jams += 1;
                        stats.jams += 1;
                        info!(
                            "combat: {} jammed at {}% condition (clip {}, {secs:.2} s)",
                            w.edid.as_deref().unwrap_or("?"),
                            condition::percent(item_condition(&sim, FormId::PLAYER_REF, id)),
                            anim.jam.as_deref().unwrap_or("none")
                        );
                        break;
                    }
                }
            }
            Event::ReloadStarted => {
                anim.reloads += 1;
                match cycle.timing.rounds {
                    Some(r) => info!("combat: reloading round by round (opens {:.2} s, {:.2} s a round, closes {:.2} s)", r.open, r.round, r.close),
                    None => debug!("combat: reloading"),
                }
            }
            Event::ReloadRound => anim.reload_rounds += 1,
            Event::RoundLoaded => info!("combat: a round in, magazine {}", cycle.magazine),
            Event::ReloadClosing { stopped } => {
                info!("combat: closing{}", if stopped { " (the trigger was pulled)" } else { "" });
            }
            Event::Reloaded { taken } => {
                stats.reloads += 1;
                info!("combat: reloaded {taken}, magazine {}", cycle.magazine);
            }
            Event::DryFire => {
                if let Some(sound) = weapon_fx.and_then(|f| f.no_ammo) {
                    sounds.write(PlaySound { sound, at: None });
                }
                if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
                    with_world(rt, &mut sim, |rt| rt.game.combat_line(FormId::PLAYER_REF, None, "NoAmmo"));
                }
                stats.dry_fires += 1;
                info!("combat: click (no ammo)");
            }
            Event::Regenerated => {}
            Event::Cleared => info!("combat: the jam is cleared"),
        }
    }
}

/// The target's DT and DR from the armour it wears (its carried ARMO by the wearing rule).
/// The simulation creates an actor's inventory on first touch (`World::inventory_mut`).
/// Each piece protects by its condition (§17).
fn target_armor(c: &Combat, mut script: Option<&mut ScriptState>, sim: &mut SimState, target: FormId) -> (f32, f32) {
    let (dt, dr) = c.forms.armor_values_worn(&c.tuning, &worn_armor(&c.forms, sim, target));
    // The actor's own Damage Threshold (76) and Damage Resistance (18) actor values:
    // nothing until a script changes them (abilities are not applied yet).
    let own_dt = actor_av(script.as_deref_mut(), sim, target, 76).unwrap_or(0.0);
    let own_dr = actor_av(script, sim, target, 18).unwrap_or(0.0);
    (dt + own_dt, dr + own_dr)
}

/// The armour `target` wears, each piece with its condition: what the player has
/// equipped; of another actor, what the rules dress it in (`World::equipment`: its
/// equipped items once outfitted, as the engine draws it), else its carried ARMO by the
/// wearing rule.
fn worn_armor(forms: &CombatForms, sim: &mut SimState, target: FormId) -> Vec<(FormId, f32)> {
    let dressed = if target == FormId::PLAYER_REF { None } else { sim.world.equipment(target) };
    let Ok(inv) = sim.world.inventory_mut(target) else { return Vec::new() };
    let items: Vec<FormId> = match dressed {
        Some(d) => d,
        None if target == FormId::PLAYER_REF => inv.equipped(),
        None => inv.iter().filter(|(_, n)| *n > 0).map(|(id, _)| id).collect(),
    };
    forms.worn_armor(items).into_iter().map(|id| (id, inv.condition_in_use(id).unwrap_or(1.0))).collect()
}

/// The condition of the unit of `item` that `actor` uses; 1 when it carries none (the
/// Fists).
fn item_condition(sim: &SimState, actor: FormId, item: FormId) -> f32 {
    sim.world.peek(actor).and_then(|s| s.inventory.as_ref()).and_then(|i| i.condition_in_use(item)).unwrap_or(1.0)
}

fn item_name(sim: &SimState, item: FormId) -> String {
    sim.name(item).map_or_else(|| item.to_string(), str::to_string)
}

/// A hit of `damage` (before armour) wears the armour `target` wears in `zone` (§17).
fn wear_armor(c: &Combat, sim: &mut SimState, stats: &mut CombatStats, target: FormId, zone: Zone, damage: f32) {
    let worn: Vec<FormId> = worn_armor(&c.forms, sim, target).into_iter().map(|w| w.0).collect();
    let struck = c.forms.armor_struck(&worn, zone);
    if struck.is_empty() || damage <= 0.0 {
        return;
    }
    let points = condition::armor_wear(&c.rules, damage) / struck.len() as f32;
    for item in struck {
        let health = c.forms.item_health(item);
        let Some(w) = sim.world.inventory_mut(target).ok().and_then(|inv| condition::wear_item(inv, item, points, health)) else { continue };
        stats.armor_wear += (w.before - w.after) * health as f32;
        info!("combat: {target}'s {} ({zone:?} hit) loses {points:.1}: condition {:.1}% -> {:.1}%", item_name(sim, item), w.before * 100.0, w.after * 100.0);
        if w.broke {
            stats.armor_broken += 1;
            info!("combat: {target}'s {} ({item}) is worn to nothing", item_name(sim, item));
        }
    }
}

/// Take `points` health points from the weapon `item` that `actor` uses (§17). At zero
/// it breaks: the player's is unequipped and stays in the inventory, where it cannot be
/// equipped until it is repaired; another actor's falls to the ground (at `at`, when
/// given) and [`WeaponBroken`] tells the renderer. `None` when the weapon does not wear
/// (not carried, or its record has no health).
#[allow(clippy::too_many_arguments)]
pub fn wear_weapon(
    forms: &CombatForms,
    sim: &mut SimState,
    stats: &mut CombatStats,
    hud: Option<&mut HudMessage>,
    broken: Option<&mut MessageWriter<WeaponBroken>>,
    script: Option<&mut ScriptState>,
    actor: FormId,
    item: FormId,
    points: f32,
    at: Option<V3>,
) -> Option<Wear> {
    let health = forms.item_health(item);
    let w = condition::wear_item(sim.world.inventory_mut(actor).ok()?, item, points, health)?;
    if actor == FormId::PLAYER_REF {
        stats.weapon_wear += (w.before - w.after) * health as f32;
    }
    if !w.broke {
        return Some(w);
    }
    stats.weapons_broken += 1;
    // Its owner says WeaponBroke (docs/spec-notes/combat-dialogue.md).
    if let Some(rt) = script.and_then(|s| s.rt.as_mut()) {
        with_world(rt, sim, |rt| rt.game.combat_line(actor, None, "WeaponBroke"));
    }
    let name = item_name(sim, item);
    if actor == FormId::PLAYER_REF {
        let _ = sim.world.unequip(actor, item);
        if let Some(h) = hud {
            h.show(format!("{name} is broken"));
        }
        info!("combat: the player's {name} ({item}) broke: unequipped, it stays in the inventory until repaired");
        return Some(w);
    }
    let dropped = match sim.world.drop_equipped(actor, item) {
        Ok(refs) => refs.first().copied(),
        Err(e) => {
            warn!("combat: {actor}'s broken {name} could not be dropped: {e}");
            None
        }
    };
    if let (Some(r), Some(at)) = (dropped, at) {
        if let Ok(s) = sim.world.get(r) {
            s.pos = at;
        }
    }
    if let Some(h) = hud {
        h.show(format!("{name} broken"));
    }
    info!("combat: {actor}'s {name} ({item}) broke and fell{}", dropped.map_or(String::new(), |r| format!(": reference {r}")));
    if let Some(b) = broken {
        b.write(WeaponBroken { form_id: actor.0, weapon: item, dropped });
    }
    Some(w)
}

/// A hit landed on the weapon `target` holds (§17): the hit's damage comes off the
/// weapon's condition; the actor is not hurt [design: the owner's rule, 2026-10-11].
/// The weapon is the one the renderer put in the hand (`PosedParts::weapon`); when the
/// simulation's inventory, rolled on its own, lacks it, one is added first [design].
#[allow(clippy::too_many_arguments)]
fn strike_weapon(
    forms: &CombatForms,
    sim: &mut SimState,
    stats: &mut CombatStats,
    feedback: &mut HitFeedback,
    hud: Option<&mut HudMessage>,
    broken: &mut MessageWriter<WeaponBroken>,
    script: Option<&mut ScriptState>,
    parts: &ActorParts,
    target: FormId,
    points: f32,
    at: V3,
) {
    let Some(item) = parts.by_ref.get(&target.0).and_then(|p| p.weapon) else { return };
    if sim.world.inventory_mut(target).is_ok_and(|i| i.count(item) == 0) {
        let _ = sim.world.add_item(target, item, 1);
    }
    stats.weapon_hits += 1;
    feedback.marker_age = Some(0.0);
    feedback.kill = false;
    match wear_weapon(forms, sim, stats, hud, Some(broken), script, target, item, points, Some(at)) {
        Some(w) => info!("combat: hit {target}'s {} for {points:.1}: condition {:.1}% -> {:.1}%", item_name(sim, item), w.before * 100.0, w.after * 100.0),
        None => info!("combat: hit {target}'s {} ({item}): it does not wear", item_name(sim, item)),
    }
}

/// The magic items a hit by `attacker` with `weapon` applies (`hit_effects`): the
/// weapon's object effect; for a blow without a weapon or with the Fists, the attacker's
/// unarmed attack effect.
/// Read inside the simulation's world: an attacker a script created (PlaceAtMe) is only
/// there. `anim`: a power attack's attack group ([`ActorStrike::anim`]).
#[allow(clippy::too_many_arguments)]
fn hit_items(c: &Combat, script: Option<&mut ScriptState>, sim: &mut SimState, stats: &mut CombatStats, attacker: FormId, weapon: Option<FormId>, anim: Option<&str>) -> Vec<FormId> {
    let Some(rt) = script.and_then(|s| s.rt.as_mut()) else { return Vec::new() };
    let unarmed = weapon.is_none() || weapon == c.fists;
    with_world(rt, sim, |rt| hit_effects::hit_items(&c.forms, &rt.game, Some(attacker), weapon, unarmed, anim, &mut stats.hit_effects))
}

/// Apply `dealt` health damage through the script runtime; report and remember it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_hit(
    script: Option<&mut ScriptState>,
    sim: &mut SimState,
    target: FormId,
    dealt: f32,
    info: HitInfo,
    stats: &mut CombatStats,
    feedback: &mut HitFeedback,
    mut hud: Option<&mut HudMessage>,
    messages: &mut MessageWriter<ActorHit>,
    effects: &[FormId],
    guard_broken: bool,
) -> Option<HitResult> {
    let mine = info.attacker == Some(FormId::PLAYER_REF);
    let rt = script.and_then(|s| s.rt.as_mut())?;
    if target == FormId::PLAYER_REF {
        return hit_player(rt, sim, dealt, info, stats, hud, effects);
    }
    // The hit's effects go on before its damage (`hit_effects`).
    with_world(rt, sim, |rt| hit_effects::apply(&mut rt.game, &mut stats.hit_effects, target, effects, info.attacker));
    let r = with_world(rt, sim, |rt| rt.game.hit(target, dealt as f64, info));
    let name = sim.ref_name(Some(rt.game.data), target.0).map_or_else(|| target.to_string(), str::to_string);
    if mine {
        stats.damage += r.dealt as f32;
    }
    if r.died {
        if mine {
            stats.kills += 1;
            stats.xp += r.xp;
            if let Some(h) = hud.as_deref_mut() {
                h.show(if r.xp > 0.0 { format!("{name} killed (+{} XP)", r.xp) } else { format!("{name} killed") });
            }
        }
        let by = info.attacker.filter(|a| *a != FormId::PLAYER_REF).map_or(String::new(), |a| format!(" by {a}"));
        info!("combat: {name} {target} killed{by}{}", if r.xp > 0.0 { format!(", +{} XP", r.xp) } else { String::new() });
    } else if r.knocked_out {
        info!("combat: {name} {target} knocked out (essential)");
    }
    if let Some((code, condition)) = r.limb {
        let limb = code.checked_sub(25).and_then(|i| LIMB_NAMES.get(i as usize)).copied().unwrap_or("limb");
        if r.crippled {
            info!("combat: {name} {target}: {limb} crippled");
            if let Some(h) = hud.filter(|_| !r.died && mine) {
                h.show(format!("{name}: {limb} crippled"));
            }
        } else {
            debug!("combat: {name} {target}: {limb} condition {condition:.0}");
        }
    }
    if !r.immune {
        if mine {
            feedback.marker_age = Some(0.0);
            feedback.kill = r.died;
            feedback.target = Some((name, r.health as f32, r.max_health as f32, 0.0));
        }
        messages.write(ActorHit { form_id: target.0, died: r.died, knocked_out: r.knocked_out, location: info.location, crippled: r.crippled, part_crippled: r.limb.is_some_and(|(_, c)| c <= 0.0), by: info.attacker, share: if r.max_health > 0.0 { (r.dealt / r.max_health) as f32 } else { 0.0 }, guard_broken });
    }
    Some(r)
}

/// God mode: the player takes no damage from hits, so cannot be killed by them
/// (settings page `god-mode`, `--god`, test aid `NVF_PLAY_TEST_GOD=1`) [design]. One
/// switch for the process: hits reach the player from several systems.
static GOD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether god mode is on.
pub fn god_mode() -> bool {
    GOD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Switch god mode on or off.
pub fn set_god_mode(on: bool) {
    GOD.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Another actor hits the player (module notes): the player's health takes the damage
/// through `Game::hit`, and at 0 the player dies.
fn hit_player(rt: &mut crate::script::Runtime, sim: &mut SimState, dealt: f32, info: HitInfo, stats: &mut CombatStats, hud: Option<&mut HudMessage>, effects: &[FormId]) -> Option<HitResult> {
    let by = info.attacker.map_or("?".to_string(), |a| sim.ref_name(Some(rt.game.data), a.0).map_or_else(|| a.to_string(), str::to_string));
    stats.player_hits += 1;
    if god_mode() {
        info!("combat: {by} hits the player for {dealt:.1} (god mode: no damage taken)");
        let health = with_world(rt, sim, |rt| fnv_game::functions::actor_value(&mut rt.game, FormId::PLAYER_REF, fnv_game::actor_values::HEALTH).unwrap_or(0.0));
        return Some(HitResult { health, immune: true, ..HitResult::default() });
    }
    with_world(rt, sim, |rt| hit_effects::apply(&mut rt.game, &mut stats.hit_effects, FormId::PLAYER_REF, effects, info.attacker));
    let r = with_world(rt, sim, |rt| rt.game.hit(FormId::PLAYER_REF, dealt as f64, info));
    stats.player_damage += r.dealt as f32;
    info!("combat: {by} hits the player for {:.1}, health {:.0}/{:.0}", r.dealt, r.health, r.max_health);
    if r.died {
        stats.player_died = true;
        info!("combat: the player died, killed by {by}");
        if let Some(h) = hud {
            h.show("You are dead.".to_string());
        }
    }
    Some(r)
}

/// Melee and unarmed attacks (see the module notes).
#[allow(clippy::too_many_arguments)]
fn melee(
    time: Res<Time>,
    mut c: ResMut<Combat>,
    mut sim: ResMut<SimState>,
    mut phys: ResMut<PhysState>,
    (player, intent): (Res<Player>, Res<MoveIntent>),
    control: Res<PlayerControl>,
    mut anim: ResMut<WeaponAnim>,
    mut stats: ResMut<CombatStats>,
    mut script: Option<ResMut<ScriptState>>,
    mut feedback: ResMut<HitFeedback>,
    mut hud: Option<ResMut<HudMessage>>,
    (mut messages, mut pushes, mut broken): (MessageWriter<ActorHit>, MessageWriter<ActorPush>, MessageWriter<WeaponBroken>),
    (parts, limbs): (Res<ActorParts>, Res<PlayerLimbs>),
    (sfx, mut sounds, mut impacts): (Option<Res<Sfx>>, MessageWriter<PlaySound>, MessageWriter<ImpactFx>),
) {
    let dt = time.delta_secs();
    let c = &mut *c;
    let Some(id) = c.weapon.filter(|_| c.melee) else { return };
    if !anim.ready {
        c.swings.reset();
        anim.swing = None;
        return;
    }
    let w = c.forms.weapons[&id].clone();
    // The clip the body timed for the swing playing; a new swing runs on the weapon's
    // own timing until the body has read its clip (the next frame).
    let speed = w.attack_mult.max(0.01) * w.anim_mult.max(0.01);
    if let Some(stem) = anim.timed.as_deref() {
        let t = &anim.timing;
        c.swings.set_timing(stem, SwingTiming::of(t.attack_len, t.hit_time, t.next_attack, speed, 1.0));
    }
    let fallback = Timing::of(&w, &ClipTiming::default(), &c.tuning).interval;
    let first = fnv_actor::player_clips::attack_stem(anim.attack.as_deref());
    let input = MeleeInput { trigger: control.trigger, dt, forward: intent.forward, right: intent.right, first: &first, rolls: [c.swing_rng.next_f32(), c.swing_rng.next_f32()] };
    let mut strike = None;
    for e in c.swings.tick(input, &c.tuning, SwingTiming::of(None, None, None, speed, fallback)) {
        match e {
            MeleeEvent::Started { stem, power, chained } => {
                if power.is_some() {
                    stats.power_attacks += 1;
                    // Without a clip to time it, a power attack takes longer [design].
                    if let Some(s) = c.swings.swing.as_mut() {
                        s.timing = SwingTiming::of(None, None, None, speed, fallback * c.tuning.power_time);
                    }
                }
                stats.swings += 1;
                stats.chained_swings += u32::from(chained);
                // The player's PowerAttack or Attack line (docs/spec-notes/combat-dialogue.md).
                if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
                    with_world(rt, &mut sim, |rt| rt.game.attack_line(FormId::PLAYER_REF, None, power.is_some(), false));
                }
                info!("combat: {} swing {stem}{}", w.edid.as_deref().unwrap_or("?"), if chained { " (chained)" } else { "" });
                anim.swing = Some(stem);
                anim.attacks += 1;
            }
            MeleeEvent::Strike { power } => strike = Some(power),
            MeleeEvent::Ended => {}
        }
    }
    if let Some(power) = strike {
        let power_mult = power.map_or(1.0, |d| d.damage(&c.rules));
        let reach = damage::melee_reach(&c.rules, &w, 1.0);
        let eye = player.eye();
        // A blow that meets no one but strikes the world within reach (a wall, the
        // ground ahead) shows its impact there.
        let mut struck_world: Option<WorldHit> = None;
        let hits = {
            let world = view(&mut phys);
            // Low and high: a short creature at the player's feet is struck within the
            // reach across the ground (`swing_reaching`).
            let hits = fnv_combat::projectile::swing_reaching(&world, &c.tuning, eye, player.feet[2], player.look_dir(), reach, PLAYER);
            let fan = fnv_combat::projectile::swing(&world, &c.tuning, eye, player.look_dir(), [0.0, 0.0, 1.0], reach, PLAYER).len();
            if hits.len() > fan {
                info!("combat: swing reaches {} actors (reach {reach:.0}), {} of them out of the fan of rays at eye height", hits.len(), hits.len() - fan);
            }
            if hits.is_empty() {
                let ahead = world.ray(eye, vec::add(eye, vec::scale(player.look_dir(), 400.0)), &[]);
                struck_world = ahead.clone().filter(|h| h.actor.is_none() && h.distance <= reach);
                info!(
                    "combat: swing misses (reach {reach:.0}); straight ahead: {}",
                    ahead.map_or("nothing within 400".to_string(), |h| format!("{} at {:.0}", h.actor.map_or("the world".to_string(), |a| format!("actor {a:08X}")), h.distance))
                );
            }
            hits
        };
        // A blow that lands wears the weapon, once however many it strikes (§17); the
        // blow itself is dealt at the condition the weapon had.
        let weapon_condition = item_condition(&sim, FormId::PLAYER_REF, id);
        // The blow's impacts, as a shot's (the owner's playtest list 5 item 24): blood on
        // the actors struck, dust or sparks on the world, with their sounds.
        if let Some(s) = sfx.as_deref() {
            let dir = player.look_dir();
            let landed = hits.iter().map(|(a, h)| (Some(*a), h)).chain(struck_world.iter().map(|h| (None, h)));
            for (actor, h) in landed {
                if h.part == Some(WEAPON_PART) {
                    continue;
                }
                let (impact, material) = impact_for(s, &sim, id, actor, h.material);
                match impact {
                    Some(impact) => {
                        stats.melee_impacts += 1;
                        debug!("combat: melee impact {impact} ({material}) at ({:.0}, {:.0}, {:.0})", h.point[0], h.point[1], h.point[2]);
                        show_impact(s, ImpactFx { impact, shooter: FormId::PLAYER_REF, point: h.point, normal: h.normal, dir }, &mut sounds, &mut impacts);
                    }
                    None => debug!("combat: {} has no impact for {material}", w.edid.as_deref().unwrap_or("?")),
                }
            }
        }
        if !hits.is_empty() {
            wear_weapon(&c.forms, &mut sim, &mut stats, hud.as_deref_mut(), None, script.as_deref_mut(), FormId::PLAYER_REF, id, condition::weapon_wear(&c.rules, &w, &[]), None);
        }
        for (actor, hit) in hits {
            let target = FormId(actor as u32);
            stats.melee_hits += 1;
            let (dt_v, dr) = target_armor(c, script.as_deref_mut(), &mut sim, target);
            let mut a = attacker(script.as_deref_mut(), &mut sim, &c.rules, FormId::PLAYER_REF, w.skill.as_deref(), false);
            a.arm_condition = limbs.arm_condition();
            let bp = body_part(c, &parts, actor, hit.part);
            let part = bp.as_ref().map_or_else(HitPart::default, BodyPart::hit_part);
            let h = HitInput { weapon: &w, projectiles: 1, condition: weapon_condition, ammo_effects: &[], part, power: power_mult };
            let roll = c.rng.next_f32();
            if hit.part == Some(WEAPON_PART) {
                let d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target::default(), &NoPerks, &NoPerks, roll);
                strike_weapon(&c.forms, &mut sim, &mut stats, &mut feedback, hud.as_deref_mut(), &mut broken, script.as_deref_mut(), &parts, target, d.raw, hit.point);
                continue;
            }
            let mut d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target { dt: dt_v, dr, ..Target::default() }, &NoPerks, &NoPerks, roll);
            // A person blocking it from the front stops part of it (combat-ai.md §14.3); a
            // power attack breaks through and staggers it (combat-rules.md §12).
            let mut guard_broken = false;
            if c.guarding.get(&target).is_some_and(|f| damage::from_front(*f, player.look_dir())) {
                let before = d.dealt;
                (d.dealt, guard_broken) = damage::against_block(&c.tuning, before, power.is_some());
                if guard_broken {
                    stats.blocks_broken += 1;
                    info!("combat: the player's power attack breaks {target}'s block ({before:.1}, all dealt)");
                } else {
                    stats.blows_blocked += 1;
                    info!("combat: {target} blocks the player's blow: {:.1} of {before:.1} stopped", before - d.dealt);
                    c.blocked_blows.push(BlockedBlow { attacker: FormId::PLAYER_REF, blocker: target, damage: before, dealt: d.dealt });
                }
            }
            let agility = actor_av(script.as_deref_mut(), &mut sim, target, fnv_game::actor_values::AGILITY).unwrap_or(5.0);
            let push = damage::knockback_force(&c.rules, d.dealt, agility);
            let location = bp.as_ref().map_or(0, BodyPart::location);
            let limb = limb_hit(bp.as_ref(), d.dealt, &w);
            let info = HitInfo { attacker: Some(FormId::PLAYER_REF), weapon: Some(id), location, critical: d.critical, limb };
            pushes.write(ActorPush { form_id: target.0, point: hit.point, dv: vec::scale(player.look_dir(), MELEE_PUSH * if power.is_some() { POWER_PUSH } else { 1.0 }) });
            let items = hit_items(c, script.as_deref_mut(), &mut sim, &mut stats, FormId::PLAYER_REF, Some(id), None);
            let r = apply_hit(script.as_deref_mut(), &mut sim, target, d.dealt, info, &mut stats, &mut feedback, hud.as_deref_mut(), &mut messages, &items, guard_broken);
            if r.as_ref().is_none_or(|r| !r.immune) {
                wear_armor(c, &mut sim, &mut stats, target, Zone::of(bp.as_ref()), d.raw);
            }
            match r {
                Some(r) => info!(
                    "combat: {}{} hits {target} on {} for {:.1} (DT {:.0}{}), health {:.0}/{:.0}, knockback {push:.0}",
                    w.edid.as_deref().unwrap_or("?"),
                    if power.is_some() { " power attack" } else { "" },
                    bp.as_ref().and_then(|b| b.name.as_deref()).unwrap_or("the body"),
                    r.dealt,
                    d.dt,
                    if d.critical { ", critical" } else { "" },
                    r.health,
                    r.max_health
                ),
                None => info!("combat: melee hit {target} for {:.1} (no script runtime: not applied)", d.dealt),
            }
        }
    }
}

/// Explosions set off per frame at most (chains of destructibles) [design].
const MAX_EXPLOSIONS_PER_FRAME: u32 = 64;
/// How far above and below an explosion the ground is looked for to show the impact of
/// its data set there, game units [design].
const EXPLOSION_GROUND_UP: f32 = 32.0;
const EXPLOSION_GROUND_REACH: f32 = 160.0;

/// Where a reference stands (its simulation state, else its placement).
fn ref_pos(sim: &SimState, r: FormId) -> Option<V3> {
    sim.world.peek(r).map(|s| s.pos).or_else(|| sim.refs.get(r).map(|p| p.pos))
}

/// Queue what a destruction change sets off and report the rest (step C7).
fn queue_destruction(c: &mut Combat, sim: &SimState, r: FormId, ch: &DestructionChange) {
    if ch.after == ch.before {
        return;
    }
    let at = ref_pos(sim, r).map(|p| [p[0], p[1], p[2] + DESTRUCTION_BLAST_HEIGHT]);
    info!(
        "combat: {r} destruction stage {} -> {}{}{}{}",
        ch.before,
        ch.after,
        if ch.destroyed { ", destroyed" } else { "" },
        if ch.disabled { ", disabled" } else { "" },
        if ch.explosions.is_empty() { String::new() } else { format!(", {} explosion(s)", ch.explosions.len()) }
    );
    if ch.model.is_some() || ch.damage_stage.is_some() {
        debug!("combat: {r} model {:?} damage stage {:?}: not drawn yet", ch.model, ch.damage_stage);
    }
    if let Some(at) = at {
        c.boom.extend(ch.explosions.iter().map(|&e| (e, at)));
    }
}

/// Object damage to a destructible reference through the script runtime.
fn damage_destructible(c: &mut Combat, sim: &mut SimState, script: Option<&mut ScriptState>, r: FormId, amount: f32) {
    let Some(rt) = script.and_then(|s| s.rt.as_mut()) else { return };
    let ch = with_world(rt, sim, |rt| rt.game.damage_object(r, amount as f64));
    if let Some(ch) = ch {
        queue_destruction(c, sim, r, &ch);
    }
}

/// Destructible references whose collision overlaps a sphere, with their distance.
fn destructibles_near(phys: &mut PhysState, sim: &SimState, script: Option<&ScriptState>, at: V3, radius: f32) -> Vec<(FormId, f32)> {
    let Some(rt) = script.and_then(|s| s.rt.as_ref()) else { return Vec::new() };
    phys.world.update_queries();
    let shape = fnv_collide::Shape::Sphere { radius };
    let pose = fnv_collide::Iso { translation: at, ..fnv_collide::Iso::IDENTITY };
    let owners = phys.world.queries().overlap(&shape, &pose, &Filter::default()).unwrap_or_default();
    let mut out: Vec<(FormId, f32)> = Vec::new();
    for o in owners {
        let Owner::Ref { form_id, .. } = o else { continue };
        let r = FormId(form_id);
        if out.iter().any(|x| x.0 == r) || !rt.game.is_destructible(r) {
            continue;
        }
        let d = ref_pos(sim, r).map_or(radius, |p| vec::len(vec::sub(p, at)).min(radius));
        out.push((r, d));
    }
    out
}

/// What a shot sends out: sounds, impact effects, explosions, broken weapons, and what
/// each impact picked (for `crate::world_sounds`).
type ShotEffects<'w> = (MessageWriter<'w, PlaySound>, MessageWriter<'w, ImpactFx>, MessageWriter<'w, ExplosionFx>, MessageWriter<'w, WeaponBroken>, MessageWriter<'w, crate::world_sounds::ImpactPicked>);

/// Fly the projectiles, deliver hits when due, set off explosions (and what they set
/// off: destruction stages, detonations).
/// What says whether the player blocks: its controls, its weapon's state, where it looks.
type Guard<'w> = (Option<Res<'w, PlayerControl>>, Option<Res<'w, WeaponAnim>>, Option<Res<'w, Player>>);

/// Other actors' attacks (module notes): their shots go into flight as the player's do,
/// their blows are dealt at once.
#[allow(clippy::too_many_arguments)]
fn actor_attacks(
    mut c: ResMut<Combat>,
    mut sim: ResMut<SimState>,
    mut phys: ResMut<PhysState>,
    mut stats: ResMut<CombatStats>,
    mut script: Option<ResMut<ScriptState>>,
    mut feedback: ResMut<HitFeedback>,
    mut hud: Option<ResMut<HudMessage>>,
    (mut shots, mut blows): (MessageReader<ActorFire>, MessageReader<ActorStrike>),
    (mut fired, mut sounds, mut messages, mut pushes): (MessageWriter<ShotFired>, MessageWriter<PlaySound>, MessageWriter<ActorHit>, MessageWriter<ActorPush>),
    sfx: Option<Res<Sfx>>,
    mut impacts: MessageWriter<ImpactFx>,
    (control, anim, player): Guard,
) {
    let c = &mut *c;
    let gravity = phys.world.params.gravity;
    // The player blocks with Block held and fists or a melee weapon drawn (combat.md §5.4);
    // NPC blows from its front are then blocked as an NPC's are (combat-ai.md §14.3).
    let player_guard = (c.melee && control.as_deref().is_some_and(|k| k.aiming) && anim.as_deref().is_some_and(|a| a.ready)).then(|| player.as_deref().map(|p| p.look_dir())).flatten().map(|d| [d[0], d[1]]);
    let script_shots = std::mem::take(&mut c.script_shots);
    for s in shots.read().chain(script_shots.iter()) {
        let carried = |a: FormId| sim.world.peek(s.shooter).and_then(|st| st.inventory.as_ref()).is_some_and(|i| i.count(a) > 0);
        let ammo = c.forms.ammo_for(s.weapon, carried);
        let Some(chain) = c.forms.chain(s.weapon, ammo) else { continue };
        let Some(proj) = chain.projectile else { continue };
        stats.actor_shots += 1;
        let range = c.forms.projectiles.get(&proj).map_or(0.0, |p| p.range);
        let reach = if range > 0.0 { range } else { AIM_REACH };
        let leaving = {
            let world = view(&mut phys);
            shot::leaving(&world, s.origin, s.aim, s.muzzle, reach, Some(s.shooter.0 as ActorId), false)
        };
        let seen_from = leaving.seen.at(leaving.origin, 0.0);
        fired.write(ShotFired { projectile: proj, weapon: s.weapon, shooter: s.shooter, at: Some(seen_from) });
        // Heard where it is fired: the weapon's 3D shoot sound.
        if let Some(sound) = sfx.as_deref().and_then(|x| x.forms.weapons.get(&s.weapon)).and_then(|f| f.shoot_3d.or(f.shoot_2d)) {
            sounds.write(PlaySound { sound, at: Some(seen_from) });
        }
        let (tracer_chance, force) = c.forms.projectiles.get(&proj).map_or((0.0, 0.0), |p| (p.tracer_chance, p.impact_force));
        let by = By { actor: s.shooter, weapon: s.weapon, ammo, harmless: s.harmless, vats: false };
        let rng = &mut c.rng;
        let mut rand = || rng.next_f32();
        let ps = shot::fire(&c.forms, &chain, leaving.origin, leaving.aim, s.cone, Some(s.shooter.0 as ActorId), gravity, &c.tuning, &mut rand);
        for p in ps {
            let rolled = c.rng.next_f32() < tracer_chance || c.tracer_hold.is_some();
            let shown = rolled || DRAW_EVERY_PROJECTILE;
            let tracer = rolled || (shown && instant(&p));
            let dir = vec::normalize(p.vel);
            c.next_visual += 1;
            c.flying.push(Flying { p, id: c.next_visual, proj, shown, dir, from_muzzle: s.muzzle.is_some(), origin: leaving.origin, seen: leaving.seen, tracer, projectiles_per_shot: chain.projectiles_per_shot, force, by });
        }
        debug!("combat: {} fires {} (cone {:.1}°)", s.shooter, c.forms.weapons.get(&s.weapon).and_then(|w| w.edid.as_deref()).unwrap_or("?"), s.cone);
    }
    for b in blows.read() {
        // The weapon: its own, else the Fists; a creature's own attack damage in the Fists'
        // place [design: the Fists' other figures stand in for a creature's].
        let Some(id) = b.weapon.or(c.fists) else { continue };
        let Some(mut w) = c.forms.weapons.get(&id).cloned() else { continue };
        if let Some(d) = b.creature_damage {
            w.base_damage = d.round() as i32;
        }
        stats.actor_strikes += 1;
        let a = attacker(script.as_deref_mut(), &mut sim, &c.rules, b.attacker, w.skill.as_deref(), false);
        let h = HitInput { weapon: &w, projectiles: 1, condition: item_condition(&sim, b.attacker, id), ammo_effects: &[], part: HitPart::default(), power: 1.0 };
        let (dt_v, dr) = target_armor(c, script.as_deref_mut(), &mut sim, b.target);
        let roll = c.rng.next_f32();
        let mut d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target { dt: dt_v, dr, ..Target::default() }, &NoPerks, &NoPerks, roll);
        // A blocker facing the blow stops part of it (docs/design/combat-ai.md §14.3); a
        // power attack (a creature's, `ActorStrike::anim`) breaks through (combat-rules.md
        // §12): an NPC staggers, the player cannot block for a while.
        let player_guard = player_guard.filter(|_| c.time >= c.guard_down_until);
        let guard = if b.target == FormId::PLAYER_REF { player_guard } else { c.guarding.get(&b.target).copied() };
        let mut guard_broken = false;
        if guard.is_some_and(|f| damage::from_front(f, b.dir)) {
            let before = d.dealt;
            (d.dealt, guard_broken) = damage::against_block(&c.tuning, before, b.anim.is_some());
            if guard_broken {
                stats.blocks_broken += 1;
                info!("combat: {}'s power attack breaks {}'s block ({before:.1}, all dealt)", b.attacker, b.target);
                if b.target == FormId::PLAYER_REF {
                    c.guard_down_until = c.time + c.tuning.guard_break;
                }
            } else {
                stats.blows_blocked += 1;
                info!("combat: {} blocks {}'s blow: {:.1} of {before:.1} stopped", b.target, b.attacker, before - d.dealt);
                c.blocked_blows.push(BlockedBlow { attacker: b.attacker, blocker: b.target, damage: before, dealt: d.dealt });
            }
        }
        let info = HitInfo { attacker: Some(b.attacker), weapon: b.weapon, location: 0, critical: d.critical, limb: None };
        if b.target != FormId::PLAYER_REF {
            pushes.write(ActorPush { form_id: b.target.0, point: b.point, dv: vec::scale(b.dir, MELEE_PUSH) });
        }
        if let Some(sound) = sfx.as_deref().and_then(|x| b.weapon.and_then(|w| x.forms.weapons.get(&w))).and_then(|f| f.shoot_3d.or(f.shoot_2d)) {
            sounds.write(PlaySound { sound, at: Some(b.point) });
        }
        // Its impact on the one struck, as the player's blows (item 24).
        if let Some(s) = sfx.as_deref() {
            if let (Some(impact), _) = impact_for(s, &sim, id, Some(b.target.0 as ActorId), None) {
                stats.melee_impacts += 1;
                show_impact(s, ImpactFx { impact, shooter: b.attacker, point: b.point, normal: vec::scale(b.dir, -1.0), dir: b.dir }, &mut sounds, &mut impacts);
            }
        }
        let items = hit_items(c, script.as_deref_mut(), &mut sim, &mut stats, b.attacker, b.weapon, b.anim);
        let r = apply_hit(script.as_deref_mut(), &mut sim, b.target, d.dealt, info, &mut stats, &mut feedback, hud.as_deref_mut(), &mut messages, &items, guard_broken);
        if r.as_ref().is_none_or(|r| !r.immune) {
            wear_armor(c, &mut sim, &mut stats, b.target, Zone::Body, d.raw);
        }
        if b.target != FormId::PLAYER_REF {
            if let Some(r) = r {
                info!("combat: {} strikes {} with {} for {:.1}, health {:.0}/{:.0}", b.attacker, b.target, w.edid.as_deref().unwrap_or("?"), r.dealt, r.health, r.max_health);
            }
        }
    }
}

/// The shot `FireWeapon` asks of `from` (`fnv_game::functions` fighting): from the
/// reference's centre straight ahead along its rotation, with no spread [documented:
/// GECK wiki FireWeapon, "from the center of the reference … straight out"]. The centre
/// is the reference's origin, an actor's raised to its middle (half of 128 units)
/// [hypothesis]; ahead is its heading (z, clockwise from north) tilted by its x angle,
/// a positive x looking down [hypothesis: the game turns both clockwise]. The model's
/// projectile node is not used yet.
fn script_shot(sim: &SimState, from: FormId, weapon: FormId) -> Option<ActorFire> {
    let s = sim.world.peek(from)?;
    let actor = from == FormId::PLAYER_REF || sim.world.placed.get(from).is_some_and(|p| matches!(&p.sig, b"ACHR" | b"ACRE"));
    let [x, _, z] = s.rot;
    let aim = [z.sin() * x.cos(), z.cos() * x.cos(), -x.sin()];
    let origin = [s.pos[0], s.pos[1], s.pos[2] + if actor { 64.0 } else { 0.0 }];
    Some(ActorFire { shooter: from, weapon, origin, aim, cone: 0.0, muzzle: None, harmless: false })
}

#[allow(clippy::too_many_arguments)]
fn fly(
    time: Res<Time>,
    mut c: ResMut<Combat>,
    mut sim: ResMut<SimState>,
    mut phys: ResMut<PhysState>,
    mut stats: ResMut<CombatStats>,
    mut script: Option<ResMut<ScriptState>>,
    mut feedback: ResMut<HitFeedback>,
    mut hud: Option<ResMut<HudMessage>>,
    mut messages: MessageWriter<ActorHit>,
    mut pushes: MessageWriter<ActorPush>,
    parts: Res<ActorParts>,
    mut visuals: ResMut<ShotVisuals>,
    anim: Res<WeaponAnim>,
    sfx: Option<Res<Sfx>>,
    mut effects: ShotEffects,
) {
    let dt = time.delta_secs();
    let c = &mut *c;
    // What scripts asked (DetonatePlacedExplosives) and what burns down (self-damage).
    if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
        let (requests, burning) = with_world(rt, &mut sim, |rt| (std::mem::take(&mut rt.game.combat_requests), rt.game.destruction_tick(dt as f64)));
        for req in requests {
            let actor = match req {
                CombatRequest::Detonate(actor) => actor,
                CombatRequest::Fire { from, weapon } => {
                    match script_shot(&sim, from, weapon) {
                        Some(shot) => c.script_shots.push(shot),
                        None => debug!("combat: FireWeapon on {from}: no such reference in the world"),
                    }
                    continue;
                }
            };
            let shooter = if actor == FormId::PLAYER_REF { Some(PLAYER) } else { Some(actor.0 as ActorId) };
            let mut n = 0;
            for f in c.flying.iter_mut().filter(|f| f.p.shooter == shooter) {
                if let Some(ProjEvent::Explode { explosion, point }) = f.p.detonate() {
                    c.boom.push((explosion, point));
                    n += 1;
                }
            }
            c.flying.retain(|f| f.p.alive);
            info!("combat: {actor} detonates placed explosives: {n}");
        }
        for (r, ch) in burning {
            queue_destruction(c, &sim, r, &ch);
        }
    }
    if c.flying.is_empty() && c.pending.is_empty() && c.boom.is_empty() && c.ghosts.is_empty() {
        if !visuals.flying.is_empty() {
            visuals.flying.clear();
        }
        return;
    }
    {
        let world = view(&mut phys);
        let hold = c.tracer_hold;
        for f in &mut c.flying {
            let before = f.p.pos;
            // Where it is drawn: from the muzzle, closing on its path (`Seen`).
            let seen_before = f.seen.at(before, f.p.travelled);
            // Found at once: what is drawn flies (or, a beam, stands) from there to
            // where it ended.
            let at_once = instant(&f.p) && f.p.age == 0.0;
            let fade = c.forms.projectiles.get(&f.proj).map_or(0.0, |p| p.fade_duration);
            let (id, proj, speed, beam) = (f.id, f.proj, f.p.speed, f.p.motion == Motion::Beam);
            // What stays on show after the shot (a beam, the line along the shot) keeps
            // its start on the muzzle: the attack clip moves the gun in the frames after
            // the shot [observed: the laser rifle's muzzle is 10 units higher one frame
            // after it fires], and a beam left where the muzzle was hangs below the barrel.
            // Only the player's own: the muzzle followed is the player's weapon's, so an
            // enemy's shot was drawn from the player's gun (the owner's playtest list 5
            // item 20).
            let follow = at_once && f.from_muzzle && f.by.actor == FormId::PLAYER_REF;
            let ghost = move |to: V3| Ghost { id, proj, from: seen_before, to, speed, travelled: 0.0, beam, left: fade, fresh: true, follow: follow && beam };
            let line = move |to: V3, life: f32| Mark { follow, ..Mark::tracer(seen_before, to, life) };
            for e in f.p.step(&world, dt) {
                match e {
                    ProjEvent::Hit { actor, part, object, point, delay, normal, material } => {
                        let from = if f.p.hitscan { f.origin } else { before };
                        let d = vec::sub(point, from);
                        let dir = if vec::len(d) > 1e-3 { vec::scale(d, 1.0 / vec::len(d)) } else { [0.0; 3] };
                        c.pending.push(Pending { due: c.time + delay, actor, part, object, point, dir, normal, material, projectiles: f.projectiles_per_shot, force: f.force, by: f.by });
                        if f.tracer {
                            debug!(
                                "combat: tracer from ({:.1}, {:.1}, {:.1}) to the hit at ({:.1}, {:.1}, {:.1})",
                                seen_before[0], seen_before[1], seen_before[2], point[0], point[1], point[2]
                            );
                            c.marks.push(line(point, hold.unwrap_or(0.08)));
                        }
                        if at_once && f.shown {
                            c.ghosts.push(ghost(point));
                        }
                    }
                    // A harmless shot sets nothing off [design].
                    ProjEvent::Explode { explosion, point } if !f.by.harmless => c.boom.push((explosion, point)),
                    ProjEvent::Explode { .. } => {}
                    // A tracer that meets nothing is drawn to where it ends.
                    ProjEvent::Expired { point } => {
                        if f.tracer {
                            c.marks.push(line(point, hold.unwrap_or(0.08)));
                        }
                        if at_once && f.shown {
                            c.ghosts.push(ghost(point));
                        }
                    }
                }
            }
            if f.tracer && f.p.alive {
                c.marks.push(Mark::tracer(seen_before, f.seen.at(f.p.pos, f.p.travelled), hold.unwrap_or(0.05)));
            }
            if vec::len(f.p.vel) > 1.0 {
                f.dir = vec::normalize(f.p.vel);
            }
        }
        c.flying.retain(|f| f.p.alive);
    }
    // What is drawn this frame: the projectiles in flight where they are seen, and
    // those found at once on their way.
    c.ghosts.retain_mut(|g| {
        if let (true, Some(m)) = (g.follow, anim.muzzle) {
            g.from = m;
        }
        if std::mem::take(&mut g.fresh) {
            return true;
        }
        if g.beam {
            g.left -= dt;
            g.left > 0.0
        } else {
            g.travelled += g.speed.max(1.0) * dt;
            g.travelled < vec::len(vec::sub(g.to, g.from))
        }
    });
    visuals.flying.clear();
    visuals.flying.extend(c.flying.iter().filter(|f| f.shown).map(|f| ShotVisual { id: f.id, projectile: f.proj, pos: f.seen.at(f.p.pos, f.p.travelled), dir: f.dir, beam: None, travelled: f.p.travelled }));
    visuals.flying.extend(c.ghosts.iter().map(|g| {
        let d = vec::sub(g.to, g.from);
        let len = vec::len(d);
        let dir = if len > 1e-3 { vec::scale(d, 1.0 / len) } else { [0.0, 1.0, 0.0] };
        ShotVisual { id: g.id, projectile: g.proj, pos: vec::add(g.from, vec::scale(dir, g.travelled)), dir, beam: g.beam.then_some(len), travelled: g.travelled }
    }));
    let weapon = c.weapon;
    // Explosions, with those they set off, a bounded number per frame.
    let mut n = 0;
    while let Some((id, at)) = c.boom.pop() {
        n += 1;
        if n > MAX_EXPLOSIONS_PER_FRAME {
            warn!("combat: more than {MAX_EXPLOSIONS_PER_FRAME} explosions in one frame; {} left for the next", c.boom.len() + 1);
            c.boom.push((id, at));
            break;
        }
        stats.explosions += 1;
        let Some(x) = c.forms.explosions.get(&id).cloned() else { continue };
        effects.2.write(ExplosionFx { explosion: id, point: at });
        if let Some(e) = sfx.as_deref().and_then(|s| s.forms.explosions.get(&id)) {
            for sound in e.sound_1.into_iter().chain(e.sound_2) {
                effects.0.write(PlaySound { sound, at: Some(at) });
            }
            // The impact of the EXPL's data set for the ground under it, when the ground
            // is near: the dirt it throws up (`ImpactExplosionDirt.nif`) [hypothesis:
            // that the game shows the EXPL's IPDS on what lies under the blast, as a
            // shot's on what it hits; the reach is a design choice].
            let ground = phys.ground_below(at, EXPLOSION_GROUND_UP, EXPLOSION_GROUND_REACH);
            let impact = e.impact_dataset.zip(ground.as_ref()).and_then(|(set, g)| sfx.as_deref()?.forms.impact_id(set, fnv_combat::fx::impact_material(g.material)));
            if let (Some(impact), Some(g)) = (impact, ground) {
                effects.1.write(ImpactFx { impact, shooter: FormId::PLAYER_REF, point: g.point, normal: g.normal, dir: [0.0, 0.0, -1.0] });
                if let Some(i) = sfx.as_deref().and_then(|s| s.forms.impacts.get(&impact)) {
                    for sound in i.sound_1.into_iter().chain(i.sound_2) {
                        effects.0.write(PlaySound { sound, at: Some(g.point) });
                    }
                }
            }
        }
        let hits = {
            let world = view(&mut phys);
            blast(&world, &c.tuning, &x, at)
        };
        let always = x.flags.iter().any(|f| f == "Knock Down - Always");
        let by_formula = x.flags.iter().any(|f| f == "Knock Down - By Formula");
        for b in hits {
            let target = FormId(b.actor as u32);
            let (dt_v, dr) = target_armor(c, script.as_deref_mut(), &mut sim, target);
            let dealt = (b.damage - dt_v).max(b.damage * c.tuning.dt_floor) * (1.0 - dr.clamp(0.0, c.rules.max_armor_rating) / 100.0);
            let info = HitInfo { attacker: Some(FormId::PLAYER_REF), weapon, location: -1, critical: false, limb: None };
            // The blast's object effect on those it catches (hit-effects.md §1).
            let items: Vec<FormId> = x.enchantment.into_iter().collect();
            let r = apply_hit(script.as_deref_mut(), &mut sim, target, dealt, info, &mut stats, &mut feedback, hud.as_deref_mut(), &mut messages, &items, false);
            if r.as_ref().is_none_or(|r| !r.immune) {
                wear_armor(c, &mut sim, &mut stats, target, Zone::All, b.damage);
            }
            if let Some(r) = r {
                // Knockdown [documented: the EXPL flags; Explosion Knockdown Formula].
                let agility = actor_av(script.as_deref_mut(), &mut sim, target, fnv_game::actor_values::AGILITY).unwrap_or(5.0);
                let chance = if r.died || r.immune {
                    0.0
                } else if always {
                    1.0
                } else if by_formula {
                    damage::knockdown_chance(&c.rules, r.dealt as f32, agility, (r.health + r.dealt) as f32, r.max_health as f32)
                } else {
                    0.0
                };
                let down = chance > 0.0 && c.rng.next_f32() < chance;
                if down {
                    stats.knockdowns += 1;
                }
                info!(
                    "combat: blast {} on {target} at {:.0}: {:.1} health damage, health {:.0}/{:.0}{}",
                    x.edid.as_deref().unwrap_or("?"),
                    b.distance,
                    r.dealt,
                    r.health,
                    r.max_health,
                    if down { ", knocked down (not animated yet)" } else { "" }
                );
            }
        }
        for (r, d) in destructibles_near(&mut phys, &sim, script.as_deref(), at, x.radius) {
            let dmg = damage::explosion_damage(&c.tuning, x.damage, x.radius, d);
            damage_destructible(c, &mut sim, script.as_deref_mut(), r, dmg);
        }
        let pushed = phys.world.push_dynamic(at, x.radius, x.force);
        c.marks.push(Mark::at(at, 0.6, MarkKind::Impact));
        info!(
            "combat: explosion {} at ({:.0}, {:.0}, {:.0}), {pushed} loose object(s) pushed{}",
            x.edid.as_deref().unwrap_or("?"),
            at[0],
            at[1],
            at[2],
            if x.radiation_level > 0.0 || x.placed_object.is_some() { " (radiation and placed object not built yet)" } else { "" }
        );
    }
    let now = c.time;
    let due: Vec<Pending> = {
        let (d, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut c.pending).into_iter().partition(|p| p.due <= now);
        c.pending = keep;
        d
    };
    for p in due {
        let id = p.by.weapon;
        let Some(w) = c.forms.weapons.get(&id).cloned() else { continue };
        let chain = c.forms.chain(id, p.by.ammo);
        let effect_ids = chain.map(|ch| ch.ammo_effects).unwrap_or_default();
        let shooter = p.by.actor;
        c.marks.push(Mark::at(p.point, 0.3, MarkKind::Impact));
        // The impact the weapon's data set names for what was hit: an actor's is its
        // base's Impact Material Type (NPC_ / CREA `NAM4`), organic without one
        // [design]; anything else's its collision material (docs/spec-notes/audio.md §11).
        if let Some(s) = sfx.as_deref() {
            let (impact, material) = impact_for(s, &sim, id, p.actor, p.material);
            if let Some(impact) = impact {
                show_impact(s, ImpactFx { impact, shooter, point: p.point, normal: p.normal, dir: p.dir }, &mut effects.0, &mut effects.1);
            }
            let hit = p.actor.map(|a| a as u32).or(p.object.map(|o| o as u32));
            effects.4.write(crate::world_sounds::log_impact(s, hit, p.actor.is_some(), p.material, material, impact));
        }
        // A harmless shot is seen and heard where it lands, and does nothing more.
        if p.by.harmless {
            stats.harmless_hits += 1;
            continue;
        }
        let ammo_effects: Vec<&AmmoEffect> = effect_ids.iter().filter_map(|e| c.forms.ammo_effects.get(e)).collect();
        let a = attacker(script.as_deref_mut(), &mut sim, &c.rules, shooter, w.skill.as_deref(), p.by.vats);
        let bp = p.actor.and_then(|a| body_part(c, &parts, a, p.part));
        let part = bp.as_ref().map_or_else(HitPart::default, BodyPart::hit_part);
        let h = HitInput { weapon: &w, projectiles: p.projectiles, condition: item_condition(&sim, shooter, id), ammo_effects: &ammo_effects, part, power: 1.0 };
        let roll = c.rng.next_f32();
        let Some(actor) = p.actor else {
            stats.world_hits += 1;
            debug!("combat: shot lands at ({:.0}, {:.0}, {:.0}) on {}", p.point[0], p.point[1], p.point[2], p.object.map_or("no object".to_string(), |o| format!("{:08X}", o as u32)));
            if let Some(obj) = p.object {
                let r = FormId(obj as u32);
                // The object's OnHit and OnHitWith blocks, before any damage
                // [documented: GECK wiki Event Handling] (combat-rules.md §14).
                let info = HitInfo { attacker: Some(shooter), weapon: Some(id), location: -1, critical: false, limb: None };
                let ran = script.as_deref_mut().and_then(|s| s.rt.as_mut()).is_some_and(|rt| with_world(rt, &mut sim, |rt| rt.game.hit_object(r, info)));
                // A destructible object takes the hit's damage (no armour) (step C7).
                let d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target::default(), &NoPerks, &NoPerks, roll);
                damage_destructible(c, &mut sim, script.as_deref_mut(), r, d.dealt);
                // A loose object is knocked away.
                let impulse = vec::scale(p.dir, p.force * OBJECT_PUSH * phys.world.params.units_per_metre);
                let pushed = phys.world.push_ref(obj as u32, p.point, impulse, OBJECT_PUSH_MAX_SPEED);
                stats.object_scripts += ran as u32;
                stats.objects_pushed += pushed.is_some() as u32;
                if ran || pushed.is_some() {
                    info!(
                        "combat: shot hits {} {r}{}{}",
                        sim.base_of(obj as u32).and_then(|b| sim.name(b).map(str::to_string)).unwrap_or_else(|| "object".into()),
                        if ran { ", its hit blocks ran" } else { "" },
                        pushed.map_or(String::new(), |v| format!(", knocked away at {v:.0} units/s"))
                    );
                }
            }
            continue;
        };
        stats.actor_hits += 1;
        let target = if actor == PLAYER { FormId::PLAYER_REF } else { FormId(actor as u32) };
        // A shot on the weapon the actor holds wears the weapon, not the actor (§17).
        if p.part == Some(WEAPON_PART) {
            let d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target::default(), &NoPerks, &NoPerks, roll);
            strike_weapon(&c.forms, &mut sim, &mut stats, &mut feedback, hud.as_deref_mut(), &mut effects.3, script.as_deref_mut(), &parts, target, d.raw, p.point);
            continue;
        }
        let (dt_v, dr) = target_armor(c, script.as_deref_mut(), &mut sim, target);
        let d = damage::hit(&c.rules, &c.tuning, &h, &a, &Target { dt: dt_v, dr, ..Target::default() }, &NoPerks, &NoPerks, roll);
        // Without body parts the whole capsule counts as the torso (location 0).
        let location = bp.as_ref().map_or(0, BodyPart::location);
        let limb = limb_hit(bp.as_ref(), d.dealt, &w);
        let info = HitInfo { attacker: Some(shooter), weapon: Some(id), location, critical: d.critical, limb };
        if target != FormId::PLAYER_REF {
            pushes.write(ActorPush { form_id: target.0, point: p.point, dv: vec::scale(p.dir, SHOT_PUSH) });
        }
        let items = hit_items(c, script.as_deref_mut(), &mut sim, &mut stats, shooter, Some(id), None);
        let r = apply_hit(script.as_deref_mut(), &mut sim, target, d.dealt, info, &mut stats, &mut feedback, hud.as_deref_mut(), &mut messages, &items, false);
        if r.as_ref().is_none_or(|r| !r.immune) {
            wear_armor(c, &mut sim, &mut stats, target, Zone::of(bp.as_ref()), d.raw);
        }
        match r {
            Some(r) => info!(
                "combat: hit {target} on {} for {:.1} (DT {:.0}{}), health {:.0}/{:.0}{}",
                bp.as_ref().and_then(|b| b.name.as_deref()).unwrap_or("the body"),
                r.dealt,
                d.dt,
                if d.critical { ", critical" } else { "" },
                r.health,
                r.max_health,
                if r.immune { ", takes no damage" } else { "" }
            ),
            None => {
                stats.damage += d.dealt;
                info!("combat: hit {target} for {:.1} (no script runtime: not applied)", d.dealt);
            }
        }
    }
}

fn draw_marks(time: Res<Time>, mut c: ResMut<Combat>, anim: Res<WeaponAnim>, show: Res<HitMarks>, mut gizmos: Gizmos) {
    let dt = time.delta_secs();
    // The test aid keeps its lines where they were drawn, for the record of a check.
    let follow = c.tracer_hold.is_none();
    let draw = show.show || c.tracer_hold.is_some();
    for m in &mut c.marks {
        m.age += dt;
        if let (true, true, Some(muzzle)) = (follow, m.follow, anim.muzzle) {
            m.from = muzzle;
        }
        if !draw {
            continue;
        }
        let fade = (1.0 - m.age / m.life).clamp(0.0, 1.0);
        match m.kind {
            MarkKind::Tracer => {
                gizmos.line(space::to_bevy(m.from), space::to_bevy(m.to), Color::srgba(1.0, 0.9, 0.6, fade));
            }
            MarkKind::Impact | MarkKind::Flash => {
                let p = space::to_bevy(m.to);
                let (s, col) = if m.kind == MarkKind::Flash { (FLASH_SIZE, Color::srgba(1.0, 0.95, 0.7, fade)) } else { (4.0, Color::srgba(1.0, 0.8, 0.3, fade)) };
                gizmos.line(p - Vec3::X * s, p + Vec3::X * s, col);
                gizmos.line(p - Vec3::Y * s, p + Vec3::Y * s, col);
                gizmos.line(p - Vec3::Z * s, p + Vec3::Z * s, col);
            }
        }
    }
    c.marks.retain(|m| m.age < m.life);
}

/// What the HUD shows of the weapon the player has equipped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudWeapon {
    /// The equipped weapon's condition, 0 to 1 (§17); `None` with bare hands.
    pub condition: Option<f32>,
    /// Rounds in the magazine and rounds carried besides, for a weapon that uses ammo;
    /// a weapon that spends itself (thrown) has its count first and 0 besides.
    pub ammo: Option<(u32, u32)>,
}

impl Combat {
    /// The equipped weapon as the HUD shows it (docs/spec-notes/hud.md §5).
    pub fn hud_weapon(&self, sim: &SimState) -> HudWeapon {
        let condition = self.equipped.flatten().map(|id| item_condition(sim, FormId::PLAYER_REF, id));
        let ammo = match (self.equipped.flatten(), self.cycle.as_ref()) {
            (Some(_), Some(cy)) if !cy.no_ammo => {
                let total = player_count(sim, self.ammo);
                Some((cy.magazine.min(total), total.saturating_sub(cy.magazine)))
            }
            (Some(id), Some(_)) if self.self_ammo => Some((player_count(sim, Some(id)), 0)),
            _ => None,
        };
        HudWeapon { condition, ammo }
    }
}

/// The Combat-tab lines said this frame (`fnv_game::combat_lines`), one log line each:
/// `combat line: <who> says <topic> line <info>`; tries without a line at debug level.
fn log_combat_lines(mut script: Option<ResMut<ScriptState>>, mut stats: ResMut<CombatStats>) {
    let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) else { return };
    let g = &mut rt.game;
    if g.combat_lines.said.is_empty() {
        return;
    }
    for (topic, who, line) in std::mem::take(&mut g.combat_lines.said) {
        match line {
            Some(l) => {
                stats.combat_lines += 1;
                info!("combat line: {} says {topic} line {:08X}", g.data.name(who), l.0);
            }
            None => {
                stats.combat_lines_none += 1;
                debug!("combat line: {} has no {topic} line", g.data.name(who));
            }
        }
    }
}

fn report(mut exit: MessageReader<AppExit>, stats: Res<CombatStats>) {
    if exit.read().last().is_some() {
        let s = &*stats;
        info!(
            "combat: {} shots ({} drawn from the muzzle, {} from behind cover), {} projectiles, {} actor hits, {} world hits ({} objects' hit blocks, {} objects knocked away), {} explosions ({} knockdowns), {} swings ({} chained, {} power, {} hits), {:.0} health damage, {} kills (+{} XP), {} reloads, {} dry fires, {} jams, {} melee impacts",
            s.shots, s.muzzle_shots, s.covered_shots, s.projectiles, s.actor_hits, s.world_hits, s.object_scripts, s.objects_pushed, s.explosions, s.knockdowns, s.swings, s.chained_swings, s.power_attacks, s.melee_hits, s.damage, s.kills, s.xp, s.reloads, s.dry_fires, s.jams, s.melee_impacts
        );
        info!("combat lines: {} said, {} tries without a line", s.combat_lines, s.combat_lines_none);
        info!("combat blocks: {} blows blocked, {} blocks broken by power attacks", s.blows_blocked, s.blocks_broken);
        info!(
            "condition summary: the player's weapons lost {:.1} health, {} hits on held weapons, {} weapons broken, armour lost {:.1} health, {} pieces worn to nothing",
            s.weapon_wear, s.weapon_hits, s.weapons_broken, s.armor_wear, s.armor_broken
        );
        info!("{}", hit_effects::summary(&s.hit_effects));
    }
}

/// Hit effects that ended this frame are logged (`hit_effects`).
fn watch_hit_effects(script: Option<Res<ScriptState>>, mut stats: ResMut<CombatStats>) {
    let Some(rt) = script.as_deref().and_then(|s| s.rt.as_ref()) else { return };
    hit_effects::watch(&rt.game, &mut stats.hit_effects);
    // A death by an effect's drain (a burn, a poison) rather than by a hit.
    if !stats.player_died && rt.game.is_dead(FormId::PLAYER_REF) {
        stats.player_died = true;
        info!("combat: the player died (no hit: an effect's drain)");
    }
}

#[derive(Component)]
struct HitMarker;

fn spawn_hit_hud(mut commands: Commands) {
    // The hit marker: a small square ring around the crosshair.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(14.0),
            height: Val::Px(14.0),
            margin: UiRect { left: Val::Px(-7.0), top: Val::Px(-7.0), ..default() },
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BorderColor::all(Color::NONE),
        HitMarker,
    ));
}

/// The hit marker [design]. The player's crippled limbs are `crate::hud_limbs`' picture.
fn update_hit_hud(time: Res<Time>, state: Res<State<PlayState>>, mut fb: ResMut<HitFeedback>, mut marker: Query<&mut BorderColor, With<HitMarker>>) {
    let dt = time.delta_secs();
    let playing = *state.get() == PlayState::Playing;
    let mut col = Color::NONE;
    if let Some(a) = fb.marker_age.as_mut() {
        *a += dt;
        if *a < MARKER_SECONDS && playing {
            col = if fb.kill { Color::srgb(1.0, 0.25, 0.2) } else { Color::srgba(1.0, 1.0, 1.0, 0.9) };
        }
    }
    if let Ok(mut b) = marker.single_mut() {
        *b = BorderColor::all(col);
    }
}

/// The cycle's clip timing from the attack clip (length, `Hit` or `Fire` key, and
/// whether a `Loop` key follows that key), the reload clip (length and `Hit` key) and the
/// length of the reload's opening clip, as the player body reads them; `next_attack`: the
/// attack clip's `a:` key, its time and text (`fnv_actor::player_clips::ClipKeys`).
pub fn clip_timing(attack: Option<(f32, Option<f32>, bool)>, reload: Option<(f32, Option<f32>)>, reload_open: Option<f32>, next_attack: Option<(f32, &str)>) -> ClipTiming {
    ClipTiming {
        next_attack: next_attack.and_then(|(t, v)| fnv_combat::melee::NextAttack::parse(v).map(|n| (t, n))),
        attack_len: attack.map(|a| a.0).filter(|l| *l > 0.0),
        hit_time: attack.and_then(|a| a.1),
        reload_len: reload.map(|r| r.0).filter(|l| *l > 0.0),
        reload_hit: reload.and_then(|r| r.1),
        reload_open_len: reload_open,
        spin_up: attack.filter(|a| a.2).and_then(|a| a.1),
        jam_len: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phys::NpcBody;

    /// An NPC walked by a character controller: its character (a man-sized capsule from
    /// the ground) and, inside it, a smaller body hovering (a bloatfly's hit volume).
    fn npc_world() -> PhysState {
        let mut phys = PhysState::default();
        let id = 0x1234;
        let ch = phys.world.add_character(id, fnv_phys::CharacterParams::default(), [0.0, 0.0, 0.0]);
        phys.npcs.insert(id, NpcBody { prev_feet: ch.feet, ch, velocity: [0.0; 2], sink: 0.0 });
        phys.world.set_actor_volume(id, fnv_phys::HitVolume { feet: [0.0, 0.0, 30.0], radius: 8.0, height: 30.0, dead: false });
        phys
    }

    #[test]
    fn shots_pass_an_npcs_character_and_strike_its_body() {
        // The playtest list 5 item 16: the character was struck as a wall (dust, no damage).
        let mut phys = npc_world();
        let world = view(&mut phys);
        let h = world.ray([-300.0, 0.0, 45.0], [300.0, 0.0, 45.0], &[PLAYER]).expect("the body is struck");
        assert_eq!(h.actor, Some(0x1234));
        assert!(h.point[0] > -10.0, "struck at the body, not the character's side: {:?}", h.point);
        // Below the body, through the character's legs: nothing.
        assert!(world.ray([-300.0, 0.0, 10.0], [300.0, 0.0, 10.0], &[PLAYER]).is_none());
    }

    #[test]
    fn jam_animations_beside_reload_animations() {
        assert_eq!(jam_anim("ReloadA").as_deref(), Some("JamA"));
        assert_eq!(jam_anim("ReloadZ").as_deref(), Some("JamZ"));
        assert_eq!(jam_anim("Reload"), None);
        assert_eq!(jam_anim("AttackLeft"), None);
    }
}
