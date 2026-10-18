//! The Layer 5 game state over the Layer 4 world: script instances and their variables,
//! quests, event dispatch, conditions, and the [`fnv_script::vm::Host`] that connects the
//! interpreter to all of it. Behaviour notes with evidence: docs/spec-notes/scripts.md
//! §4–7.

use crate::cond::{self, Condition, Operand, RunOn};
use crate::data::{Compiled, GameData, UnitId};
use crate::functions::{self, Args, Ctx, Kind};
use crate::quest::{QuestState, STAGE_COMPLETE, STAGE_FAIL};
use crate::trace::{Event, EventKind};
use fnv_script::ir::{Arg, Block, RefIndex, Stmt, VarRef, VarSlot};
use fnv_script::unit::{RefSlot, VarType};
use fnv_script::vm::{to_int_storage, ArgValue, Host, RunOpts, Value, Vm, VmError};
use fnv_sim::world::{Outcome, Snapshot};
use fnv_sim::{FormId, World};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The last hit an actor took (`crate::combat`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LastHit {
    pub attacker: Option<FormId>,
    pub weapon: Option<FormId>,
    /// GetHitLocation's index: −1 none, 0 torso, 1–2 head, 3–6 arms, 7–12 legs, 13 brain,
    /// 14 weapon [documented: catalogue notes of GetHitLocation].
    pub location: i32,
    pub critical: bool,
}

/// The furniture an actor is using (docs/spec-notes/furniture.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FurnitureUse {
    /// The furniture reference.
    pub reference: FormId,
    /// The ID of the model marker the actor entered by (`GetFurnitureMarkerID`).
    pub marker_id: u8,
    /// `GetSitting` / `GetSleeping`: 2 getting on, 3 on it, 4 getting off.
    pub state: u8,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

/// Begin-block event codes (docs/spec-notes/script-functions.md §3.3 [observed]).
pub mod event {
    pub const GAME_MODE: u16 = 0x00;
    pub const MENU_MODE: u16 = 0x01;
    pub const ON_ACTIVATE: u16 = 0x02;
    pub const ON_ADD: u16 = 0x03;
    pub const ON_EQUIP: u16 = 0x04;
    pub const ON_UNEQUIP: u16 = 0x05;
    pub const ON_DROP: u16 = 0x06;
    pub const SAY_TO_DONE: u16 = 0x07;
    pub const ON_HIT: u16 = 0x08;
    pub const ON_HIT_WITH: u16 = 0x09;
    pub const ON_DEATH: u16 = 0x0A;
    pub const ON_COMBAT_END: u16 = 0x0C;
    pub const ON_PACKAGE_START: u16 = 0x0F;
    pub const ON_PACKAGE_DONE: u16 = 0x10;
    pub const ON_PACKAGE_CHANGE: u16 = 0x14;
    pub const ON_LOAD: u16 = 0x15;
    pub const SCRIPT_EFFECT_START: u16 = 0x11;
    pub const SCRIPT_EFFECT_FINISH: u16 = 0x12;
    pub const SCRIPT_EFFECT_UPDATE: u16 = 0x13;
    pub const ON_DESTRUCTION_STAGE_CHANGE: u16 = 0x23;
    pub const ON_FIRE: u16 = 0x24;
    pub const ON_TRIGGER: u16 = 0x18;
    pub const ON_START_COMBAT: u16 = 0x19;
    pub const ON_RESET: u16 = 0x1E;
    pub const ON_TRIGGER_ENTER: u16 = 0x1A;
    pub const ON_TRIGGER_LEAVE: u16 = 0x1B;
}

/// Nested script runs allowed (a stage result script setting another stage, an
/// `Activate` running an OnActivate block) [documented: GECK wiki SetStage, at least 250
/// recursive SetStage calls work].
pub const MAX_DEPTH: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    /// Fixed frame step in seconds (deterministic traces).
    pub frame_seconds: f64,
    /// Unknown opcodes and undecodable pieces stop a script (tests) instead of being
    /// logged and skipped.
    pub strict: bool,
    /// Trace writes to an instance's own locals (off by default: timers write every
    /// frame). Writes to another script's variables and to globals are always traced.
    pub trace_locals: bool,
    pub seed: u64,
    pub max_steps: u64,
    /// A message box is a menu that holds GameMode until the player closes it
    /// ([`Game::show_box`]) [observed: layer5-captures.md §10.1]. Off: message boxes are
    /// only traced, as before round 4 (the headless runner for inputs that do not state
    /// the menus of their save, docs/spec-notes/scripts.md §8).
    #[serde(default = "yes")]
    pub modal_message_boxes: bool,
    /// The AI runs packages: actors around the player choose their package again every
    /// this many seconds ([`Game::tick`], `crate::ai`), and a package's begin, end and
    /// change scripts run when it starts, finishes and is replaced. `None` (the
    /// default): packages change only when a script or an input asks
    /// (`EvaluatePackage`, the runner's `package` input), as in Layer 5. The game's
    /// interval is about 10 seconds [documented: GECK wiki, How Are Packages Evaluated].
    #[serde(default)]
    pub package_interval: Option<f64>,
}

fn yes() -> bool {
    true
}

impl Default for GameConfig {
    fn default() -> Self {
        GameConfig { frame_seconds: 1.0 / 60.0, strict: false, trace_locals: false, seed: 0, max_steps: 100_000, modal_message_boxes: true, package_interval: None }
    }
}

/// Whose variables a script instance holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstKey {
    /// A placed or created reference whose base form has a script.
    Ref { reference: FormId },
    /// A quest's script.
    Quest { quest: FormId },
    /// A scripted item in an inventory (one instance per holder and item).
    Item { holder: FormId, item: FormId },
    /// The effect script of one active magic effect ([`ActiveEffect::id`]).
    Effect { id: u32 },
}

impl InstKey {
    pub fn label(&self) -> String {
        match self {
            InstKey::Ref { reference } => format!("ref:{reference}"),
            InstKey::Quest { quest } => format!("quest:{quest}"),
            InstKey::Item { holder, item } => format!("item:{item}@{holder}"),
            InstKey::Effect { id } => format!("effect:{id}"),
        }
    }
}

/// A script instance: the variables of one running copy of a script.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub script: Option<FormId>,
    pub vars: BTreeMap<u16, Value>,
    /// Clock time (1/64 s steps) at which the instance's per-frame block (GameMode,
    /// ScriptEffectUpdate) last started to run, and the start of the run before it:
    /// GetSecondsPassed is the difference (docs/spec-notes/scripts.md §5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub run_last: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub run_prev: Option<f64>,
}

/// Per-reference state the Layer 4 world does not keep.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RefExtra {
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub dead: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub destroyed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub ghost: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub restrained: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub unconscious: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub marked_for_delete: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub teammate: bool,
    /// SetCombatStyle's combat style for this reference, over its base form's
    /// (`Game::combat_style`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub combat_style: Option<FormId>,
    /// SetAlert's alerted animation state (weapon drawn) [documented: GECK wiki SetAlert,
    /// savebaked].
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub alerted: bool,
    /// SetForceSneak's forced sneaking [documented: GECK wiki SetForceSneak, "Turns force
    /// sneak on or off"].
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub force_sneak: bool,
    /// Faction rank changes: a rank, or −1 for "removed".
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub factions: BTreeMap<FormId, i8>,
    /// SetActorValue's number in place of an actor value's base, by code. Not for
    /// S.P.E.C.I.A.L. of actors whose attributes the world holds (`functions::actor`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub actor_values: BTreeMap<u16, f64>,
    /// The permanent modifier per actor value code (ModActorValue, ForceActorValue),
    /// S.P.E.C.I.A.L. included.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub av_permanent: BTreeMap<u16, f64>,
    /// Damage taken per actor value code (DamageActorValue; RestoreActorValue heals it),
    /// S.P.E.C.I.A.L. included. The current value is the permanent value minus this.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub av_damage: BTreeMap<u16, f64>,
    /// The temporary modifier per actor value code: the Value Modifier effects with
    /// Recover active on the actor (`crate::aid`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub av_temporary: BTreeMap<u16, f64>,
    /// Packages added with AddScriptPackage (the last one is current).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub packages: Vec<FormId>,
    /// The AI package the actor chose when its packages were last evaluated
    /// ([`Game::evaluate_package`]); `None` until then.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_package: Option<FormId>,
    /// The current package reached its goal ([`Game::package_goal_met`]).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub package_goal_met: bool,
    /// Once Per Day packages: GameDaysPassed when the actor last chose each.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub package_chosen_at: BTreeMap<FormId, f32>,
    /// Game time ([`Game::time`]) of the actor's next periodic package choice.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub package_due: Option<f64>,
    /// Packages whose location the actor could not reach: not chosen until
    /// GameDaysPassed reaches the value ([`Game::block_package`]).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub package_blocked: BTreeMap<FormId, f32>,
    /// The current package (first) ends when GameDaysPassed reaches the second
    /// ([`Game::end_timed_packages`]): set by SetPackDuration, or when a package with a
    /// duration and no starting hour is chosen (`Game::set_package`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub package_ends: Option<(FormId, f64)>,
    /// The `package_ends` timer is the package's own schedule duration (a package with
    /// no starting hour), not SetPackDuration's: Must Complete and Continue If PC Near
    /// hold it ([`Game::end_timed_packages`]).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub package_ends_by_schedule: bool,
    /// SetActorsAI 0: the actor's AI does not run (no package choice, no detection, no
    /// walking); its scripts still do.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub ai_off: bool,
    /// SetActorRefEssential (the reference only; the base's flag is separate).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub essential: Option<bool>,
    /// Who killed the actor (KillActor's killer), for IsKiller.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub killer: Option<FormId>,
    /// KillActor's cause of death, which GetCauseofDeath reads before the last hit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub death_cause: Option<i32>,
    /// The furniture the actor is on, getting on or getting off.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub furniture: Option<FurnitureUse>,
    /// The item (base form) the actor last used or took under its package (a Find or
    /// Use Item At package), until its package changes; with the furniture it is on, what
    /// GetIsUsedItem and its kin read (`functions::furniture`). Not saved.
    #[serde(skip)]
    pub used_item: Option<FormId>,
    /// The last hit taken (combat), for GetHitLocation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_hit: Option<LastHit>,
    /// The actor it last hit (GetOwnerLastTarget, `crate::unconscious`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_target: Option<FormId>,
    /// Knocked out: an essential actor at zero health, or its Fatigue ran out
    /// (`crate::unconscious`). Not SetUnconscious's flag (`unconscious`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub knocked_out: Option<crate::unconscious::KnockOut>,
    /// The actor has its weapon out (IsWeaponOut): told by whoever animates the actor
    /// ([`Game::set_weapon_out`]).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub weapon_out: bool,
    /// The weapon the animating side draws for an actor whose inventory here has none
    /// equipped (an NPC's weapon as its appearance resolves it), for GetWeaponAnimType
    /// ([`Game::set_held_weapon`]). Not saved: told again when the actor is drawn.
    #[serde(skip)]
    pub held_weapon: Option<FormId>,
    /// Health damage taken from the player, and from anyone (kill XP's share rule).
    #[serde(skip_serializing_if = "is_zero_f64", default)]
    pub player_damage: f64,
    #[serde(skip_serializing_if = "is_zero_f64", default)]
    pub total_damage: f64,
    /// Object damage taken (DamageObject), for the destruction stage.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub damage: Option<f64>,
    /// Self-damage per second of the destruction stage reached (burning down).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub burning: Option<f64>,
    /// Spells added with AddSpell, in order (the authored abilities among them).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub spells: Vec<FormId>,
    /// The actor's authored abilities (record and race SPLO) were given
    /// (`crate::abilities`), and the race they were given for.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub abilities_given: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ability_race: Option<FormId>,
    /// Abilities a Dispel took off while the actor is loaded (they come back when it is
    /// loaded again), and worn items whose enchantment a Dispel took off (until the item
    /// is unequipped).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dispelled: Vec<FormId>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub worn_dispelled: Vec<FormId>,
    /// SetBroadcastState of a radio station (talking activator).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub broadcast: Option<bool>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub ignore_crime: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub ignore_friendly_hits: bool,
    /// SetTalkingActivatorActor.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub talking_actor: Option<FormId>,
    /// SetCriticalStage (0 none, 1 goo start, 2 goo end, 3 disintegrate start, 4 end).
    #[serde(skip_serializing_if = "is_zero", default)]
    pub critical_stage: u16,
    /// SetScale.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scale: Option<f64>,
    /// SetLevel of an actor other than the player.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub level: Option<i32>,
    /// SetMinimalUse of a door.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub minimal_use: Option<bool>,
    /// The actor has been in a conversation with the player (GetTalkedToPC).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub talked_to_pc: bool,
    /// SetDisposition: the disposition towards an actor, set by a script
    /// (`functions::actor::get_disposition`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub dispositions: BTreeMap<FormId, f64>,
    /// The special idle the actor plays (IsIdlePlaying): told by whoever animates the
    /// actor ([`Game::set_idle_playing`]); not part of the saved state.
    #[serde(skip)]
    pub idle: Option<FormId>,
    /// The special idle the actor last played (IsLastIdlePlayed): the last one told
    /// ([`Game::set_idle_playing`]); not part of the saved state.
    #[serde(skip)]
    pub last_idle: Option<FormId>,
    /// How many times ResurrectActor rolled the actor's random choices again (its leveled
    /// template and carried items, `crate::resurrect`); the seed of its picks
    /// (`fnv_sim::leveled::roll_seed`).
    #[serde(skip_serializing_if = "is_zero_u32", default)]
    pub rolls: u32,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

/// A challenge's run-time state (UnlockChallenge, RemoveRecurringFromChallenge, the
/// completion the Layer 6 challenge counters report).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChallengeState {
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub unlocked: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub completed: bool,
    /// RemoveRecurringFromChallenge cleared the Recurring flag.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub not_recurring: bool,
    /// Progress towards the threshold (IncrementScriptedChallenge).
    #[serde(skip_serializing_if = "is_zero_u32", default)]
    pub progress: u32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// EnableFastTravel's three flags (all false = the default: travel and waiting allowed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FastTravel {
    pub travel_disabled: bool,
    pub wait_disabled: bool,
    /// The setting survives an interior → exterior change.
    pub keep: bool,
}

impl FastTravel {
    pub fn is_default(&self) -> bool {
        *self == FastTravel::default()
    }
}

/// Run-time changes to base forms and cells (they apply to every reference of the form)
/// [documented per function: catalogue summaries of SetEssential, SetActorFullName,
/// SetQuestObject, SetCellOwnership, SetCellPublicFlag, SetItemValue, SetEnemy, SetAlly].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FormChanges {
    /// Faction → other faction → group combat reaction (0 neutral, 1 enemy, 2 ally,
    /// 3 friend), over the FACT XNAM list.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub faction_reactions: BTreeMap<FormId, BTreeMap<FormId, i64>>,
    /// SetEssential on actor bases.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub essential: BTreeMap<FormId, bool>,
    /// SetActorFullName: base → the message whose title is the new name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub names: BTreeMap<FormId, FormId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub quest_items: BTreeMap<FormId, bool>,
    /// SetCellOwnership (None: the owner was cleared).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub cell_owners: BTreeMap<FormId, FormId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub public_cells: BTreeMap<FormId, bool>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub item_values: BTreeMap<FormId, i32>,
}

/// A magic effect active on an actor (docs/spec-notes/scripts.md §7.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveEffect {
    pub id: u32,
    pub target: FormId,
    /// The magic item it came from (ALCH, SPEL, ENCH, INGR).
    pub source: FormId,
    /// The base effect (MGEF).
    pub effect: FormId,
    /// Its index in the source's effect list.
    pub index: usize,
    pub magnitude: u32,
    /// Seconds; ignored when `permanent`.
    pub duration: f64,
    pub elapsed: f64,
    /// An ability added with AddSpell: lasts until removed.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub permanent: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caster: Option<FormId>,
    /// The effect script (MGEF archetype Script), if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub script: Option<FormId>,
    /// What it does to an actor value (Value Modifier, Value And Parts): `crate::aid`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub change: Option<crate::aid::ValueChange>,
    /// The usage monitor of an ingestible: the magnitudes of its uses while it lasts
    /// (GetSpellUsageNum; docs/spec-notes/aid-items.md §5).
    #[serde(skip_serializing_if = "is_zero_f64", default)]
    pub usage: f64,
    /// A worn enchantment: the equipped item it comes from (`crate::abilities`); it
    /// lasts while the item is worn.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub worn: Option<FormId>,
    /// An ability's or worn enchantment's effect whose conditions fail: listed, with no
    /// value change and no script running (aid-items.md §13).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub inactive: bool,
}

/// An image space modifier playing on the screen (docs/spec-notes/image-modifiers.md §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveImageModifier {
    /// The IMAD record.
    pub modifier: FormId,
    /// Seconds since it was applied ([`Game::image_modifiers`] says which seconds count).
    pub elapsed: f64,
}

/// What the last ShowAllMapMarkers asked for (docs/design/script-stubs.md §3.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllMarkers {
    /// Every marker is on the map (1), or the call took them off (0).
    pub shown: bool,
    /// Fast travel to them is allowed (no fast travel is built yet; kept for it).
    pub travel: bool,
    /// Markers flagged "Show All" Hidden are left out.
    pub skip_hidden: bool,
}

/// What the race menu (1036) lets the player change, by the function that opened it:
/// ShowRaceMenu, ShowBarberMenu and ShowPlasticSurgeonMenu all open menu 1036
/// [documented: GECK wiki "MenuMode (Function)", notes: "ShowRaceMenu,
/// ShowPlasticSurgeonMenu, and ShowBarberMenu all return true for MenuMode 1036"].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RaceMenu {
    /// ShowRaceMenu: everything (the character creator).
    #[default]
    Creator,
    /// ShowBarberMenu: the hair [documented: GECK wiki ShowBarberMenu, "select hair color
    /// and style"] and the beard [owner 2026-10-17, todo 6]; not the race, sex or name
    /// [owner].
    Barber,
}

/// The player's state outside the world's reference table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub xp: f64,
    /// The name typed in the name menu (`crate::chargen`); `None` before it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub perks: BTreeMap<FormId, u8>,
    /// Perks given to the player's teammates (AddPerk with forTeammates).
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub teammate_perks: BTreeSet<FormId>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub notes: BTreeSet<FormId>,
    /// Infamy (index 0) and fame (index 1) per reputation, as GetReputation's axis.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub reputation: BTreeMap<FormId, [f64; 2]>,
    /// `SexChange`: true = female.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub female: Option<bool>,
    /// The look chosen in the character creator (the race menu, `crate::chargen::Look`);
    /// `None` keeps the player record's.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub look: Option<crate::chargen::Look>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub tag_skills: BTreeSet<u16>,
    /// The points spent on each skill in the level-up menu (`crate::levelup`), added to
    /// the skill's formula [documented: GECK wiki ActorValue, the player's spent skill
    /// points sit on top of the derived base; docs/spec-notes/levelup.md §4].
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub skill_points: BTreeMap<u16, f64>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub achievements: BTreeSet<i32>,
    /// Actors killed, by base form (GetDeadCount).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub dead_count: BTreeMap<FormId, u32>,
    /// Disabled control groups: movement, Pip-Boy, fighting, POV, looking, rollover
    /// text, sneaking.
    #[serde(default)]
    pub controls_disabled: [bool; 7],
    /// SetHardcore (otherwise the configuration's).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hardcore: Option<bool>,
    /// The stage record (RADS, DEHY, HUNG, SLPD) whose effect the player has, by the
    /// need's actor value code (`crate::needs`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub need_stages: BTreeMap<u16, FormId>,
    /// Miscellaneous statistics by code (ModPCMiscStat).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub misc_stats: BTreeMap<u16, f64>,
    /// Map markers ShowMap made visible.
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub map_markers: BTreeSet<FormId>,
    /// Map markers ShowMap made fast-travel targets (its canTravel argument).
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub travel_markers: BTreeSet<FormId>,
    /// Map markers the player found by coming near them: the engine's discovery
    /// (`nvf-play` compass), told to the rules (`Game::set_found_markers`).
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub found_markers: BTreeSet<FormId>,
    /// ShowAllMapMarkers: every marker shown at once; `None` before any call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_markers: Option<AllMarkers>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub challenges: BTreeMap<FormId, ChallengeState>,
    #[serde(skip_serializing_if = "FastTravel::is_default", default)]
    pub fast_travel: FastTravel,
    /// Factions the player is an enemy of after a crime (SetPCEnemyOfFaction).
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub enemy_of: BTreeSet<FormId>,
    /// SetInCharGen.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub in_chargen: bool,
    /// Casino winnings level (0–4) per casino (CSNO), set by the casino games (Layer 6).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub casino_levels: BTreeMap<FormId, i32>,
    /// The net chips won at each casino over every game (`crate::casino`; a ban at the
    /// casino's max winnings).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub casino_earnings: BTreeMap<FormId, i64>,
    /// The roulette help message has been shown (it comes the first time only).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub roulette_help_shown: bool,
    /// The blackjack help message has been shown (it comes the first time only).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub blackjack_help_shown: bool,
    /// SetPCCanUsePowerArmor.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub power_armor: bool,
    /// Dialogue topics added with AddTopic.
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub topics: BTreeSet<FormId>,
    /// SetLevel on the player (otherwise the configuration's level).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub level: Option<i32>,
    /// Hardcore mode was off at some point (IsAlwaysHardcore).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub hardcore_was_off: bool,
    /// The player murdered a non-Evil NPC (IsPCAMurderer; `crate::crime`).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub murderer: bool,
    /// Crimes against each actor: [minor, major] (GetMinorCrimeCount, GetMajorCrimeCount).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub crime_counts: BTreeMap<FormId, [u32; 2]>,
    /// Actors that are the player's enemies because of crimes against them or seen by
    /// them (GetActorCrimePlayerEnemy; `crate::crime`).
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub crime_enemies: BTreeSet<FormId>,
}

/// A message box waiting for a button (`ShowMessage` of a message with buttons).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageBox {
    pub message: FormId,
    pub buttons: usize,
    /// The button chosen (runner input), returned once by `GetButtonPressed`.
    pub pressed: Option<i32>,
    /// The script that showed the box (its instance, or the unit of a result script):
    /// only that script's `GetButtonPressed` receives the button; any other script
    /// reads −1 and leaves the box alone [hypothesis, from the GECK wiki's ShowMessage
    /// and GetButtonPressed pages: the script that shows a message box polls its button].
    /// `None` (a box from an older snapshot): the first caller takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

impl MessageBox {
    /// The identity of a calling script for [`MessageBox::owner`].
    pub fn owner_of(inst: Option<InstKey>, unit: &str) -> String {
        match inst {
            Some(k) => k.label(),
            None => unit.to_string(),
        }
    }
}

/// Menu codes (`MenuMode <code>`) of the menus scripts open [observed: each code was
/// open right after the stage whose result script calls the function, save `nvfnew`,
/// docs/spec-notes/layer5-captures.md §10.4].
pub mod menus {
    /// A message box (ShowMessage of a message with the Message Box flag).
    pub const MESSAGE: u16 = 1001;
    /// The race menu (ShowRaceMenu) [observed: layer5-captures.md §10.3].
    pub const RACE: u16 = 1036;
    /// The tag-skill menu (SetTagSkills).
    pub const TAG_SKILLS: u16 = 1048;
    /// The name menu (GetPlayerName).
    pub const NAME: u16 = 1051;
    /// The Vigor Tester's SPECIAL menu (ShowLoveTesterMenuParams).
    pub const LOVE_TESTER: u16 = 1074;
    /// The trait menu (ShowTraitMenu).
    pub const TRAITS: u16 = 1084;
    /// The level-up menu (AdvancePCLevel; `crate::levelup`) [documented: GECK wiki
    /// MenuMode, the example "if MenuMode 1027 ; level-up menu is active"; not captured].
    pub const LEVEL_UP: u16 = 1027;
    /// The lockpicking menu [documented: GECK wiki GetMenuTargetRef, owner export: "1014
    /// The door/container being picked"].
    pub const LOCKPICK: u16 = 1014;
    /// The barter menu (ShowBarterMenu) [hypothesis: the GECK wiki's MenuMode list as
    /// remembered; no shipped script reads it and the owner's export lacks the page].
    pub const BARTER: u16 = 1053;
    /// The dialogue menu [documented: GECK wiki GetMenuTargetRef, owner export; not
    /// captured]. It is not kept in `Game::menu`: the conversation itself
    /// (`Game::dialogue`) is the open menu, under any other menu that opens during it
    /// (`Game::in_menu`, `Game::open_menu_code`, `Game::menu_open`).
    pub const DIALOGUE: u16 = crate::dialogue::DIALOGUE_MENU;
}

/// `merchant.ShowBarterMenu [discount]`: trade with this actor. `discount` is a percentage
/// in -100 to 100 that lowers the prices the merchant sells at and raises those it buys
/// at; 0 leaves them [documented: GECK wiki ShowBarterMenu, owner export].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarterRequest {
    pub merchant: FormId,
    pub discount: i32,
}

/// A dialogue line being said outside a conversation (SayTo, Say, or a greeting with the
/// Goodbye flag, `Game::start_dialogue`): docs/spec-notes/scripts.md §7.7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speech {
    pub speaker: FormId,
    /// The listener (SayTo's target; none for Say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<FormId>,
    pub topic: FormId,
    pub info: FormId,
    /// Seconds of game time left until the line ends.
    pub remaining: f64,
    /// Said by SayTo (else by Say). The speaker's SayToDone blocks run when either ends
    /// [documented: level-design.md §11.2 D1].
    #[serde(default)]
    pub say_to: bool,
    /// A greeting with the Goodbye flag, said at the player without a conversation
    /// (`Game::start_dialogue`, docs/spec-notes/dialogue.md §5): when it ends its Add
    /// Topics are applied and no SayToDone block runs [hypothesis for both].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub greeting: bool,
    /// A line of a conversation between two actors (StartConversation with an actor as
    /// the target, `crate::dialogue`, docs/spec-notes/dialogue.md §5.1): its number in
    /// the conversation, from 1. Only its first response is said; when it ends the
    /// conversation goes on with the next line, and no SayToDone block runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<u32>,
    /// A hello said to the player as the player passes (`Game::say_hello`,
    /// docs/spec-notes/dialogue.md §5.2): when it ends no SayToDone block runs and no
    /// topic is added [hypothesis for both].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hello: bool,
    /// Who the line turns to whom ([`Game::facings`]).
    #[serde(default, skip_serializing_if = "LineLook::is_both")]
    pub look: LineLook,
    /// Position (in the order the responses are said) of the response whose idles were
    /// last asked for (`crate::idles`); not part of the saved state.
    #[serde(skip)]
    pub response: usize,
}

/// Who a line said to someone ([`Speech::target`]) turns to whom ([`Game::facings`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineLook {
    /// The speaker turns to the listener and the listener looks at the speaker.
    #[default]
    Both,
    /// The listener is not made to look at the speaker: SayTo's `noTargetLookFlag`
    /// [documented: GECK wiki SayTo, owner export: with the flag set "Joe is not forced
    /// to look at the speaker during the SayTo"].
    Speaker,
    /// Nobody turns: a conversation between actors started with `headTrack` 0
    /// [documented: GECK wiki StartConversation: "Enter 0 to disable" head tracking
    /// during the conversation].
    Nobody,
}

impl LineLook {
    pub fn is_both(&self) -> bool {
        *self == LineLook::Both
    }
}

/// One actor turning to another while a line is said ([`Game::facings`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facing {
    /// Who turns, and to whom (either may be the player).
    pub who: FormId,
    pub to: FormId,
    /// The body turns too, where it stands free (else the head only).
    pub body: bool,
    /// `who` says the line (else it listens).
    pub speaking: bool,
    /// The line (INFO).
    pub info: FormId,
}

/// What a line said outside a conversation with the player is ([`Speech`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Say,
    SayTo,
    /// A greeting with the Goodbye flag ([`Speech::greeting`]).
    Greeting,
    /// Line `n` of a conversation between two actors ([`Speech::conversation`]).
    Conversation(u32),
    /// A hello to the passing player ([`Speech::hello`]).
    Hello,
}

pub struct Game<'a> {
    pub data: &'a GameData,
    /// The console's world toggles (`functions::toggles`).
    pub toggles: crate::functions::toggles::Toggles,
    /// What scripts asked of the engine's combat (`crate::combat::CombatRequest`), for
    /// the engine to take.
    pub combat_requests: Vec<crate::combat::CombatRequest>,
    /// Knockouts and getting up since the engine last took them
    /// (`crate::unconscious::News`): actor, cause, `true` when it got up.
    pub knockout_news: Vec<crate::unconscious::News>,
    /// The idle animations actors were asked to play (`crate::idles::IdleRequest`), for
    /// the layer that animates actors to take.
    pub idle_requests: Vec<crate::idles::IdleRequest>,
    /// Actors' weapon uses (`crate::use_weapon`: UseWeapon, Use Weapon packages), and
    /// the last number given to one. Not part of the saved state.
    pub weapon_uses: BTreeMap<FormId, crate::use_weapon::WeaponUse>,
    pub weapon_use_seq: u64,
    /// The sounds scripts asked for (`crate::sound::SoundRequest`), for the engine to
    /// take. Not part of the saved state.
    pub sound_requests: Vec<crate::sound::SoundRequest>,
    /// The music types scripts asked for (`PlayMusic`, `crate::sound`), for the engine to
    /// take. Not part of the saved state.
    pub music_requests: Vec<FormId>,
    /// The textures scripts put on references' shapes (`SwapTexture`,
    /// `crate::functions::misc::swap_texture`), for the engine to draw. Not part of the
    /// saved state.
    pub texture_swaps: crate::functions::misc::TextureSwaps,
    /// Effect shaders and actor alpha scripts asked for (`crate::functions::misc::ScriptLooks`),
    /// for the engine to draw. Not part of the saved state.
    pub script_looks: crate::functions::misc::ScriptLooks,
    /// Animation groups scripts asked objects to play, and what the engine reports they
    /// play (`crate::anims`). Not part of the saved state.
    pub anims: crate::anims::ObjectAnims,
    /// Barter menus scripts opened (ShowBarterMenu), for the engine to draw; the menu
    /// is open for scripts ([`menus::BARTER`]) until the engine closes it. Not part of
    /// the saved state.
    pub barter_requests: Vec<BarterRequest>,
    /// Recipe menus scripts opened (ShowRecipeMenu, `crate::crafting`), for the engine to
    /// draw; the menu is open for scripts ([`crate::crafting::RECIPE_MENU`]) until the
    /// engine closes it. Not part of the saved state.
    pub recipe_requests: Vec<crate::crafting::RecipeRequest>,
    /// Repair services menus scripts opened (ShowRepairMenu, `crate::repair`), for the
    /// engine to draw; the menu is open for scripts
    /// ([`crate::repair::REPAIR_SERVICES_MENU`]) until the engine closes it. Not part of
    /// the saved state.
    pub repair_requests: Vec<crate::repair::RepairRequest>,
    /// Roulette menus asked for by ShowRouletteMenuParams (`crate::roulette`); the engine
    /// opens them.
    pub roulette_requests: Vec<crate::roulette::RouletteRequest>,
    /// Blackjack menus asked for by ShowBlackJackMenuParams (`crate::blackjack`); the
    /// engine opens them.
    pub blackjack_requests: Vec<crate::blackjack::BlackjackRequest>,
    /// Teammates' inventories scripts opened (OpenTeammateContainer,
    /// `crate::followers`), for the engine to show as a container menu. Not part of the
    /// saved state.
    pub teammate_containers: Vec<FormId>,
    /// Teammates brought to the player when the player changed cells
    /// (`crate::followers`), for the engine's log; they are in [`Game::teleported`] too.
    /// Not part of the saved state.
    pub brought_along: BTreeSet<FormId>,
    /// Doors locked or unlocked by packages' door flags (`crate::ai`), for the engine's
    /// log: actor, package, locked (else unlocked), how many. Not part of the saved state.
    pub package_door_notes: Vec<(FormId, FormId, bool, usize)>,
    /// Sleeping and waiting as scripts see and ask for them (`crate::rest`); the menu is
    /// the engine's. Not part of the saved state.
    pub rest: crate::rest::Rest,
    /// The latest detection value of each (detector, target) pair the engine measured
    /// (`crate::detection`); GetDetected and GetDetectionLevel read it. Not part of the
    /// saved state: the engine measures again.
    pub detection: crate::detection::Detections,
    /// Every actor's detection state (Normal, Alert, Combat, Lost), its alert and target
    /// lists, and detection events waiting to be heard (`crate::awareness`). Not part of
    /// the saved state: the engine measures again.
    pub awareness: crate::awareness::Awarenesses,
    /// Fights started by being hit, friendly hits, yields and allies joining
    /// (`crate::provocation`). Not part of the saved state.
    pub provocations: crate::provocation::Provocations,
    /// Combat-tab lines: attack timers, health marks, DeathResponse lines due
    /// (`crate::combat_lines`). Not part of the saved state.
    pub combat_lines: crate::combat_lines::CombatLines,
    /// What switched radio receivers on and off (`crate::radio`); the engine plays the
    /// stations. Not part of the saved state.
    pub radio: crate::radio::Radios,
    /// Terminals hacked or locked out (`crate::terminal`).
    pub terminals: crate::terminal::Terminals,
    /// Locks broken by forcing them (`crate::lockpick`). Not part of the saved state yet.
    pub locks: crate::lockpick::Locks,
    /// What the engine tells the scripts of the actors' movement, sneaking and sight and
    /// of the camera (`functions::senses`). Not part of the saved state: the engine
    /// measures again.
    pub senses: crate::functions::senses::Senses,
    /// VATS mode's state and the marked shot leaving now, as the engine says
    /// (`crate::vats`). Not part of the saved state: the mode is never saved.
    pub vats: crate::vats::VatsState,
    /// The weather scripts forced or set, and the one the engine shows
    /// (`crate::weather`). Not part of the saved state yet.
    pub weather: crate::weather::WeatherState,
    /// What scripts asked of the Pip-Boy and the character-creation menus, and to end
    /// the game (`functions::ui`), for the engine to take. Not part of the saved state.
    pub ui: crate::functions::ui::UiRequests,
    /// What console commands asked of the engine (`functions::console`), for it to take.
    pub console_requests: Vec<crate::functions::console::ConsoleRequest>,
    /// Crimes being reported, trespass warnings and karma changes for the engine
    /// (`crate::crime`); what lasts is in [`PlayerState`]. Not part of the saved state.
    pub crimes: crate::crime::Crimes,
    pub world: World<'a>,
    pub cfg: GameConfig,
    pub quests: BTreeMap<FormId, QuestState>,
    pub instances: BTreeMap<InstKey, Instance>,
    pub extra: BTreeMap<FormId, RefExtra>,
    pub player: PlayerState,
    /// Forms added to form lists at run time.
    pub form_lists: BTreeMap<FormId, Vec<FormId>>,
    pub loaded_cells: BTreeSet<FormId>,
    /// Scripted references a script moved (they may have left their authored cell).
    pub moved: BTreeSet<FormId>,
    /// References other than the player that a script put somewhere (MoveTo) since the
    /// layer that moves actors last took them ([`Game::take_teleported`]): it has to
    /// start them again from where they are now. Not saved.
    pub teleported: BTreeSet<FormId>,
    /// Rotate: this frame's turn per reference (axis 0–2, degrees per second); the last
    /// call of the frame counts. Applied at the end of [`Game::tick`]. Not saved.
    pub rotations: BTreeMap<FormId, (usize, f64)>,
    /// References Rotate turned in the latest [`Game::tick`], for the engine to draw at
    /// their new angle. Not saved.
    pub turned: BTreeSet<FormId>,
    /// Actors whose OnStartCombat block was fired and whose OnCombatEnd was not yet
    /// (`Game::combat_blocks`). Not saved, as the combat states are not [documented:
    /// GECK wiki event handler table, OnStartCombat does not run again on loading a save
    /// with combat already going].
    pub fighting: BTreeSet<FormId>,
    /// The target of the condition list being evaluated ([`Game::conditions_pass`]), for
    /// the functions that ask for it (Exists). Not saved.
    pub condition_target: Option<FormId>,
    /// Cell resets since the engine last took them ([`Game::take_cell_resets`],
    /// `crate::respawn`). Not saved.
    pub cell_resets: Vec<crate::respawn::CellReset>,
    /// Actors ResurrectActor brought back since the engine last took them
    /// ([`Game::take_resurrections`], `crate::resurrect`). Not saved.
    pub resurrections: Vec<crate::resurrect::Resurrection>,
    /// The open menu, by menu code. While a menu is open, [`Game::tick`] runs MenuMode
    /// blocks instead of GameMode, and game time stops for scripts (GetSecondsPassed and
    /// the quest delay of GameMode) [observed: layer5-captures.md §8.3 e]. Part of the
    /// engine API, docs/spec-notes/scripts.md §7.2.
    pub menu: Option<u16>,
    /// Which function opened the race menu (1036): it is one menu for the race menu, the
    /// barber and the plastic surgeon, which change different parts of the look
    /// (docs/spec-notes/character-creator.md §10). Not saved.
    pub race_menu: RaceMenu,
    /// A loading screen is up: [`Game::tick`] runs no script and game time stops
    /// [observed: GetSecondsPassed did not count a `coc` loading screen,
    /// layer5-captures.md §8.3 e]. Part of the engine API, scripts.md §7.2.
    pub loading: bool,
    pub message_box: Option<MessageBox>,
    /// Message boxes shown and not yet closed, in the order they were shown; the first
    /// is on screen (menu [`menus::MESSAGE`]), the others wait for it to close
    /// [hypothesis: one box at a time, the next when the open one closes]. `FormId(0)`:
    /// a box whose message is not known (a scenario's `given`).
    pub boxes: Vec<FormId>,
    /// Dialogue lines being said (SayTo, Say), one per speaker.
    pub speech: Vec<Speech>,
    /// (speaker, line) pairs of the Say Once lines already said, and of every line said
    /// in a conversation with the player (the shading of choices).
    pub said: BTreeSet<(FormId, FormId)>,
    /// Say Once a Day lines: speaker → line → the game clock in hours when it was last
    /// said (`crate::dialogue`).
    pub said_day: BTreeMap<FormId, BTreeMap<FormId, f64>>,
    /// The conversation between the player and an actor, if one is running
    /// (`crate::dialogue`).
    pub dialogue: Option<crate::dialogue::Conversation>,
    /// The lengths of voice files, when the game was given them (`crate::voice`): a
    /// response then lasts as long as its sound. Not part of the saved state.
    pub voice: Option<std::sync::Arc<dyn crate::voice::VoiceLengths>>,
    pub frame: u64,
    /// Game time in seconds: the frame steps that were not in a menu or a loading
    /// screen. GetSecondsPassed reads it.
    pub time: f64,
    /// References enabled since the last frame whose OnLoad runs at the start of the
    /// next one [observed: OnLoad after Enable had not run on the line after Enable and
    /// had run by the next frame, layer5-captures.md §8.3 d].
    pub pending_onload: Vec<FormId>,
    pub events: Vec<Event>,
    /// Calls per opcode that have no implementation.
    pub unhandled: BTreeMap<u16, u64>,
    /// Calls per opcode that reached a Layer 6 stub.
    pub stubbed: BTreeMap<u16, u64>,
    unhandled_seen: BTreeSet<(u16, String)>,
    stubs_seen: BTreeSet<(u16, String, String)>,
    pub rng: ChaCha8Rng,
    /// The random numbers of the idle tree's talking gestures (`crate::idles`), kept
    /// apart so that picking them does not move the scripts' `rng`. Not saved.
    pub(crate) idle_rng: ChaCha8Rng,
    /// Run-time changes to base forms and cells.
    pub forms: FormChanges,
    /// Magic effects active on actors, in the order they started.
    pub effects: Vec<ActiveEffect>,
    pub next_effect: u32,
    /// Bringing an actor's abilities and worn enchantments up to date
    /// (`crate::abilities`): not again inside. Not saved.
    pub(crate) syncing_effects: bool,
    /// When the abilities' and worn enchantments' conditions are checked next. Not saved.
    pub(crate) condition_check_due: f64,
    /// Image space modifiers on the screen, in the order they were applied
    /// (ApplyImageSpaceModifier). An animatable one ends by itself when its duration has
    /// passed; the others stay until removed. Their time passes with the frames outside
    /// menus and loading screens, conversations included [hypothesis: as the lines of a
    /// conversation do; image-modifiers.md §3].
    pub image_modifiers: Vec<ActiveImageModifier>,
    /// ScriptEffectElapsedSeconds inside a ScriptEffectUpdate block (0 elsewhere).
    pub effect_elapsed: f64,
    depth: u32,
    /// A function asked the running script to stop as `Return` does (RemoveMe).
    stop_run: bool,
    /// Game time of the next scan for actors whose package choice is due (`crate::ai`).
    pub(crate) package_scan_due: f64,
}

/// Snapshot of the Layer 5 state plus the world's (`fnv.game.state/1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub format: String,
    pub sim: Snapshot,
    pub quests: BTreeMap<FormId, QuestState>,
    pub instances: Vec<(InstKey, Instance)>,
    pub extra: BTreeMap<FormId, RefExtra>,
    pub player: PlayerState,
    pub form_lists: BTreeMap<FormId, Vec<FormId>>,
    pub loaded_cells: BTreeSet<FormId>,
    pub moved: BTreeSet<FormId>,
    pub menu: Option<u16>,
    #[serde(default)]
    pub loading: bool,
    pub message_box: Option<MessageBox>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<FormId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speech: Vec<Speech>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub said: BTreeSet<(FormId, FormId)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub said_day: BTreeMap<FormId, BTreeMap<FormId, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<crate::dialogue::Conversation>,
    pub frame: u64,
    pub time: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_onload: Vec<FormId>,
    pub rng_word_pos: String,
    #[serde(default)]
    pub forms: FormChanges,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ActiveEffect>,
    #[serde(default)]
    pub next_effect: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_modifiers: Vec<ActiveImageModifier>,
    /// Broken locks and the lockpicking help's first display (`crate::lockpick::Locks`).
    #[serde(default, skip_serializing_if = "crate::lockpick::Locks::is_empty")]
    pub locks: crate::lockpick::Locks,
    /// Terminals hacked or locked out (`crate::terminal::Terminals::access`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terminals: BTreeMap<FormId, crate::terminal::Access>,
    /// Terminals scripts locked or unlocked (`crate::terminal::Terminals::script_locks`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terminal_locks: BTreeMap<FormId, crate::terminal::ScriptLock>,
    /// The Pip-Boy radio as scripts and the player left it: on, and the station tuned.
    #[serde(default)]
    pub pipboy_radio_on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipboy_station: Option<FormId>,
    /// SetNPCRadio: actor -> the station's reference it plays.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub npc_radios: BTreeMap<FormId, FormId>,
    /// The weather the sky showed and whether it rained (`crate::weather::WeatherState`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather_shown: Option<FormId>,
    #[serde(default)]
    pub raining: bool,
    /// Long-lived provocations: (actor, whom it fights) and why; when each actor last
    /// accepted the player's yield (`crate::provocation::Provocations`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grudges: Vec<((FormId, FormId), crate::provocation::Grudge)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub forgiven: BTreeMap<FormId, f64>,
}

pub const SNAPSHOT_FORMAT: &str = "fnv.game.state/1";

/// How an activation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActMode {
    /// The player (or an actor) activates by hand: a script's OnActivate block runs
    /// instead of the default action; the default action runs only when the block calls
    /// `Activate` [documented: GECK wiki Activate; layer5.md §3.4].
    Hand,
    /// `Activate` with flag 0 (or none): the default action, bypassing any OnActivate
    /// block [documented: GECK wiki Activate; observed: a scripted `Activate player` of
    /// the Doc Mitchell house door and of dead ED-E did not run their blocks,
    /// layer5-captures.md §8.3 f].
    Default,
    /// `Activate actor 1`: the OnActivate block instead of the default action
    /// [documented: GECK wiki Activate, RunOnActivateBlockFlag]; the default action when
    /// the target has no block [observed: a dead legionary without an OnActivate block
    /// opened its loot menu, `L5.quest.dead_activation`].
    Block,
}

/// Where a running script keeps its locals.
enum Locals {
    Instance(InstKey),
    /// A result script (stage, dialogue, package): locals live for one run.
    Temp(BTreeMap<u16, Value>),
}

/// What a script run is attached to.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunCtx {
    pub self_ref: Option<FormId>,
    pub action_ref: Option<FormId>,
    pub quest: Option<FormId>,
}

impl<'a> Game<'a> {
    pub fn new(data: &'a GameData, mut world: World<'a>, cfg: GameConfig) -> Game<'a> {
        // Placed actors carry their template's items (template flag "Inventory").
        world.set_inventory_sources(data.inventory_sources(world.placed));
        Game {
            data,
            toggles: Default::default(),
            combat_requests: Vec::new(),
            knockout_news: Vec::new(),
            idle_requests: Vec::new(),
            weapon_uses: BTreeMap::new(),
            weapon_use_seq: 0,
            sound_requests: Vec::new(),
            music_requests: Vec::new(),
            texture_swaps: Default::default(),
            script_looks: Default::default(),
            anims: Default::default(),
            barter_requests: Vec::new(),
            recipe_requests: Vec::new(),
            repair_requests: Vec::new(),
            roulette_requests: Vec::new(),
            blackjack_requests: Vec::new(),
            teammate_containers: Vec::new(),
            brought_along: BTreeSet::new(),
            package_door_notes: Vec::new(),
            rest: Default::default(),
            detection: Default::default(),
            awareness: Default::default(),
            provocations: Default::default(),
            combat_lines: Default::default(),
            radio: Default::default(),
            terminals: Default::default(),
            locks: Default::default(),
            senses: Default::default(),
            vats: Default::default(),
            weather: Default::default(),
            ui: Default::default(),
            console_requests: Vec::new(),
            crimes: Default::default(),
            world,
            cfg,
            quests: BTreeMap::new(),
            instances: BTreeMap::new(),
            extra: BTreeMap::new(),
            player: PlayerState::default(),
            form_lists: BTreeMap::new(),
            loaded_cells: BTreeSet::new(),
            moved: BTreeSet::new(),
            teleported: BTreeSet::new(),
            rotations: BTreeMap::new(),
            turned: BTreeSet::new(),
            cell_resets: Vec::new(),
            resurrections: Vec::new(),
            fighting: BTreeSet::new(),
            condition_target: None,
            menu: None,
            race_menu: RaceMenu::Creator,
            loading: false,
            message_box: None,
            boxes: Vec::new(),
            speech: Vec::new(),
            said: BTreeSet::new(),
            said_day: BTreeMap::new(),
            dialogue: None,
            voice: None,
            frame: 0,
            time: 0.0,
            pending_onload: Vec::new(),
            events: Vec::new(),
            unhandled: BTreeMap::new(),
            stubbed: BTreeMap::new(),
            unhandled_seen: BTreeSet::new(),
            stubs_seen: BTreeSet::new(),
            rng: ChaCha8Rng::seed_from_u64(cfg.seed ^ 0x5C21_7000),
            idle_rng: ChaCha8Rng::seed_from_u64(cfg.seed ^ 0x1D1E_7A1C),
            forms: FormChanges::default(),
            effects: Vec::new(),
            next_effect: 1,
            syncing_effects: false,
            condition_check_due: 0.0,
            image_modifiers: Vec::new(),
            effect_elapsed: 0.0,
            depth: 0,
            stop_run: false,
            package_scan_due: 0.0,
        }
    }

    pub fn emit(&mut self, kind: EventKind) {
        self.events.push(Event { frame: self.frame, kind });
    }

    /// The new-game state: every quest flagged "Start game enabled" is running, with no
    /// stage set [hypothesis: the flag's documented meaning (xEdit); which quest the
    /// "New Game" menu starts is a separate input, scripts.md §8]; the placed actors that
    /// start as corpses are dead ([`Game::lay_out_corpses`]).
    pub fn new_game(&mut self) {
        // Placed corpses (Base Health 0) lie dead from the start.
        self.lay_out_corpses();
        let start: Vec<FormId> = self.data.quests.values().filter(|q| q.start_enabled()).map(|q| q.form).collect();
        for q in start {
            self.start_quest(q);
        }
    }

    // ---- quests ------------------------------------------------------------------

    pub fn quest(&mut self, q: FormId) -> &mut QuestState {
        self.quests.entry(q).or_default()
    }

    pub fn quest_state(&self, q: FormId) -> QuestState {
        self.quests.get(&q).cloned().unwrap_or_default()
    }

    pub fn start_quest(&mut self, q: FormId) -> bool {
        if !self.data.is_quest(q) {
            return false;
        }
        let s = self.quest(q);
        if !s.running {
            s.running = true;
            s.since_run = 0.0;
            s.since_menu_run = 0.0;
            self.emit(EventKind::QuestStarted { quest: q });
        }
        true
    }

    pub fn stop_quest(&mut self, q: FormId) -> bool {
        if !self.data.is_quest(q) {
            return false;
        }
        let s = self.quest(q);
        if s.running {
            s.running = false;
            self.emit(EventKind::QuestStopped { quest: q });
        }
        true
    }

    /// `SetStage quest stage` [documented: GECK wiki SetStage; observed: the 5C captures
    /// named per rule]:
    /// - a stage the quest does not define does nothing [observed: VMS16 stage 15];
    /// - a stage already done does nothing unless the quest allows repeated stages
    ///   [observed: VMS16 stage 10 and VCG03 stage 10 set again did not rerun their
    ///   scripts; Generic, with the flag, did];
    /// - the quest starts if it is not running [observed: VMS16];
    /// - the stage is marked done, then every entry whose conditions pass runs its result
    ///   script immediately, nested in the caller [documented: wiki, "resolves any of the
    ///   attached stage items that pass their conditions"], then applies its Complete /
    ///   Fail flag [hypothesis on the order]. Completing does not stop the quest
    ///   [observed: VMS22 ran on after its complete stage].
    pub fn set_stage(&mut self, q: FormId, stage: i32) -> bool {
        let data = self.data;
        let Some(def) = data.quests.get(&q) else { return false };
        let Some(entries) = def.stages.get(&stage) else {
            self.emit(EventKind::StageIgnored { quest: q, stage, reason: "undefined_stage" });
            return false;
        };
        if self.quest(q).stages_done.contains(&stage) && !def.repeated_stages() {
            self.emit(EventKind::StageIgnored { quest: q, stage, reason: "already_done" });
            return false;
        }
        self.start_quest(q);
        self.quest(q).stages_done.insert(stage);
        let player = Some(FormId::PLAYER_REF);
        let label = format!("QUST {q} stage {stage}");
        let mut ran = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            // Stage conditions run on the player [hypothesis].
            if self.conditions_pass(&e.conditions, player, player, &label) {
                ran.push(i);
            }
        }
        self.emit(EventKind::StageSet { quest: q, stage, entries: ran.clone() });
        for i in ran {
            let e = &entries[i];
            if let Some(u) = e.unit {
                self.run_result(u, RunCtx { quest: Some(q), ..Default::default() });
            }
            if e.flags & STAGE_COMPLETE != 0 {
                let s = self.quest(q);
                if !s.completed {
                    s.completed = true;
                    self.emit(EventKind::QuestCompleted { quest: q });
                }
            }
            if e.flags & STAGE_FAIL != 0 {
                // A Fail Quest stage also reads as completed (GetQuestCompleted 1)
                // [observed: VMS16's fail stage 110, `L5.quest.stage_order`].
                let s = self.quest(q);
                let first = !s.failed;
                s.failed = true;
                s.completed = true;
                if first {
                    self.emit(EventKind::QuestFailed { quest: q });
                }
            }
        }
        true
    }

    /// `SetObjectiveDisplayed` / `SetObjectiveCompleted`. Only objectives the quest
    /// defines change; displaying one starts a stopped quest [documented: GECK wiki
    /// SetObjectiveDisplayed]. Completing does not display; displaying clears the
    /// completed flag [observed: VMS16 objectives 10 and 5, layer5-captures.md §5.2].
    pub fn set_objective(&mut self, q: FormId, index: i32, displayed: Option<bool>, completed: Option<bool>) -> bool {
        let data = self.data;
        let Some(def) = data.quests.get(&q) else { return false };
        if !def.objectives.contains_key(&index) {
            return false;
        }
        if displayed == Some(true) {
            self.start_quest(q);
        }
        let o = self.quest(q).objectives.entry(index).or_default();
        let (d0, c0) = (o.displayed, o.completed);
        if let Some(d) = displayed {
            o.displayed = d;
            if d {
                o.completed = false;
            }
        }
        if let Some(c) = completed {
            o.completed = c;
        }
        let (d1, c1) = (o.displayed, o.completed);
        if displayed.is_some() && d0 != d1 {
            self.emit(EventKind::ObjectiveDisplayed { quest: q, index, on: d1 });
        }
        if c0 != c1 {
            self.emit(EventKind::ObjectiveCompleted { quest: q, index, on: c1 });
        }
        true
    }

    // ---- instances and variables -------------------------------------------------

    /// The script a reference runs (its base form's), if any.
    pub fn ref_script(&self, r: FormId) -> Option<FormId> {
        let base = match self.world.peek(r) {
            Some(s) => s.base,
            None => self.world.placed.get(r).map(|p| p.base).or((r == FormId::PLAYER_REF).then_some(FormId::PLAYER_BASE))?,
        };
        // An actor with the template flag "Script" runs its template's script [documented:
        // GECK wiki GetActorTemplateFlag, "Use Script"].
        let base = if self.data.actors.contains_key(&base) || self.data.leveled_actors.contains_key(&base) {
            self.data.template_source(base, r, |d| d.template_script).map_or(base, |(id, _)| id)
        } else {
            base
        };
        self.data.script_of_base(base)
    }

    /// The instance that `form.var` addresses: a quest's script or a reference's.
    pub fn instance_of(&self, form: FormId) -> Option<(InstKey, FormId)> {
        if let Some(q) = self.data.quests.get(&form) {
            let s = q.script.filter(|s| self.data.scripts.contains_key(s))?;
            return Some((InstKey::Quest { quest: form }, s));
        }
        Some((InstKey::Ref { reference: form }, self.ref_script(form)?))
    }

    fn instance(&mut self, key: InstKey, script: Option<FormId>) -> &mut Instance {
        self.instances.entry(key).or_insert_with(|| Instance { script, ..Default::default() })
    }

    /// Read a variable of an instance (0 when never written).
    pub fn var(&self, key: InstKey, index: u16) -> Value {
        self.instances.get(&key).and_then(|i| i.vars.get(&index)).copied().unwrap_or_default()
    }

    /// Store into a variable by its declared type: integer variables truncate (see
    /// [`to_int_storage`]), float/ref variables keep the value.
    pub fn store(&mut self, key: InstKey, script: Option<FormId>, index: u16, v: Value, unit: &str, letter_int: bool) {
        let data = self.data;
        let decl = script.and_then(|s| data.script(s)).and_then(|c| c.var_type(index).cloned());
        if script.is_some() && decl.is_none() {
            self.emit(EventKind::UndeclaredVariable { owner: key.label(), index, unit: unit.to_string() });
        }
        let int = match decl {
            Some(VarType::Int) => true,
            Some(_) => false,
            None => letter_int,
        };
        let v = if int { Value::Num(to_int_storage(v.num()) as f64) } else { v };
        self.instance(key, script).vars.insert(index, v);
    }

    pub fn var_name(&self, script: Option<FormId>, index: u16) -> Option<String> {
        script.and_then(|s| self.data.script(s)).and_then(|c| c.var_name(index)).map(str::to_string)
    }

    // ---- running scripts ---------------------------------------------------------

    fn opts(&self) -> RunOpts {
        RunOpts { strict: self.cfg.strict, max_steps: self.cfg.max_steps }
    }

    /// Run statements; returns whether the run ended with `Return` (or a function that
    /// acts as one).
    fn exec(&mut self, unit: &'a Compiled, stmts: &'a [Stmt], locals: Locals, ctx: RunCtx) -> bool {
        if self.depth >= MAX_DEPTH {
            self.emit(EventKind::ScriptError { unit: unit.key(), error: format!("nesting deeper than {MAX_DEPTH}") });
            return false;
        }
        self.depth += 1;
        let opts = self.opts();
        let data = self.data;
        let table = &data.table;
        let mut ex = Exec { game: self, unit, locals, ctx };
        let r = Vm::new(table, &mut ex, opts).run(stmts);
        self.depth -= 1;
        match r {
            Ok(exit) => exit == fnv_script::vm::Exit::Returned,
            Err(e) => {
                self.emit(EventKind::ScriptError { unit: unit.key(), error: e.to_string() });
                false
            }
        }
    }

    /// Run a result script (quest stage, dialogue, package, terminal): the top-level
    /// statements, with temporary locals.
    pub fn run_result(&mut self, unit: UnitId, ctx: RunCtx) {
        let c = self.data.unit(unit);
        self.exec(c, &c.program.top, Locals::Temp(BTreeMap::new()), ctx);
    }

    /// Whether a block's parameter accepts `subject` (a block without parameters accepts
    /// everything; `Begin OnActivate player` only the player; `Begin MenuMode 1036` only
    /// that menu).
    fn block_accepts(&mut self, unit: &'a Compiled, key: InstKey, block: &Block, subject: Option<Value>) -> bool {
        let Some(params) = block.params.as_ref().filter(|p| !p.is_empty()) else { return true };
        let Some(subject) = subject else { return false };
        let want = match &params[0] {
            Arg::Int(i) => Value::Num(*i as f64),
            Arg::Float(f) => Value::Num(*f),
            Arg::Enum(c) => Value::Num(*c as f64),
            Arg::Ref(r) => {
                let mut ex = Exec { game: self, unit, locals: Locals::Instance(key), ctx: RunCtx::default() };
                match ex.ref_value(*r) {
                    Ok(v) => v,
                    Err(_) => return false,
                }
            }
            _ => return true,
        };
        match (want.as_form(), subject.as_form()) {
            (Some(a), Some(b)) => a == b,
            _ => want.num() == subject.num(),
        }
    }

    /// Run every block of `event` in the instance's script that accepts `subject`.
    /// Returns whether a block ran.
    pub fn run_event(&mut self, key: InstKey, script: FormId, event: u16, subject: Option<Value>, ctx: RunCtx) -> bool {
        let data = self.data;
        let Some(unit) = data.script(script) else { return false };
        let mut ran = false;
        for b in unit.program.blocks.iter().filter(|b| b.event == event) {
            if !self.block_accepts(unit, key, b, subject) {
                continue;
            }
            self.instance(key, Some(script));
            ran = true;
            // Return ends the script for this run, not just its block: a later block
            // of the same event does not run [documented: GECK wiki Adding an Options
            // Menu, owner export; docs/spec-notes/script-language.md §7.3 row 2].
            if self.exec(unit, &b.body, Locals::Instance(key), ctx) {
                break;
            }
        }
        ran
    }

    /// Run a non-frame event on a reference's script, with a trace line.
    pub fn ref_event(&mut self, r: FormId, event: u16, subject: Option<Value>, action_ref: Option<FormId>) -> bool {
        if event == event::ON_LOAD {
            // An OnLoad run by the caller (a cell load, or an engine that runs it
            // itself) settles one queued by Enable: OnLoad runs once per load.
            self.pending_onload.retain(|&x| x != r);
        }
        let Some(script) = self.ref_script(r) else { return false };
        let has = self.data.script(script).is_some_and(|u| u.has_block(event));
        if !has {
            return false;
        }
        let ran = self.run_event(
            InstKey::Ref { reference: r },
            script,
            event,
            subject,
            RunCtx { self_ref: Some(r), action_ref, quest: None },
        );
        if ran {
            let block = self.data.table.block_name(event).unwrap_or("?").to_string();
            let unit = self.data.script(script).map(|u| u.key()).unwrap_or_default();
            self.emit(EventKind::BlockRun { unit, block, on: Some(r) });
        }
        ran
    }

    // ---- frames ------------------------------------------------------------------

    /// One frame. Nothing runs during a loading screen ([`Game::loading`]). Otherwise:
    /// the OnLoad blocks of references enabled since the last frame; quest scripts whose
    /// delay has passed; the object scripts of enabled references in loaded cells; the
    /// scripts of the items the player carries. Each runs GameMode, or MenuMode while a
    /// menu is open ([`Game::menu`]). Then active magic effects and the dialogue lines
    /// being said ([`Game::speech`]) advance.
    ///
    /// Game time ([`Game::time`]) and the quest delay of GameMode advance only outside
    /// menus and loading screens [observed: GetSecondsPassed after a 900-frame container
    /// menu was one ordinary frame for an every-frame quest and the quest's own 0.5 s
    /// delay for VMQ03aTimer, `L5.quest.seconds_after_menu`; layer5-captures.md §8.3 e;
    /// a quest timer and an object script's stopwatch stood still in the name menu,
    /// §10.1]. MenuMode keeps its own delay counter, which advances only in menus
    /// [hypothesis].
    /// Order: quest scripts before the object scripts of references [observed for one
    /// pair: BeatrixFX's GameMode moved GameHour on and two Goodsprings windows
    /// (WastelandWindowScript, copying GetCurrentTime every frame) saw the new hour in
    /// the same frame, `L5.quest.script_order`, layer5-captures.md §9.3 c; hypothesis
    /// for every other pair], each in FormID order [hypothesis], then the items the
    /// player carries, then active magic effects, then dialogue (SayTo lines)
    /// [hypothesis]. Last, in menu mode as well, the conversation with the player
    /// (`crate::dialogue`): it is a menu that pauses the game while its own lines go on
    /// [owner statement 2026-10-10].
    ///
    /// A menu opened by a GameMode script (a message box, the name menu…) holds the
    /// GameMode scripts that have not run yet in this frame: they run, quests on their
    /// due delay, once the menu is closed [hypothesis, generalised from the four pack
    /// plugins' message boxes, which opened one at a time as their start-game quests
    /// ran, each after the previous one was closed, layer5-captures.md §10.1–10.2].
    pub fn tick(&mut self) {
        self.frame += 1;
        self.turned.clear();
        if self.loading {
            return;
        }
        let dt = self.cfg.frame_seconds;
        // A conversation with the player is menu mode too (`Game::in_menu`).
        let in_menu = self.in_menu();
        if !in_menu {
            self.time += dt;
        }
        for r in std::mem::take(&mut self.pending_onload) {
            if self.present(r) && self.cell_of(r).is_some_and(|c| self.loaded_cells.contains(&c)) {
                self.ref_event(r, event::ON_LOAD, None, None);
            }
        }
        self.combat_blocks();
        let ev = if in_menu { event::MENU_MODE } else { event::GAME_MODE };
        let subject = self.open_menu_code().map(|m| Value::Num(m as f64));
        let data = self.data;
        // ToggleScripts: no per-frame block runs (`functions::toggles`).
        let scripts = !self.toggles.scripts_off;
        let due: Vec<(FormId, FormId)> = self
            .quests
            .iter_mut()
            .filter(|(_, s)| scripts && s.running)
            .filter_map(|(q, s)| {
                let def = data.quests.get(q)?;
                let script = def.script.filter(|s| data.scripts.contains_key(s))?;
                let counter = if in_menu { &mut s.since_menu_run } else { &mut s.since_run };
                *counter += dt;
                let delay = s.delay.unwrap_or_else(|| def.script_delay());
                (*counter + 1e-9 >= delay).then_some((*q, script))
            })
            .collect();
        // A menu opened during this frame's GameMode holds the rest of it.
        let held = |g: &Game| !in_menu && g.in_menu();
        for (q, script) in due {
            if held(self) {
                break;
            }
            // A quest stopped by an earlier script this frame does not run.
            let Some(s) = self.quests.get_mut(&q).filter(|s| s.running) else { continue };
            if in_menu {
                s.since_menu_run = 0.0;
            } else {
                s.since_run = 0.0;
            }
            let key = InstKey::Quest { quest: q };
            if !in_menu {
                self.mark_run(key, Some(script));
            }
            self.run_event(key, script, ev, subject, RunCtx { quest: Some(q), ..Default::default() });
        }
        for r in if scripts { self.active_refs() } else { Vec::new() } {
            if held(self) {
                break;
            }
            let Some(script) = self.ref_script(r) else { continue };
            if self.data.script(script).is_some_and(|u| u.has_block(ev)) {
                let key = InstKey::Ref { reference: r };
                if !in_menu {
                    self.mark_run(key, Some(script));
                }
                self.run_event(key, script, ev, subject, RunCtx { self_ref: Some(r), ..Default::default() });
            }
        }
        let holder = FormId::PLAYER_REF;
        for (item, script) in if scripts { self.carried_scripts(holder, ev) } else { Vec::new() } {
            if held(self) {
                break;
            }
            // An earlier item's script may have removed this one.
            if self.world.item_count(holder, item).unwrap_or(0) > 0 {
                let key = InstKey::Item { holder, item };
                if !in_menu {
                    self.mark_run(key, Some(script));
                }
                self.run_event(key, script, ev, subject, RunCtx { self_ref: None, action_ref: Some(holder), quest: None });
            }
        }
        // Abilities and worn enchantments follow the loaded actors and what they wear,
        // menus included (an item equipped in the Pip-Boy).
        self.sync_loaded_actor_effects();
        if !in_menu {
            self.update_effects(dt);
            self.tick_needs(dt);
            self.tick_knockouts(dt);
            self.update_speech(dt);
            self.apply_rotations(dt);
        }
        self.rotations.clear();
        // The conversation's own lines go on in real time while it pauses the game;
        // another menu on top of it (a message box) holds it.
        if self.menu.is_none() {
            self.update_dialogue(dt);
            self.tick_packages();
            self.update_image_modifiers(dt);
        }
    }

    /// An instance's per-frame block is about to run: GetSecondsPassed in this run
    /// returns the clock time since the previous run started [observed: quest timers
    /// counted down by GetSecondsPassed came 0.95–1.0 × their fTimer after the stage
    /// that set it, also when the script had not called GetSecondsPassed since an
    /// earlier countdown ran out (VCG01 7 at 3.22 s of 3.25, 70 at 0.96 s of 1.0,
    /// layer5-captures.md §10.7); the drops of the delay captures are the quest delay,
    /// §5.4; the rule layer5.md §3.4 planned].
    pub fn mark_run(&mut self, key: InstKey, script: Option<FormId>) {
        let now = functions::clock_now(self.time);
        let inst = self.instance(key, script);
        inst.run_prev = inst.run_last;
        inst.run_last = Some(now);
    }

    /// The scripted items `holder` carries whose script has a block for `ev`, in FormID
    /// order. Only the player's inventory runs per-frame item scripts here [observed: a
    /// jar of pistol powder's GameMode ran in the player's inventory by the frame after
    /// it was added, `L5.quest.onadd_gamemode`; hypothesis: items held by other actors
    /// and containers also run, not modelled].
    fn carried_scripts(&self, holder: FormId, ev: u16) -> Vec<(FormId, FormId)> {
        let Some(inv) = self.world.peek(holder).and_then(|s| s.inventory.as_ref()) else { return Vec::new() };
        inv.iter()
            .filter_map(|(item, _)| {
                let script = self.data.script_of_base(item)?;
                self.data.script(script).is_some_and(|u| u.has_block(ev)).then_some((item, script))
            })
            .collect()
    }

    /// Whether game time is stopped for scripts (a menu, a conversation with the player,
    /// or a loading screen).
    pub fn paused(&self) -> bool {
        self.in_menu() || self.loading
    }

    /// Whether the game is in menu mode: a menu is open ([`Game::menu`]) or a
    /// conversation with the player is running ([`Game::dialogue`]) [owner statement
    /// 2026-10-10: a back-and-forth dialogue with the player pauses the game in the
    /// background; lines said outside one (SayTo, Say) do not]. In menu mode
    /// [`Game::tick`] runs MenuMode blocks instead of GameMode and game time stands
    /// still, by the rules observed for the other menus.
    pub fn in_menu(&self) -> bool {
        self.menu.is_some() || self.dialogue.is_some()
    }

    /// The code of the menu on top: [`Game::menu`], else the dialogue menu (1009)
    /// during a conversation. `Begin MenuMode <code>` blocks get this code.
    pub fn open_menu_code(&self) -> Option<u16> {
        self.menu.or(self.dialogue.as_ref().map(|_| menus::DIALOGUE))
    }

    /// Whether the menu with this code is open (`MenuMode <code>`): the menu on top, or
    /// the dialogue menu under it during a conversation [hypothesis: a message box over
    /// a conversation leaves `MenuMode 1009` at 1].
    pub fn menu_open(&self, code: u16) -> bool {
        self.menu == Some(code) || (code == menus::DIALOGUE && self.dialogue.is_some())
    }

    /// A function asks the running script to stop as `Return` does (RemoveMe).
    pub fn request_return(&mut self) {
        self.stop_run = true;
    }

    // ---- menus and message boxes (docs/spec-notes/scripts.md §7.7) ----------------

    /// A script opens a menu (the name, race, tag-skill, trait or SPECIAL menu, a message
    /// box): it is the open menu from now on [observed: each menu was open on the frame
    /// after the stage whose result script opens it, layer5-captures.md §10.2–10.3;
    /// hypothesis: a second menu replaces the first].
    pub fn open_menu(&mut self, code: u16) {
        if self.menu != Some(code) {
            self.menu = Some(code);
            self.emit(EventKind::MenuOpened { menu: code });
        }
    }

    /// `ShowMessage` of a message box: it opens at once when no menu is open, otherwise
    /// it waits until the open menu and the boxes shown before it are closed
    /// [hypothesis]. While it is open, game time stops and MenuMode 1001 runs [observed:
    /// the pack plugins' boxes held GameMode, VCG00's countdown and the world clock,
    /// layer5-captures.md §10.1].
    pub fn show_box(&mut self, message: FormId) {
        if !self.cfg.modal_message_boxes {
            return;
        }
        self.boxes.push(message);
        if self.menu.is_none() {
            self.open_menu(menus::MESSAGE);
        }
    }

    /// The player closes the open menu with OK / Done (the console's CloseAllMenus closes
    /// it the same way [observed: layer5-captures.md §10.1, §10.3]; the name menu keeps
    /// the name it has). A message box waiting for a button gets button 0 (OK). The next
    /// waiting message box then opens [hypothesis]. Returns the menu closed.
    pub fn close_menu(&mut self) -> Option<u16> {
        let m = self.menu.take()?;
        if m == menus::MESSAGE && !self.boxes.is_empty() {
            let b = self.boxes.remove(0);
            if let Some(mb) = self.message_box.as_mut().filter(|mb| mb.message == b && mb.pressed.is_none()) {
                mb.pressed = Some(0);
            }
        }
        self.emit(EventKind::MenuClosed { menu: m });
        if !self.boxes.is_empty() {
            self.open_menu(menus::MESSAGE);
        }
        Some(m)
    }

    /// The player picks a button of the open message box: GetButtonPressed of the script
    /// that showed it returns the button once, and the box closes.
    pub fn press_button(&mut self, button: i32) -> Result<(), String> {
        let m = self.message_box.as_mut().ok_or("no message box is open")?;
        m.pressed = Some(button);
        let message = m.message;
        if self.menu == Some(menus::MESSAGE) && self.boxes.first() == Some(&message) {
            self.close_menu();
        }
        Ok(())
    }

    // ---- dialogue lines (docs/spec-notes/scripts.md §7.7) --------------------------

    /// `speaker.SayTo target topic` (`say_to`) or `speaker.Say topic`: the speaker says the
    /// first line of the topic it can say, and the line's begin result script runs at
    /// once; the end result script runs when the line ends ([`Game::tick`])
    /// [documented: GECK wiki Category:Dialogue, owner export 2026-10-09: the begin
    /// script runs when the actor starts the line, the end script when the actor finishes
    /// it].
    ///
    /// Which line: [`Game::select_line`] (who can say a line: [`Game::can_say`]), with
    /// the listener as the conditions' target (the player for Say): lines of
    /// higher-priority quests first, then the stored order, the first line the speaker
    /// can say, Random stacks apart [observed: Doc Mitchell's SayTo VCG01Intro took the
    /// next unsaid line each time, also the first line of the topic when it had never
    /// been said (stage 60 without 55), layer5-captures.md §10.3]. A speaker already
    /// saying a line stops it (its end script does not run) [hypothesis].
    pub fn say(&mut self, speaker: FormId, target: Option<FormId>, topic: FormId, say_to: bool) -> Option<FormId> {
        let listener = target.or(Some(FormId::PLAYER_REF));
        let Some(info) = self.select_line(speaker, listener, topic) else {
            self.emit(EventKind::NoLine { speaker, topic });
            return None;
        };
        self.say_line(speaker, target, topic, info, if say_to { LineKind::SayTo } else { LineKind::Say });
        Some(info)
    }

    /// Set who the line `speaker` is saying turns to whom ([`LineLook`]).
    pub fn set_line_look(&mut self, speaker: FormId, look: LineLook) {
        if let Some(s) = self.speech.iter_mut().find(|s| s.speaker == speaker) {
            s.look = look;
        }
    }

    /// Who turns to whom while the lines outside a conversation with the player are
    /// said (docs/design/dialogue-play.md §9). Each line said to someone (SayTo, a hello
    /// or a Goodbye greeting at the player, a line of a conversation between actors):
    /// - the speaker turns its head and body to the listener [owner decision
    ///   2026-10-11: "speaker turns head and body"];
    /// - the listener looks at the speaker [documented: GECK wiki SayTo: the target is
    ///   "forced to look at the speaker" unless `noTargetLookFlag`]: its head only for a
    ///   SayTo line [design], head and body in a conversation between actors, where it
    ///   speaks next [design, as `fnv_ai` has turned both since it started them];
    /// - nobody, when the line's [`LineLook`] says so.
    ///
    /// A line said to nobody (Say) turns nobody. Whether the engine can turn a body (one
    /// on furniture or on its way cannot) is the engine's.
    pub fn facings(&self) -> Vec<Facing> {
        let mut out = Vec::new();
        for s in &self.speech {
            let Some(to) = s.target.filter(|t| *t != s.speaker) else { continue };
            if s.look == LineLook::Nobody {
                continue;
            }
            out.push(Facing { who: s.speaker, to, body: true, speaking: true, info: s.info });
            if s.look == LineLook::Both {
                out.push(Facing { who: to, to: s.speaker, body: s.conversation.is_some(), speaking: false, info: s.info });
            }
        }
        out
    }

    /// The speaker starts saying the line `info` outside a conversation with the player:
    /// a line it was saying stops, the line is remembered for Say Once, and its begin
    /// result script runs ([`Game::say`]; [`LineKind`]). A line of a conversation
    /// between actors lasts as long as its first response.
    pub(crate) fn say_line(&mut self, speaker: FormId, target: Option<FormId>, topic: FormId, info: FormId, kind: LineKind) {
        let data = self.data;
        let Some(def) = data.infos.get(&info) else { return };
        self.cut_line(speaker);
        self.mark_said(speaker, info, false);
        let conversation = match kind {
            LineKind::Conversation(n) => Some(n),
            _ => None,
        };
        let seconds = match (conversation, def.response_order().first()) {
            (Some(_), Some(&r)) => self.response_seconds(speaker, info, r),
            _ => self.line_seconds(speaker, info),
        };
        self.emit(EventKind::LineStarted { speaker, target, topic, info, seconds });
        let (say_to, greeting, hello) = (kind == LineKind::SayTo, kind == LineKind::Greeting, kind == LineKind::Hello);
        self.speech.push(Speech { speaker, target, topic, info, remaining: seconds, say_to, greeting, conversation, hello, look: LineLook::Both, response: 0 });
        if let Some(&first) = def.response_order().first() {
            self.ask_response_idles(speaker, target, info, first);
        }
        if let Some(u) = def.begin {
            self.emit(EventKind::ResultScript { unit: data.unit(u).key(), on: Some(speaker) });
            self.run_result(u, RunCtx { self_ref: Some(speaker), action_ref: None, quest: def.quest });
        }
    }

    /// The lines being said outside a conversation move on to their next responses as
    /// the time of each passes: the idles of a response that starts are asked for
    /// (`crate::idles`; the first response's when the line starts).
    fn speech_responses(&mut self) {
        let data = self.data;
        let mut started = Vec::new();
        for k in 0..self.speech.len() {
            let s = &self.speech[k];
            // A conversation line between actors is its first response only.
            if s.conversation.is_some() {
                continue;
            }
            let Some(def) = data.infos.get(&s.info) else { continue };
            let order = def.response_order();
            if order.len() < 2 {
                continue;
            }
            let lengths = self.response_lengths(s.speaker, s.info);
            let total: f64 = lengths.iter().sum();
            let mut end = 0.0;
            let at = order.iter().position(|&r| {
                end += lengths.get(r).copied().unwrap_or(0.0);
                total - s.remaining < end
            });
            let at = at.unwrap_or(order.len() - 1);
            if at > s.response {
                started.push((s.speaker, s.target, s.info, order[at]));
                self.speech[k].response = at;
            }
        }
        for (speaker, target, info, response) in started {
            self.ask_response_idles(speaker, target, info, response);
        }
    }

    /// Stop the line a speaker is saying, without its end script. A conversation between
    /// actors whose line is cut ends there [hypothesis].
    pub(crate) fn cut_line(&mut self, speaker: FormId) {
        if let Some(k) = self.speech.iter().position(|s| s.speaker == speaker) {
            let s = self.speech.remove(k);
            self.emit(EventKind::LineEnded { speaker, info: s.info, cut: true });
            if let (Some(lines), Some(target)) = (s.conversation, s.target) {
                self.emit(EventKind::ActorConversationEnded { speaker, target, reason: crate::dialogue::ended::CUT, lines });
            }
        }
    }

    /// Whether an actor is saying a line (IsTalking): a SayTo or Say line, or a line of
    /// the conversation with the player. While the player chooses, the actor is not
    /// talking [hypothesis]. A SayTo line held by a menu (a conversation included) still
    /// counts, as for the other menus.
    pub fn is_talking(&self, actor: FormId) -> bool {
        self.speech.iter().any(|s| s.speaker == actor) || self.dialogue.as_ref().is_some_and(|c| c.actor == actor && c.line.is_some())
    }

    /// One frame of dialogue (game time only, like script time [hypothesis: a line does
    /// not advance while a menu is open; observed: Doc's line started at stage 60 while
    /// a message box was open ended after the boxes, layer5-captures.md §10.3]): lines
    /// whose time is up end, in the order they started; each line's end result script
    /// runs on its speaker, then the speaker's SayToDone blocks (after Say too, D1). A speaker
    /// that was disabled, deleted or died stops its line [hypothesis].
    fn update_speech(&mut self, dt: f64) {
        let mut ended = Vec::new();
        let mut cut = Vec::new();
        self.speech_responses();
        // A Death line goes on after its speaker died (`crate::combat_lines`).
        let death = self.data.by_edid("Death");
        for s in &mut self.speech {
            s.remaining -= dt;
            let here = self.world.peek(s.speaker).is_none_or(|w| !w.deleted) && self.world.is_enabled(s.speaker).unwrap_or(false);
            if !here || (self.extra.get(&s.speaker).is_some_and(|e| e.dead) && Some(s.topic) != death) {
                cut.push(s.speaker);
            } else if s.remaining <= 1e-9 {
                ended.push((s.speaker, s.info));
            }
        }
        for sp in cut {
            self.cut_line(sp);
        }
        let data = self.data;
        for (speaker, info) in ended {
            let Some(k) = self.speech.iter().position(|s| s.speaker == speaker && s.info == info) else { continue };
            let s = self.speech.remove(k);
            self.emit(EventKind::LineEnded { speaker, info, cut: false });
            if let Some(def) = data.infos.get(&info) {
                if let Some(u) = def.end {
                    self.emit(EventKind::ResultScript { unit: data.unit(u).key(), on: Some(speaker) });
                    self.run_result(u, RunCtx { self_ref: Some(speaker), action_ref: None, quest: def.quest });
                }
                // A greeting said at the player ends as a conversation's line does
                // (`Game::start_dialogue`): its Add Topics join the known topics, and it
                // is no SayTo line [hypothesis for both].
                if s.greeting {
                    for &t in &def.add_topics {
                        if self.player.topics.insert(t) {
                            self.emit(EventKind::TopicAdded { topic: t, by: Some(info) });
                        }
                    }
                }
            }
            if s.greeting {
                continue;
            }
            // A conversation between actors goes on (`crate::dialogue`); its lines are no
            // SayTo lines [hypothesis].
            if let Some(lines) = s.conversation {
                self.next_conversation_line(&s, lines);
                continue;
            }
            if s.hello {
                continue;
            }
            // SayToDone follows a line said by Say as well as by SayTo [documented: GECK
            // tutorials, docs/spec-notes/level-design.md §11.2 D1].
            self.ref_event(speaker, event::SAY_TO_DONE, Some(Value::Ref(s.topic.0)), None);
        }
        self.combat_lines_tick();
    }

    /// `r` was enabled: when it is in a loaded cell and its script has an OnLoad block,
    /// that block runs at the start of the next frame, not inside Enable [observed: the
    /// jukebox's and the broken radio's OnLoad effects were not there on the line after
    /// Enable and were there one frame later, layer5-captures.md §8.3 d].
    pub fn note_enabled(&mut self, r: FormId) {
        let has = self.ref_script(r).and_then(|s| self.data.script(s)).is_some_and(|u| u.has_block(event::ON_LOAD));
        if has && self.cell_of(r).is_some_and(|c| self.loaded_cells.contains(&c)) && !self.pending_onload.contains(&r) {
            self.pending_onload.push(r);
        }
    }

    pub fn frames(&mut self, n: u64) {
        for _ in 0..n {
            self.tick();
        }
    }

    /// The current cell of a reference.
    pub fn cell_of(&self, r: FormId) -> Option<FormId> {
        match self.world.peek(r) {
            Some(s) => s.cell,
            None => self.world.placed.get(r).and_then(|p| p.cell),
        }
    }

    /// Whether a reference is in the world and enabled (enable parents followed).
    pub fn present(&self, r: FormId) -> bool {
        self.world.peek(r).is_none_or(|s| !s.deleted) && self.world.is_enabled(r).unwrap_or(false)
    }

    /// Scripted references that run per-frame blocks now: in a loaded cell and not
    /// deleted, disabled ones included [documented: GECK wiki Object scripts, owner
    /// export: object scripts run even for disabled references]. OnLoad at a cell load
    /// still needs the reference enabled (a disabled reference has no 3D to load)
    /// [hypothesis].
    fn active_refs(&self) -> Vec<FormId> {
        let mut out: BTreeSet<FormId> = BTreeSet::new();
        for c in &self.loaded_cells {
            if let Some(v) = self.data.scripted_by_cell.get(c) {
                out.extend(v.iter().copied());
            }
        }
        out.extend(self.moved.iter().copied());
        out.into_iter()
            .filter(|&r| self.cell_of(r).is_some_and(|c| self.loaded_cells.contains(&c)) && self.world.peek(r).is_none_or(|s| !s.deleted))
            .collect()
    }

    /// The cells loaded around `cell`: the cell itself for an interior; for an exterior,
    /// the 5×5 cells around it [hypothesis: the uGridsToLoad default of 5].
    pub fn cells_around(&self, cell: FormId) -> BTreeSet<FormId> {
        let mut out = BTreeSet::from([cell]);
        let placed = self.world.placed;
        let Some(world) = placed.world_of(cell) else { return out };
        let Some(grid) = placed.world_grid.get(&world) else { return out };
        let Some((&(x, y), _)) = grid.iter().find(|(_, &c)| c == cell) else { return out };
        for dx in -2..=2 {
            for dy in -2..=2 {
                if let Some(&c) = grid.get(&(x + dx, y + dy)) {
                    out.insert(c);
                }
            }
        }
        out
    }

    /// Make `cells` the loaded set; scripted references in newly loaded cells get
    /// OnLoad [hypothesis: OnLoad runs when the reference's cell is loaded].
    pub fn set_loaded(&mut self, cells: BTreeSet<FormId>) {
        let new: Vec<FormId> = cells.difference(&self.loaded_cells).copied().collect();
        let left: Vec<FormId> = self.loaded_cells.difference(&cells).copied().collect();
        // Cells left start their reset clock; cells entered whose clock ran out are reset
        // before anything in them runs (`crate::respawn`).
        self.cells_left(&left);
        let reset: Vec<FormId> = new.iter().copied().filter(|&c| self.cell_entered(c).is_some()).collect();
        self.loaded_cells = cells;
        // The references of a reset cell run their OnReset blocks, before OnLoad
        // (`crate::respawn`).
        for c in reset {
            self.cell_reset_blocks(c);
        }
        // Actors of the new cells choose their package at the next tick, and dress now.
        self.package_scan_due = 0.0;
        self.outfit_loaded_actors();
        for c in new {
            let refs = self.data.scripted_by_cell.get(&c).cloned().unwrap_or_default();
            for r in refs {
                if self.present(r) {
                    self.ref_event(r, event::ON_LOAD, None, None);
                }
            }
        }
    }

    /// Actors in loaded cells that never were loaded put on their outfit and take up
    /// their weapon (`fnv_sim::World::outfit`), dead ones too [hypothesis: a corpse
    /// placed dead wears its outfit as the living do].
    pub fn outfit_loaded_actors(&mut self) {
        for a in self.package_actors() {
            let _ = self.world.outfit(a);
        }
    }

    /// The player is now in `cell`: load it (and its neighbours outdoors).
    pub fn player_entered(&mut self, cell: Option<FormId>) {
        if let Some(c) = cell {
            // EnableFastTravel without its keep flag: going from an interior back to an
            // exterior allows fast travel again [documented: catalogue notes of
            // EnableFastTravel].
            let placed = self.world.placed;
            let was_interior = !self.loaded_cells.is_empty() && self.loaded_cells.iter().all(|&l| placed.world_of(l).is_none());
            if was_interior && placed.world_of(c).is_some() && !self.player.fast_travel.keep {
                self.player.fast_travel = crate::game::FastTravel::default();
            }
            let cells = self.cells_around(c);
            self.set_loaded(cells);
        }
    }

    // ---- activation and other events -------------------------------------------

    /// `actor` activates `target`.
    pub fn activate(&mut self, actor: FormId, target: FormId, mode: ActMode) -> Result<serde_json::Value, String> {
        let has_block = self.ref_script(target).and_then(|s| self.data.script(s)).is_some_and(|u| u.has_block(event::ON_ACTIVATE));
        match mode {
            ActMode::Hand | ActMode::Block if has_block => {
                // The block runs inside the activation [observed: a variable the door's
                // OnActivate set was read on the next line of the script that called
                // `Activate player 1`, `L5.quest.ref_activation`].
                let ran = self.ref_event(target, event::ON_ACTIVATE, Some(Value::Ref(actor.0)), Some(actor));
                Ok(serde_json::json!({"result": "script", "ran": ran}))
            }
            ActMode::Hand | ActMode::Block | ActMode::Default => self.default_activation(actor, target, mode == ActMode::Hand),
        }
    }

    /// The default action of an activation (Layer 4), with its Layer 5 consequences:
    /// a picked-up item reaches the actor's inventory (OnAdd), a door moves the player, a
    /// dead actor opens its inventory (the loot menu) [observed: a scripted Activate of a
    /// dead actor opened the container menu, `L5.quest.dead_activation`].
    pub fn default_activation(&mut self, actor: FormId, target: FormId, by_hand: bool) -> Result<serde_json::Value, String> {
        let base = functions::base_of(self, target);
        let dead_actor = self.extra.get(&target).is_some_and(|e| e.dead)
            && base.and_then(|b| self.data.sig(b)).is_some_and(|s| &s == b"NPC_" || &s == b"CREA");
        let act = if dead_actor && self.present(target) {
            let script = base.and_then(|b| self.world.forms.get(b)).and_then(|b| b.script);
            fnv_sim::world::Activation { outcome: Outcome::OpenedContainer { container: target }, unlocked_with: None, script }
        } else if by_hand {
            self.world.activate(actor, target).map_err(|e| e.to_string())?
        } else {
            self.world.activate_scripted(actor, target).map_err(|e| e.to_string())?
        };
        let v = serde_json::to_value(&act).map_err(|e| e.to_string())?;
        self.emit(EventKind::Activated { reference: target, by: actor, outcome: Some(v.clone()) });
        if matches!(act.outcome, Outcome::Activated) {
            self.radio_activated(target);
        }
        match act.outcome {
            Outcome::PickedUp { item, count, .. } => {
                self.emit(EventKind::ItemAdded { to: actor, item, count });
                self.item_event(actor, item, event::ON_ADD, Some(Value::Ref(actor.0)));
            }
            Outcome::Teleported { cell, .. } if actor == FormId::PLAYER_REF => {
                self.player_entered(cell);
                self.bring_teammates();
            }
            // A placed mine disarmed by the player gives its XP (`crate::experience`).
            Outcome::Disarmed { .. } if actor == FormId::PLAYER_REF => {
                let x = crate::experience::mine_disarm_xp(self);
                crate::experience::award(self, crate::experience::Source::DisarmMine, x);
            }
            _ => {}
        }
        Ok(v)
    }

    /// An inventory event on a scripted item (OnAdd, OnDrop, OnEquip, OnUnequip): the
    /// item's script runs in an instance per holder and item [hypothesis].
    pub fn item_event(&mut self, holder: FormId, item: FormId, ev: u16, subject: Option<Value>) -> bool {
        let Some(script) = self.data.script_of_base(item) else { return false };
        if !self.data.script(script).is_some_and(|u| u.has_block(ev)) {
            return false;
        }
        let key = InstKey::Item { holder, item };
        let ran = self.run_event(key, script, ev, subject, RunCtx { self_ref: None, action_ref: Some(holder), quest: None });
        if ran {
            let block = self.data.table.block_name(ev).unwrap_or("?").to_string();
            let unit = self.data.script(script).map(|u| u.key()).unwrap_or_default();
            self.emit(EventKind::BlockRun { unit, block, on: Some(holder) });
        }
        ran
    }

    /// A dialogue line was said (runner input standing in for Layer 6 dialogue): its
    /// begin and end result scripts run on the speaker.
    ///
    /// The line's conditions are evaluated first (subject the speaker, target the player)
    /// and a line whose conditions fail is refused unless `force` is set: in the game such
    /// a line would not be said. Returns whether the conditions passed.
    pub fn run_info(&mut self, info: FormId, speaker: Option<FormId>, force: bool) -> Result<bool, String> {
        let data = self.data;
        let def = data.infos.get(&info).ok_or_else(|| format!("{info} is not a dialogue line (INFO)"))?;
        let speaker = speaker.or(def.speaker);
        let pass = self.conditions_pass(&def.conditions, speaker, Some(FormId::PLAYER_REF), &format!("INFO {info}"));
        if !pass && !force {
            let why = self.explain_conditions(&def.conditions, speaker, Some(FormId::PLAYER_REF));
            return Err(format!("the conditions of {info} do not pass: {}", why.join("; ")));
        }
        if let Some(sp) = speaker {
            self.mark_said(sp, info, false);
        }
        for u in [def.begin, def.end].into_iter().flatten() {
            self.emit(EventKind::ResultScript { unit: data.unit(u).key(), on: speaker });
            self.run_result(u, RunCtx { self_ref: speaker, action_ref: None, quest: def.quest });
        }
        Ok(pass)
    }

    /// A package event (runner input standing in for Layer 6 AI): the package's result
    /// script (0 begin, 1 end, 2 change) and the actor's OnPackageStart / OnPackageDone /
    /// OnPackageChange blocks.
    pub fn package_event(&mut self, actor: FormId, package: FormId, which: usize) {
        let data = self.data;
        if which != 1 {
            self.extra.entry(actor).or_default().current_package = Some(package);
        }
        if let Some(Some(u)) = data.packages.get(&package).map(|p| p[which]) {
            self.emit(EventKind::ResultScript { unit: data.unit(u).key(), on: Some(actor) });
            self.run_result(u, RunCtx { self_ref: Some(actor), ..Default::default() });
        }
        // The idle follows the result script, so it sees what the script set
        // [documented: GECK wiki, Category:Packages].
        self.ask_package_idle(actor, package, which);
        let ev = [event::ON_PACKAGE_START, event::ON_PACKAGE_DONE, event::ON_PACKAGE_CHANGE][which];
        self.ref_event(actor, ev, Some(Value::Ref(package.0)), None);
    }

    // AI package choice: crate::ai.

    // ---- magic effects (docs/spec-notes/scripts.md §7.6) --------------------------

    /// Apply the effects of a magic item (an ingestible the actor consumes, a spell
    /// cast on it, an ability added to it) to `target`. Each effect whose conditions
    /// pass (subject: the target) becomes an [`ActiveEffect`]; an effect whose base
    /// effect is a Script effect runs its ScriptEffectStart block at once, inside this
    /// call, in an instance of its own [observed: Nuka-Cola's cap, Cateye's Nighteye and
    /// Contreras' supplies' restored limb were there on the line after `EquipItem`,
    /// layer5-captures.md §9.3 b; hypothesis: per effect, in the item's effect order].
    ///
    /// An effect of duration 0 that is not permanent finishes inside the call as well
    /// (ScriptEffectFinish right after the start): nothing of it runs afterwards
    /// [observed: a limb damaged again after Contreras' supplies (duration 0) stayed
    /// damaged in the next 20 frames, §9.3 b; hypothesis: Finish runs inside the call
    /// rather than not at all; the supplies' blocks all do the same].
    ///
    /// The same item applied again while its effect is still active adds its duration
    /// to the active effect: one effect, one Finish [observed: a second Cateye 10.2 s
    /// after the first ended 240 s after the first, with no second Finish, §9.3 b;
    /// hypothesis: the start block does not run again, and only effects of the same
    /// item (and effect index) merge]. Returns the effects started.
    pub fn apply_magic_item(&mut self, target: FormId, source: FormId, caster: Option<FormId>, permanent: bool) -> Vec<u32> {
        self.apply_magic_item_with(target, source, caster, permanent, &crate::aid::Mods::default()).into_iter().filter(|a| !a.extended).map(|a| a.id).collect()
    }

    /// [`Game::apply_magic_item`] with the beneficial Value Modifier effects' magnitudes
    /// multiplied by `mods.potency` (an ingestible's Medicine or Food potency), the
    /// beneficial effects' durations by `mods.duration` and the rads given by
    /// `mods.rads` (perks; `crate::aid::Mods`). Returns the effects started and extended,
    /// with their value changes as applied.
    pub fn apply_magic_item_with(&mut self, target: FormId, source: FormId, caster: Option<FormId>, permanent: bool, mods: &crate::aid::Mods) -> Vec<crate::aid::Applied> {
        let data = self.data;
        let Some(items) = data.effect_items.get(&source) else { return Vec::new() };
        let mut started: Vec<crate::aid::Applied> = Vec::new();
        for (index, e) in items.iter().enumerate() {
            let label = format!("{} effect {index}", data.name(source));
            // A lasting effect (an ability, a worn enchantment) whose conditions fail is
            // listed inactive, to take hold when they pass (`crate::abilities`).
            let pass = self.conditions_pass(&e.conditions, Some(target), caster, &label);
            if !pass && !permanent {
                continue;
            }
            let seconds = mods.seconds(data.magic_effects.get(&e.effect), e);
            if !permanent && e.duration > 0 {
                let active = self
                    .effects
                    .iter_mut()
                    .find(|x| !x.permanent && x.target == target && x.source == source && x.index == index);
                if let Some(x) = active {
                    if mods.hit {
                        // A hit's effect starts its time again (hit-effects.md §2).
                        x.duration = x.duration.max(x.elapsed + seconds);
                    } else {
                        x.duration += seconds;
                    }
                    // The usage monitor counts every use while it lasts (aid-items.md §5).
                    if e.effect == crate::aid::USAGE_MONITOR {
                        x.usage += e.magnitude as f64;
                    }
                    let (id, duration, change) = (x.id, x.duration, x.change);
                    self.emit(EventKind::EffectExtended { id, target, source, duration });
                    started.push(crate::aid::Applied { id, effect: e.effect, change, duration, extended: true });
                    continue;
                }
            }
            let id = self.next_effect;
            self.next_effect += 1;
            let script = data.magic_effects.get(&e.effect).and_then(|m| m.script).filter(|s| data.scripts.contains_key(s));
            let resisted = if mods.hit { self.resisted(target, data.magic_effects.get(&e.effect)) } else { 0.0 };
            if resisted >= 1.0 {
                continue;
            }
            let change = data
                .magic_effects
                .get(&e.effect)
                .and_then(|m| crate::aid::value_change(m, e, mods.potency, permanent))
                .map(|c| mods.on_change(c))
                .map(|mut c| {
                    c.amount *= 1.0 - resisted;
                    c
                });
            let usage = if e.effect == crate::aid::USAGE_MONITOR { e.magnitude as f64 } else { 0.0 };
            self.effects.push(ActiveEffect {
                id,
                target,
                source,
                effect: e.effect,
                index,
                magnitude: e.magnitude,
                duration: seconds,
                elapsed: 0.0,
                permanent,
                caster,
                script,
                change,
                usage,
                worn: mods.worn,
                inactive: !pass,
            });
            self.emit(EventKind::EffectStarted { id, target, source, effect: e.effect });
            if !pass {
                started.push(crate::aid::Applied { id, effect: e.effect, change, duration: seconds, extended: false });
                continue;
            }
            self.start_change(id);
            let change = self.effects.iter().find(|x| x.id == id).and_then(|x| x.change);
            started.push(crate::aid::Applied { id, effect: e.effect, change, duration: seconds, extended: false });
            self.effect_archetype_start(id);
            self.effect_event(id, event::SCRIPT_EFFECT_START, 0.0);
            if !permanent && e.duration == 0 && self.effects.iter().any(|x| x.id == id) {
                self.finish_effect(id, true);
            }
        }
        started
    }

    /// The player (or an actor) consumes an ingestible (ALCH) from its inventory: one is
    /// removed and it is used ([`Game::ingest`]: its effects with the item's potency, the
    /// usage monitor and the addiction roll). EquipItem of an aid item does the same.
    pub fn consume(&mut self, actor: FormId, item: FormId) -> Result<Vec<u32>, String> {
        let removed = self.world.remove_item(actor, item, 1).map_err(|e| e.to_string())?;
        if removed == 0 {
            return Err(format!("{} carries no {}", self.data.name(actor), self.data.name(item)));
        }
        self.emit(EventKind::ItemRemoved { from: actor, item, count: 1 });
        Ok(self.ingest(actor, item).effects)
    }

    /// ScriptEffectStart of an effect taking hold later (`crate::abilities`).
    pub(crate) fn effect_started(&mut self, id: u32) {
        self.effect_event(id, event::SCRIPT_EFFECT_START, 0.0);
    }

    /// ScriptEffectFinish of an effect letting go while it stays listed.
    pub(crate) fn effect_finished(&mut self, id: u32) {
        self.effect_event(id, event::SCRIPT_EFFECT_FINISH, 0.0);
    }

    /// Run an effect's script block (if its script has one) on the effect's target.
    fn effect_event(&mut self, id: u32, ev: u16, elapsed: f64) {
        let Some(e) = self.effects.iter().find(|x| x.id == id) else { return };
        let (Some(script), target, caster) = (e.script, e.target, e.caster) else { return };
        if !self.data.script(script).is_some_and(|u| u.has_block(ev)) {
            return;
        }
        let before = std::mem::replace(&mut self.effect_elapsed, elapsed);
        if ev == event::SCRIPT_EFFECT_UPDATE {
            self.mark_run(InstKey::Effect { id }, Some(script));
        }
        self.run_event(InstKey::Effect { id }, script, ev, None, RunCtx { self_ref: Some(target), action_ref: caster, quest: None });
        self.effect_elapsed = before;
    }

    /// End an effect: its ScriptEffectFinish block runs (when `run_finish`), then it and
    /// its script instance are dropped.
    pub fn finish_effect(&mut self, id: u32, run_finish: bool) {
        if !self.effects.iter().any(|x| x.id == id) {
            return;
        }
        // An inactive effect never started: nothing of it runs.
        let inactive = self.effects.iter().any(|x| x.id == id && x.inactive);
        if run_finish && !inactive {
            self.effect_event(id, event::SCRIPT_EFFECT_FINISH, 0.0);
        }
        self.end_change(id);
        if let Some(i) = self.effects.iter().position(|x| x.id == id) {
            let e = self.effects.remove(i);
            self.instances.remove(&InstKey::Effect { id });
            self.emit(EventKind::EffectFinished { id, target: e.target, source: e.source });
        }
    }

    /// One frame of active effects (game time only): each runs ScriptEffectUpdate with
    /// ScriptEffectElapsedSeconds = the frame time, then an effect whose duration has
    /// passed finishes. Observed (layer5-captures.md §9.3 b): Update runs every frame
    /// (an ability restoring ScriptEffectElapsedSeconds / 10 health changed it in 59 of
    /// 59 frame steps, at 0.1 HP per second of script time, so Elapsed is the time since
    /// the previous Update, as documented on the GECK wiki); an open menu stops Update
    /// and the duration; Cateye's 120 s effect finished 119.82–119.94 s of
    /// GetSecondsPassed time after the use. Hypothesis: the effects run after the
    /// frame's quest and object scripts, and effects on dead actors keep running.
    /// Active effects advance by `seconds` at once, as if that much game-mode time had
    /// passed in one frame (a wait or a sleep, `crate::needs`): each runs one
    /// ScriptEffectUpdate with those seconds, a value changed by the second changes by
    /// the seconds inside its duration, and the effects whose duration has passed
    /// finish [hypothesis: aid-items.md §6].
    pub fn advance_effects(&mut self, seconds: f64) {
        if seconds > 0.0 {
            self.update_effects(seconds);
        }
    }

    fn update_effects(&mut self, dt: f64) {
        self.refresh_suspended(dt);
        self.recheck_conditions();
        let ids: Vec<u32> = self.effects.iter().filter(|e| !e.inactive).map(|e| e.id).collect();
        for id in ids {
            let Some(e) = self.effects.iter_mut().find(|x| x.id == id) else { continue };
            let before = e.elapsed;
            e.elapsed += dt;
            let done = !e.permanent && e.elapsed + 1e-9 >= e.duration;
            self.tick_change(id, before);
            self.effect_event(id, event::SCRIPT_EFFECT_UPDATE, dt);
            if done {
                self.finish_effect(id, true);
            }
        }
    }

    /// Remove the effects of `target` that `pick` selects. An effect whose own script is
    /// running stops that script like a return and does not run its finish block
    /// [documented: GECK wiki Dispel]; the others run ScriptEffectFinish [hypothesis].
    pub fn dispel(&mut self, target: FormId, running: Option<InstKey>, pick: impl Fn(&ActiveEffect) -> bool) -> usize {
        let ids: Vec<u32> = self.effects.iter().filter(|e| e.target == target && pick(e)).map(|e| e.id).collect();
        for &id in &ids {
            let own = running == Some(InstKey::Effect { id });
            if own {
                self.request_return();
            }
            self.finish_effect(id, !own);
        }
        ids.len()
    }

    // ---- image space modifiers ---------------------------------------------------

    /// ApplyImageSpaceModifier: the modifier starts playing. One that is already on the
    /// screen starts again from its beginning [hypothesis: one instance per modifier;
    /// image-modifiers.md §3].
    pub fn apply_image_modifier(&mut self, modifier: FormId) {
        self.image_modifiers.retain(|m| m.modifier != modifier);
        self.image_modifiers.push(ActiveImageModifier { modifier, elapsed: 0.0 });
        self.emit(EventKind::ImageModifierApplied { modifier });
    }

    /// RemoveImageSpaceModifier: whether the modifier was on the screen.
    pub fn remove_image_modifier(&mut self, modifier: FormId) -> bool {
        let before = self.image_modifiers.len();
        self.image_modifiers.retain(|m| m.modifier != modifier);
        let was = self.image_modifiers.len() != before;
        if was {
            self.emit(EventKind::ImageModifierRemoved { modifier, ended: false });
        }
        was
    }

    /// IsImageSpaceActive.
    pub fn image_modifier_active(&self, modifier: FormId) -> bool {
        self.image_modifiers.iter().any(|m| m.modifier == modifier)
    }

    /// Age the modifiers; an animatable one whose duration has passed ends [hypothesis
    /// from the shipped scripts: 29 lines wait for IsImageSpaceActive to read 0 on
    /// FadeToBlackISFX (4 s), which no script removes; image-modifiers.md §3].
    fn update_image_modifiers(&mut self, dt: f64) {
        let data = self.data;
        let mut ended = Vec::new();
        self.image_modifiers.retain_mut(|m| {
            m.elapsed += dt;
            let over = data.image_modifiers.get(&m.modifier).is_some_and(|d| d.animatable && m.elapsed + 1e-9 >= d.duration as f64);
            if over {
                ended.push(m.modifier);
            }
            !over
        });
        for modifier in ended {
            self.emit(EventKind::ImageModifierRemoved { modifier, ended: true });
        }
    }

    // ---- conditions --------------------------------------------------------------

    /// Evaluate a condition list (`subject` and `target` for the run-on choices).
    pub fn conditions_pass(&mut self, conds: &[Condition], subject: Option<FormId>, target: Option<FormId>, label: &str) -> bool {
        if conds.is_empty() {
            return true;
        }
        let mut results = Vec::with_capacity(conds.len());
        let outer = std::mem::replace(&mut self.condition_target, target);
        for c in conds {
            let on = match c.run_on {
                RunOn::Subject => subject,
                RunOn::Target => target,
                RunOn::Reference(r) => r.map(FormId),
                RunOn::LinkedReference => subject.and_then(|s| self.world.placed.get(s)).and_then(|p| p.linked),
                RunOn::CombatTarget | RunOn::Other(_) => None,
            };
            let v = self.condition_value(c, on, label);
            let rhs = match c.rhs {
                Operand::Value(x) => x,
                Operand::Global(Some(g)) => self.world.global(FormId(g)).map(|v| v as f64).unwrap_or(0.0),
                Operand::Global(None) => 0.0,
            };
            results.push((c.cmp.test(v, rhs), c.or));
        }
        self.condition_target = outer;
        cond::combine(&results)
    }

    /// The buttons of a message (indices in record order) whose conditions fail when it
    /// is shown: they are not shown, and the others keep their indices [documented: GECK
    /// wiki GetButtonPressed]. The conditions run on the player as subject and the
    /// reference whose script shows the message as target [hypothesis: the craft and
    /// repair buttons of Doc Mitchell's house test the player's skills with a fixed
    /// PlayerRef run-on, so the choice matters only for Subject/Target conditions].
    pub fn hidden_buttons(&mut self, message: FormId, shown_by: Option<FormId>) -> Vec<usize> {
        let data = self.data;
        let Some(def) = data.messages.get(&message) else { return Vec::new() };
        let mut hidden = Vec::new();
        for (i, c) in def.button_conditions.iter().enumerate() {
            if !c.is_empty() && !self.conditions_pass(c, Some(FormId::PLAYER_REF), shown_by, &format!("MESG {message} button {i}")) {
                hidden.push(i);
            }
        }
        hidden
    }

    /// Each condition with its value, for diagnostics ("GetStage(0x…) = 10 == 20: false").
    pub fn explain_conditions(&mut self, conds: &[Condition], subject: Option<FormId>, target: Option<FormId>) -> Vec<String> {
        let data = self.data;
        let mut out = Vec::new();
        for c in conds {
            let on = match c.run_on {
                RunOn::Subject => subject,
                RunOn::Target => target,
                RunOn::Reference(r) => r.map(FormId),
                RunOn::LinkedReference => subject.and_then(|s| self.world.placed.get(s)).and_then(|p| p.linked),
                RunOn::CombatTarget | RunOn::Other(_) => None,
            };
            let v = self.condition_value(c, on, "explain");
            let rhs = match c.rhs {
                Operand::Value(x) => x,
                Operand::Global(Some(g)) => self.world.global(FormId(g)).map(|v| v as f64).unwrap_or(0.0),
                Operand::Global(None) => 0.0,
            };
            let name = data.table.get(c.opcode()).map(|f| f.name.clone()).unwrap_or_else(|| format!("0x{:04X}", c.opcode()));
            let args: Vec<String> = c.params.iter().map(|a| arg_json(data, a).to_string()).collect();
            out.push(format!("{name}({}) = {v} {:?} {rhs}: {}{}", args.join(", "), c.cmp, c.cmp.test(v, rhs), if c.or { " OR" } else { "" }));
        }
        out
    }

    pub(crate) fn condition_value(&mut self, c: &Condition, on: Option<FormId>, label: &str) -> f64 {
        let ctx = Ctx {
            unit: label.to_string(),
            opcode: c.opcode(),
            on,
            self_ref: on,
            action_ref: None,
            quest: None,
            inst: None,
            in_condition: true,
        };
        // IsLastIdlePlayed has a condition index but no script opcode (`crate::idles`).
        if c.function == crate::idles::IS_LAST_IDLE_PLAYED {
            return crate::idles::is_last_idle_played(self, on, &c.params);
        }
        self.call_function(&ctx, &c.params).num()
    }

    /// Dispatch a function by opcode (scripts and conditions share it).
    pub fn call_function(&mut self, ctx: &Ctx, args: &[ArgValue]) -> Value {
        let data = self.data;
        let name = || data.table.get(ctx.opcode).map(|f| f.name.clone()).unwrap_or_else(|| format!("0x{:04X}", ctx.opcode));
        // A console function (a number of ours, no opcode of the game's) by the function's catalogue name.
        let console = || (ctx.opcode >= fnv_script::table::CONSOLE_OPCODE_BASE).then(|| data.table.get(ctx.opcode)).flatten().and_then(|f| functions::lookup_console(&f.name));
        match functions::lookup(ctx.opcode).or_else(console) {
            Some(Kind::Impl(f)) => match f(self, ctx, &Args(args)) {
                Ok(v) => v,
                Err(e) => {
                    let n = name();
                    self.emit(EventKind::CallFailed { opcode: format!("0x{:04X}", ctx.opcode), name: n, unit: ctx.unit.clone(), error: e });
                    Value::ZERO
                }
            },
            Some(Kind::NoOp(_)) => Value::ZERO,
            Some(Kind::Stub(_)) => {
                *self.stubbed.entry(ctx.opcode).or_default() += 1;
                self.stub_note(ctx, args);
                Value::ZERO
            }
            None => {
                *self.unhandled.entry(ctx.opcode).or_default() += 1;
                if self.unhandled_seen.insert((ctx.opcode, ctx.unit.clone())) {
                    let n = name();
                    tracing::debug!(opcode = ctx.opcode, name = %n, unit = %ctx.unit, "unhandled script function");
                    self.emit(EventKind::Unhandled { opcode: format!("0x{:04X}", ctx.opcode), name: n, unit: ctx.unit.clone() });
                }
                Value::ZERO
            }
        }
    }

    /// Trace a call to a function that is implemented for its Layer 5 part but whose
    /// effect belongs to Layer 6 (e.g. AddScriptPackage).
    pub fn stub_note(&mut self, ctx: &Ctx, args: &[ArgValue]) {
        let data = self.data;
        let name = data.table.get(ctx.opcode).map(|f| f.name.clone()).unwrap_or_default();
        let args: Vec<serde_json::Value> = args.iter().map(|a| arg_json(data, a)).collect();
        let on = ctx.on.or(ctx.self_ref);
        let sig = format!("{on:?}{args:?}");
        if !self.stubs_seen.insert((ctx.opcode, ctx.unit.clone(), sig)) {
            return;
        }
        self.emit(EventKind::Stub { opcode: format!("0x{:04X}", ctx.opcode), name, unit: ctx.unit.clone(), on, args });
    }

    // ---- snapshots -----------------------------------------------------------------

    pub fn snapshot(&self) -> GameSnapshot {
        GameSnapshot {
            format: SNAPSHOT_FORMAT.into(),
            sim: self.world.snapshot(),
            quests: self.quests.clone(),
            instances: self.instances.iter().map(|(k, v)| (*k, v.clone())).collect(),
            extra: self.extra.clone(),
            player: self.player.clone(),
            form_lists: self.form_lists.clone(),
            loaded_cells: self.loaded_cells.clone(),
            moved: self.moved.clone(),
            menu: self.menu,
            loading: self.loading,
            message_box: self.message_box.clone(),
            boxes: self.boxes.clone(),
            speech: self.speech.clone(),
            said: self.said.clone(),
            said_day: self.said_day.clone(),
            dialogue: self.dialogue.clone(),
            frame: self.frame,
            time: self.time,
            pending_onload: self.pending_onload.clone(),
            rng_word_pos: self.rng.get_word_pos().to_string(),
            forms: self.forms.clone(),
            effects: self.effects.clone(),
            next_effect: self.next_effect,
            image_modifiers: self.image_modifiers.clone(),
            locks: self.locks.clone(),
            terminals: self.terminals.access.clone(),
            terminal_locks: self.terminals.script_locks.clone(),
            pipboy_radio_on: self.radio.pipboy.on,
            pipboy_station: self.radio.pipboy.station,
            npc_radios: self.radio.npc_radios.clone(),
            weather_shown: self.weather.shown,
            raining: self.weather.raining,
            grudges: self.provocations.grudges.iter().map(|(k, v)| (*k, *v)).collect(),
            forgiven: self.provocations.forgiven.clone(),
        }
    }

    pub fn restore(data: &'a GameData, world: World<'a>, cfg: GameConfig, s: &GameSnapshot) -> Result<Game<'a>, String> {
        if s.format != SNAPSHOT_FORMAT {
            return Err(format!("unsupported snapshot format {:?}", s.format));
        }
        let mut g = Game::new(data, world, cfg);
        g.quests = s.quests.clone();
        g.instances = s.instances.iter().cloned().collect();
        g.extra = s.extra.clone();
        g.player = s.player.clone();
        g.form_lists = s.form_lists.clone();
        g.loaded_cells = s.loaded_cells.clone();
        g.moved = s.moved.clone();
        g.menu = s.menu;
        g.loading = s.loading;
        g.message_box = s.message_box.clone();
        g.boxes = s.boxes.clone();
        g.speech = s.speech.clone();
        g.said = s.said.clone();
        g.said_day = s.said_day.clone();
        g.dialogue = s.dialogue.clone();
        g.frame = s.frame;
        g.time = s.time;
        g.pending_onload = s.pending_onload.clone();
        g.forms = s.forms.clone();
        g.effects = s.effects.clone();
        g.next_effect = s.next_effect.max(1);
        g.image_modifiers = s.image_modifiers.clone();
        g.locks = s.locks.clone();
        g.terminals.access = s.terminals.clone();
        g.terminals.script_locks = s.terminal_locks.clone();
        g.radio.pipboy.on = s.pipboy_radio_on;
        g.radio.pipboy.station = s.pipboy_station;
        g.radio.npc_radios = s.npc_radios.clone();
        g.weather.shown = s.weather_shown;
        g.weather.raining = s.raining;
        g.provocations.grudges = s.grudges.iter().copied().collect();
        g.provocations.forgiven = s.forgiven.clone();
        let pos: u128 = s.rng_word_pos.parse().map_err(|_| format!("bad rng_word_pos {:?}", s.rng_word_pos))?;
        g.rng.set_word_pos(pos);
        Ok(g)
    }
}

/// Seconds of a line per response without its voice file: a base plus a time per
/// character of the response text. The game takes the length from the voice file; these
/// two numbers are fitted to the observed spans of Doc Mitchell's lines [hypothesis:
/// one line of 29 characters took 2.9 s, three of 193 characters 15.4 s and two of 129
/// characters 10 s from the stage before them to the stage their end script set,
/// layer5-captures.md §10.2–10.3; the fit gives 2.9, 15.4 and 10.3 s].
pub const LINE_BASE_SECONDS: f64 = 1.1;
pub const LINE_SECONDS_PER_CHAR: f64 = 1.0 / 16.0;

/// The fitted length of one response of `chars` characters ([`LINE_BASE_SECONDS`],
/// [`LINE_SECONDS_PER_CHAR`]). The runtime times responses through
/// [`Game::response_seconds`] only, which falls back to this.
pub fn fitted_seconds(chars: usize) -> f64 {
    LINE_BASE_SECONDS + chars as f64 * LINE_SECONDS_PER_CHAR
}

/// The fitted length of a line from its responses' character counts (for callers that
/// have no game state, such as the subtitle display; the runtime uses
/// [`Game::line_seconds`]).
pub fn line_seconds(response_chars: &[usize]) -> f64 {
    if response_chars.is_empty() {
        return fitted_seconds(0);
    }
    response_chars.iter().map(|&c| fitted_seconds(c)).sum()
}

/// An argument as trace JSON (forms by FormID, with the Editor ID when known).
pub fn arg_json(data: &GameData, a: &ArgValue) -> serde_json::Value {
    use serde_json::json;
    match a {
        ArgValue::Value(Value::Ref(f)) => match data.edid_of.get(&FormId(*f)) {
            Some(e) => json!(format!("{} ({e})", FormId(*f))),
            None => json!(FormId(*f).to_string()),
        },
        ArgValue::Value(Value::Num(x)) => json!(x),
        ArgValue::Enum(c) => json!({"enum": c}),
        ArgValue::Axis(c) => json!(c.to_string()),
        ArgValue::Str(s) => json!(s),
        ArgValue::Vars(v) => json!(v.iter().map(|x| x.num()).collect::<Vec<_>>()),
        ArgValue::Int32(i) => json!(i),
    }
}

/// The host the VM runs against for one script run.
struct Exec<'g, 'a> {
    game: &'g mut Game<'a>,
    unit: &'a Compiled,
    locals: Locals,
    ctx: RunCtx,
}

impl Exec<'_, '_> {
    fn inst_key(&self) -> Option<InstKey> {
        match self.locals {
            Locals::Instance(k) => Some(k),
            Locals::Temp(_) => None,
        }
    }

    fn global_form(&mut self, g: RefIndex) -> Result<FormId, String> {
        match self.unit.unit.slot(g) {
            Some(RefSlot::Form(f)) => Ok(FormId(f)),
            other => Err(format!("global slot {} is {other:?}", g.0)),
        }
    }

    fn remote(&mut self, r: RefIndex) -> Result<(InstKey, FormId), String> {
        let f = self.ref_value(r)?.as_form().ok_or_else(|| format!("reference slot {} is empty", r.0))?;
        self.game.instance_of(FormId(f)).ok_or_else(|| format!("{} runs no script", self.game.data.name(FormId(f))))
    }
}

impl Host for Exec<'_, '_> {
    fn get_var(&mut self, var: &VarRef) -> Result<Value, String> {
        match (var.on, var.var) {
            (_, VarSlot::Global { global }) => {
                let g = self.global_form(global)?;
                Ok(Value::Num(self.game.world.global(g).map_err(|e| e.to_string())? as f64))
            }
            (None, VarSlot::Local { index, .. }) => Ok(match &self.locals {
                Locals::Temp(m) => m.get(&index).copied().unwrap_or_default(),
                Locals::Instance(k) => self.game.var(*k, index),
            }),
            (Some(r), VarSlot::Local { index, .. }) => {
                let (key, script) = self.remote(r)?;
                if self.game.data.script(script).is_some_and(|c| c.var_type(index).is_none()) {
                    let unit = self.unit.key();
                    self.game.emit(EventKind::UndeclaredVariable { owner: key.label(), index, unit });
                }
                Ok(self.game.var(key, index))
            }
        }
    }

    fn set_var(&mut self, var: &VarRef, value: Value) -> Result<(), String> {
        let short = |l| matches!(l, fnv_script::ir::VarLetter::Short);
        match (var.on, var.var) {
            (_, VarSlot::Global { global }) => {
                let g = self.global_form(global)?;
                let v = value.num() as f32;
                self.game.world.set_global(g, v).map_err(|e| e.to_string())?;
                self.game.emit(EventKind::GlobalSet { global: g, value: v });
            }
            (None, VarSlot::Local { index, letter }) => match &mut self.locals {
                Locals::Temp(m) => {
                    let int = matches!(self.unit.var_type(index), Some(VarType::Int)) || (self.unit.var_type(index).is_none() && short(letter));
                    m.insert(index, if int { Value::Num(to_int_storage(value.num()) as f64) } else { value });
                }
                Locals::Instance(k) => {
                    let k = *k;
                    let script = self.game.instances.get(&k).and_then(|i| i.script);
                    let unit = self.unit.key();
                    self.game.store(k, script, index, value, &unit, short(letter));
                    if self.game.cfg.trace_locals {
                        let value = self.game.var(k, index);
                        let name = self.game.var_name(script, index);
                        self.game.emit(EventKind::VarSet { owner: k.label(), index, name, value });
                    }
                }
            },
            (Some(r), VarSlot::Local { index, letter }) => {
                let (key, script) = self.remote(r)?;
                let unit = self.unit.key();
                self.game.store(key, Some(script), index, value, &unit, short(letter));
                let value = self.game.var(key, index);
                let name = self.game.var_name(Some(script), index);
                self.game.emit(EventKind::VarSet { owner: key.label(), index, name, value });
            }
        }
        Ok(())
    }

    fn ref_value(&mut self, r: RefIndex) -> Result<Value, String> {
        match self.unit.unit.slot(r) {
            Some(RefSlot::Form(f)) => Ok(Value::Ref(f)),
            Some(RefSlot::Local(i)) => {
                let v = VarRef { on: None, var: VarSlot::Local { index: i as u16, letter: fnv_script::ir::VarLetter::Float } };
                self.get_var(&v)
            }
            Some(RefSlot::Unmapped(l)) => Err(format!("reference slot {} holds unmapped FormID 0x{l:08X}", r.0)),
            None => Err(format!("reference slot {} is not in the list", r.0)),
        }
    }

    fn call(&mut self, opcode: u16, on: Option<Value>, args: &[ArgValue]) -> Result<Value, String> {
        let on = match on {
            Some(v) => match v.as_form() {
                Some(f) => Some(FormId(f)),
                None => {
                    // A call on an empty reference variable stops the script silently
                    // for this run [documented: GECK wiki Ref Variable, Eval and Let
                    // pages, owner export; docs/spec-notes/script-language.md §7.3 row
                    // 1]; traced as `call_failed`.
                    self.game.stop_run = true;
                    let name = self.game.data.table.get(opcode).map(|f| f.name.clone()).unwrap_or_default();
                    let unit = self.unit.key();
                    self.game.emit(EventKind::CallFailed {
                        opcode: format!("0x{opcode:04X}"),
                        name,
                        unit,
                        error: "called on an empty reference".into(),
                    });
                    return Ok(Value::ZERO);
                }
            },
            None => None,
        };
        let ctx = Ctx {
            unit: self.unit.key(),
            opcode,
            on,
            self_ref: self.ctx.self_ref,
            action_ref: self.ctx.action_ref,
            quest: self.ctx.quest,
            inst: self.inst_key(),
            in_condition: false,
        };
        Ok(self.game.call_function(&ctx, args))
    }

    fn skipped(&mut self, what: &VmError) {
        let unit = self.unit.key();
        self.game.emit(EventKind::Skipped { unit, what: what.to_string() });
    }

    fn take_return(&mut self) -> bool {
        std::mem::take(&mut self.game.stop_run)
    }
}
