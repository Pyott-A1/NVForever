//! Part A (docs/design/layer4-play.md §4): crosshair, prompt, activation and keeping
//! render entities in line with the simulation.
//!
//! - `PlaySet::Activation` (while playing): find the target under the crosshair
//!   ([`pick`]: the collision ray, else references' render bounds), then E activates it
//!   through `fnv_sim::World::activate` and its outcome becomes messages
//!   ([`outcome`]): `RefChanged`, `TravelRequest` (travel behind a loading screen,
//!   `crate::loading`), `OpenContainer` or a placeholder line ([`HudMessage`]: logged,
//!   not drawn). The crosshair and the prompt under it are drawn by `crate::reticle`
//!   from [`Target`].
//! - A locked door or container without its key goes to the lockpicking menu
//!   (`crate::lockpick`), which opens or says why not.
//! - While a script has the player's controls off (`control::ControlLocks`,
//!   docs/spec-notes/controls.md §5): with movement off E does nothing.
//! - `PlaySet::Sync`: `RefVisual` entities are hidden while the simulation says their
//!   reference is not in the world, on `RefChanged` and when cells stream in.
//!
//! Test aid: `NVF_PLAY_TEST_ACTIVATE=<seconds>[,<seconds>...]` presses E at those times
//! after start (unattended in-game checks).
//!
//! With Layer 5 scripts running, E goes through `script::activate_by_hand`: a target
//! whose script has an OnActivate block runs it instead of the default action (whose
//! effects then come from the script's own `Activate`, applied by `script.rs`).

pub mod outcome;
pub mod pick;
#[cfg(test)]
mod tests;

use crate::phys::PhysState;
use crate::player::Player;
use crate::sim::SimState;
use crate::loading::TravelRequest;
use crate::{space, OpenContainer, PlaySet, PlayState, PlayWorld, RefChanged, RefVisual, Travel};
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use fnv_phys::{layers, Filter, Owner};
use fnv_sim::forms::BaseData;
use fnv_sim::world::Outcome;
use fnv_sim::FormId;
use outcome::{Action, Effect};
use pick::{Pick, RayHit, Source, REACH};
use std::collections::HashMap;

pub struct ActivatePlugin;

impl Plugin for ActivatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Target>()
            .init_resource::<HudMessage>()
            .init_resource::<ActivateNext>()
            .insert_resource(AutoActivate::from_env())
            .add_systems(Update, (find_target, activate).chain().in_set(PlaySet::Activation).run_if(in_state(PlayState::Playing)))
            .add_systems(Update, sync_visuals.in_set(PlaySet::Sync));
    }
}

/// The reference under the crosshair, if it can be activated.
#[derive(Resource, Default, Debug)]
pub struct Target(pub Option<Targeted>);

#[derive(Debug, Clone, PartialEq)]
pub struct Targeted {
    pub form_id: u32,
    pub name: String,
    pub action: Action,
    pub distance: f32,
    pub source: Source,
    /// Someone else owns it: taking from it, or opening its lock, is a crime
    /// (`crate::crime`); the prompt is drawn in the crime colour.
    pub crime: bool,
}

/// A reference to activate as the player would, as soon as play resumes, whatever is
/// under the crosshair: a lock just picked or forced (`crate::lockpick`; playtest list 5
/// item 64).
#[derive(Resource, Default, Debug)]
pub struct ActivateNext(pub Option<u32>);

/// The last placeholder line: what the engine has no message of the game's for (an
/// item picked up, a door opened, a kill, a key that does nothing yet). It is logged
/// and kept here for the tests, never drawn: the owner wants no such white text on
/// screen (list 5, items 43 and 50). A line the game itself shows goes to the corner
/// messages (`crate::corner_messages::CornerMessages`).
#[derive(Resource, Default)]
pub struct HudMessage {
    text: String,
}

impl HudMessage {
    /// Note a placeholder line (replacing the last one); it is logged, not drawn.
    pub fn show(&mut self, text: String) {
        debug!("placeholder line, not drawn: {text}");
        self.text = text;
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

// ---- targeting -------------------------------------------------------------------

/// Name and action of a reference the player can activate now: present in the
/// simulation and of an activatable type. Activators and furniture need a display name
/// (FULL) [hypothesis: unnamed activators are trigger volumes and markers the player is
/// not meant to use; not captured]; placed leveled items and explosives with no weapon
/// to give are not offered.
pub fn describe(sim: &SimState, index: &fnv_world::WorldIndex, form_id: u32) -> Option<(String, Action)> {
    if !sim.present(form_id) {
        return None;
    }
    let id = FormId(form_id);
    let base = sim.forms.get(sim.base_of(form_id)?)?;
    let name = sim.name(base.id)?.to_string();
    let state = sim.world.peek(id);
    let placed = sim.refs.get(id);
    // A reference with lock data starts locked [hypothesis, as fnv_sim::World does].
    let lock = match state {
        Some(s) => s.lock.filter(|l| l.locked).map(|l| (l.level, l.key)),
        None => placed.and_then(|p| p.lock).map(|l| (l.level, l.key)),
    };
    let carried = |k: FormId| {
        sim.world.peek(FormId::PLAYER_REF).and_then(|p| p.inventory.as_ref()).is_some_and(|inv| inv.count(k) > 0)
    };
    let locked = |unlocked: Action| match lock {
        Some((_, Some(k))) if carried(k) => Action::Unlock,
        Some((level, _)) => Action::Locked { level },
        None => unlocked,
    };
    let action = match &base.data {
        BaseData::Item(_) => Action::Take { count: state.map_or_else(|| sim.forms.placed_count(base.id, placed.and_then(|p| p.count)), |s| s.count) },
        BaseData::Container(_) => locked(Action::Open),
        BaseData::Door(_) => locked(match placed.and_then(|p| p.teleport) {
            Some(tp) => Action::Travel { to: destination_name(sim, index, tp.door, tp.pos) },
            None if state.is_some_and(|s| s.open) => Action::Close,
            None => Action::Open,
        }),
        BaseData::Actor(_) => Action::Talk,
        BaseData::Activator if &base.sig == b"TERM" => Action::Access,
        BaseData::Activator | BaseData::Furniture => {
            base.name.as_ref()?;
            Action::Activate
        }
        BaseData::LeveledItem(_) | BaseData::Projectile { item: None } => return None,
        BaseData::Projectile { item: Some(_) } => Action::Disarm,
    };
    Some((name, action))
}

/// Name of the cell a door leads into: the destination door's cell, or for a worldspace
/// the exterior cell holding the destination, by display name, else its worldspace's,
/// else its Editor ID.
fn destination_name(sim: &SimState, index: &fnv_world::WorldIndex, door: FormId, pos: [f32; 3]) -> Option<String> {
    let cell = sim.refs.get(door)?.cell?;
    let cell = sim.refs.exterior_cell_at(cell, pos).unwrap_or(cell);
    let c = index.cells.get(&cell.0)?;
    c.name
        .clone()
        .or_else(|| c.world.and_then(|w| index.worldspaces.get(&w)).and_then(|w| w.name.clone()))
        .or_else(|| c.edid.clone())
}

/// The collision ray under the crosshair (the player's capsule and trigger volumes
/// ignored), then the render-bounds fallback (`pick`). In first person it is the eye's
/// view ray; in third person the camera's, through the crosshair over the shoulder
/// (`control::Crosshair`; playtest list 4, item 11).
#[allow(clippy::too_many_arguments)]
fn find_target(
    player: Option<Res<Player>>,
    phys: Option<ResMut<PhysState>>,
    mut sim: ResMut<SimState>,
    play: Res<PlayWorld>,
    visuals: Query<(&RefVisual, &Aabb, &GlobalTransform, &InheritedVisibility)>,
    mut target: ResMut<Target>,
    mut script: Option<ResMut<crate::script::ScriptState>>,
    control: Option<Res<crate::control::PlayerControl>>,
    mut talkers: Local<HashMap<u32, (bool, f64)>>,
) {
    let Some(player) = player else { return };
    let crosshair = control.as_deref().and_then(|c| c.crosshair);
    let (eye, dir) = crate::control::Crosshair::ray(crosshair.as_ref(), player.eye(), player.look_dir());
    let hit = phys.and_then(|mut p| {
        let mut filter = p.player.as_ref().map_or_else(Filter::default, |c| Filter::excluding(c.key()));
        // Trigger volumes do not block the view [hypothesis].
        filter.exclude_layers.insert(layers::TRIGGER);
        let h = p.world.queries().ray(eye, dir, REACH, &filter)?;
        let form_id = match h.owner {
            // An actor's hit volume (combat C3) targets the actor.
            Owner::Ref { form_id, .. } | Owner::Actor { form_id, .. } => Some(form_id),
            _ => None,
        };
        Some(RayHit { form_id, distance: h.distance })
    });
    // Render bounds are in Bevy space; distances are game units in both.
    let (o, d) = (space::to_bevy(eye), space::to_bevy(dir));
    let bounds = visuals.iter().filter(|(_, _, _, v)| v.get()).filter_map(|(r, aabb, gt, _)| {
        let t = gt.affine();
        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        let (sc, sr) = pick::bounding_sphere(&t, c, h);
        if sc.distance(o) - sr > REACH {
            return None;
        }
        Some((r.0, pick::ray_box(o, d, &t, c, h)?))
    });
    let mut described: HashMap<u32, Option<(String, Action)>> = HashMap::new();
    // A dead actor is searched rather than talked to (combat C5); a living one is talked
    // to only when it may be ([`actor_action`]). Actors are named by their template's
    // name where the template supplies it (playtest list 4, item 20).
    let mut rt = script.as_deref_mut().and_then(|s| s.rt.as_mut());
    let mut describe_once = |id: u32| {
        described
            .entry(id)
            .or_insert_with(|| {
                describe(&sim, play.index, id).map(|(n, a)| match a {
                    Action::Talk => {
                        let name = sim.ref_name(rt.as_deref().map(|rt| rt.game.data), id).map_or(n, str::to_string);
                        (name, actor_action(rt.as_deref_mut(), &mut sim, id, &mut talkers))
                    }
                    a => (n, a),
                })
            })
            .clone()
    };
    let picked = pick::choose(hit, bounds, REACH, |id| describe_once(id).is_some());
    let chosen = picked.and_then(|p| describe_once(p.form_id).map(|d| (p, d)));
    let game = script.as_deref().and_then(|s| s.rt.as_ref()).map(|rt| &rt.game);
    let new = chosen.map(|(Pick { form_id, distance, source }, (name, action))| {
        let crime = owned_by_other(game, &sim, form_id, &action);
        Targeted { form_id, name, action, distance, source, crime }
    });
    let key = |t: &Option<Targeted>| t.as_ref().map(|t| (t.form_id, t.action.clone(), t.crime));
    if key(&target.0) != key(&new) {
        match &new {
            Some(t) => info!("target 0x{:08X} {} ({}{}) by {:?} at {:.0}", t.form_id, t.name, t.action.label(), if t.crime { ", owned" } else { "" }, t.source, t.distance),
            None => info!("target none"),
        }
    }
    if target.0 != new {
        target.0 = new;
    }
}

/// Seconds a creature's "has something to say" answer is kept before it is asked again.
const TALKER_SECONDS: f64 = 2.0;

/// What E does to a living or dead actor (playtest list 4, item 19): a dead one is
/// searched (combat C5); one hostile to the player (it would attack the player, or is
/// fighting the player) shows only its name; a creature is talked to only when it has a
/// greeting it can say now ([`fnv_game::Game::has_greeting`]), else shows only its name;
/// any other NPC is talked to [hypothesis: the game offers Talk on creatures that have
/// dialogue, such as Rex and ED-E, and not on animals; an NPC without a line keeps the
/// prompt and says nothing, as before]. Without the scripts: Talk.
fn actor_action(rt: Option<&mut crate::script::Runtime>, sim: &mut SimState, id: u32, talkers: &mut HashMap<u32, (bool, f64)>) -> Action {
    let Some(rt) = rt else { return Action::Talk };
    let creature = sim.refs.get(FormId(id)).is_some_and(|p| &p.sig == b"ACRE");
    crate::script::with_world(rt, sim, |rt| actor_action_in(&mut rt.game, creature, id, talkers))
}

fn actor_action_in(g: &mut fnv_game::Game, creature: bool, id: u32, talkers: &mut HashMap<u32, (bool, f64)>) -> Action {
    let r = FormId(id);
    if g.is_dead(r) {
        return Action::Search;
    }
    let fighting_player = g.awareness.get(r).is_some_and(|a| a.state.in_combat() && a.targets.contains_key(&FormId::PLAYER_REF));
    if fighting_player || g.would_attack(r, FormId::PLAYER_REF) {
        return Action::Name;
    }
    if !creature {
        return Action::Talk;
    }
    let now = g.time;
    let talks = match talkers.get(&id) {
        Some(&(t, at)) if (now - at).abs() < TALKER_SECONDS => t,
        _ => {
            let t = g.has_greeting(r);
            talkers.insert(id, (t, now));
            t
        }
    };
    if talks {
        Action::Talk
    } else {
        Action::Name
    }
}

/// Whether acting on the target is a crime: an item to take, a container to open or a
/// lock to pick that someone other than the player owns (`fnv_game::crime`). A dead
/// actor's inventory is not [hypothesis].
fn owned_by_other(game: Option<&fnv_game::Game>, sim: &SimState, form_id: u32, action: &Action) -> bool {
    let Some(g) = game else { return false };
    let container = sim.base_of(form_id).and_then(|b| sim.forms.get(b)).is_some_and(|b| matches!(b.data, BaseData::Container(_)));
    let judged = match action {
        Action::Take { .. } | Action::Locked { .. } => true,
        Action::Open | Action::Unlock => container,
        _ => false,
    };
    judged && g.ownership_in(&sim.world, FormId(form_id)).is_some_and(|o| !g.counts_as_owner(FormId::PLAYER_REF, &o))
}

/// Test aid: times (seconds after start) at which E counts as pressed.
#[derive(Resource, Default)]
struct AutoActivate {
    times: Vec<f32>,
}

impl AutoActivate {
    fn from_env() -> AutoActivate {
        let times = std::env::var("NVF_PLAY_TEST_ACTIVATE")
            .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
            .unwrap_or_default();
        AutoActivate { times }
    }

    /// Whether a time has come since the last call (each fires once).
    fn fire(&mut self, now: f32) -> bool {
        let before = self.times.len();
        self.times.retain(|&t| t > now);
        self.times.len() < before
    }
}

/// Activate (E) activates the target; the outcome's effects go out as messages.
#[allow(clippy::too_many_arguments)]
fn activate(
    input: crate::bindings::ControlInput,
    time: Option<Res<Time>>,
    clock: Option<Res<crate::pause::PlayClock>>,
    auto: Option<ResMut<AutoActivate>>,
    (target, next, vats): (Res<Target>, Option<ResMut<ActivateNext>>, Option<Res<crate::vats_mode::VatsMode>>),
    mut sim: ResMut<SimState>,
    play: Res<PlayWorld>,
    mut hud: ResMut<HudMessage>,
    mut travel: MessageWriter<TravelRequest>,
    mut open: MessageWriter<OpenContainer>,
    mut changed: MessageWriter<RefChanged>,
    (mut sit, mut sounds, mut terminals, mut lockpicks): (
        MessageWriter<crate::furniture::UseFurniture>,
        MessageWriter<crate::world_sounds::Activated>,
        MessageWriter<crate::terminal::OpenTerminal>,
        MessageWriter<crate::lockpick::OpenLockpick>,
    ),
    mut seat: ResMut<crate::furniture::PlayerSeat>,
    script: Option<ResMut<crate::script::ScriptState>>,
    locks: Option<Res<crate::control::ControlLocks>>,
    stats: Option<ResMut<crate::control::LockStats>>,
) {
    // A reference queued by a lock just opened: activated as if E were pressed on it.
    let queued = next.and_then(|mut n| n.0.take()).and_then(|id| {
        let (name, action) = describe(&sim, play.index, id)?;
        info!("activate 0x{id:08X} {name}: its lock was just opened");
        Some(Targeted { form_id: id, name, action, distance: 0.0, source: Source::Bounds, crime: false })
    });
    let fired = match (auto, time) {
        // The aids' clock goes on while a menu holds the picture (`crate::pause`).
        (Some(mut a), Some(t)) => a.fire(t.elapsed_secs() + clock.map_or(0.0, |c| c.stopped_secs())),
        _ => false,
    };
    if !input.just_pressed(crate::bindings::Control::Activate) && !fired && queued.is_none() {
        return;
    }
    // VATS mode has Activate for itself (it fires the marks, docs/design/vats-mode.md §11.5).
    if vats.is_some_and(|v| v.holds_input()) {
        return;
    }
    // Scripts turned the player's movement off: world objects cannot be activated
    // [documented: GECK wiki DisablePlayerControls, MovementFlag].
    if locks.is_some_and(|l| l.movement) {
        info!("activate: ignored, a script has disabled the player's controls (movement)");
        if let Some(mut s) = stats {
            s.held[crate::control::group::MOVEMENT] += 1;
        }
        return;
    }
    let Some(t) = queued.as_ref().or(target.0.as_ref()) else {
        // On furniture with nothing under the crosshair (lying on a bed, the bed is out
        // of view): Activate gets up [design].
        seat.leave |= seat.hold.is_some();
        return;
    };
    // An actor shown by name only (hostile, or a creature with nothing to say): E does
    // nothing (playtest list 4, item 19).
    if t.action == Action::Name {
        info!("activate 0x{:08X} {}: nothing to do (name only)", t.form_id, t.name);
        return;
    }
    // The actor's place before, to put back if a teleport cannot be followed.
    let before = sim.world.peek(FormId::PLAYER_REF).map(|p| (p.cell, p.pos, p.rot));
    let a = match crate::script::activate_by_hand(script.map(|s| s.into_inner()), &mut sim, t.form_id) {
        Ok(crate::script::ByHand::Default(a)) => {
            sounds.write(crate::world_sounds::Activated { reference: t.form_id });
            a
        }
        Ok(crate::script::ByHand::Script) => {
            sounds.write(crate::world_sounds::Activated { reference: t.form_id });
            info!("activate 0x{:08X} {} ({}): its script's OnActivate ran", t.form_id, t.name, t.action.label());
            return;
        }
        Ok(crate::script::ByHand::Talk) => {
            // The dialogue screen opens (`crate::dialogue`).
            info!("activate 0x{:08X} {} ({}): a conversation started", t.form_id, t.name, t.action.label());
            return;
        }
        Ok(crate::script::ByHand::Greeted) => {
            // Heard and shown like a SayTo line; the player stays free.
            info!("activate 0x{:08X} {} ({}): it said its greeting, no conversation", t.form_id, t.name, t.action.label());
            return;
        }
        Err(e) => {
            warn!("activate 0x{:08X} {}: {e}", t.form_id, t.name);
            hud.show(format!("{}: {e}", t.name));
            return;
        }
    };
    info!("activate 0x{:08X} {} ({}): {:?}", t.form_id, t.name, t.action.label(), a);
    // Furniture's default activation: the player uses it (`crate::furniture`).
    let furniture = sim.base_of(t.form_id).and_then(|b| sim.forms.get(b)).is_some_and(|b| matches!(b.data, BaseData::Furniture));
    if furniture && matches!(a.outcome, Outcome::Activated) {
        sit.write(crate::furniture::UseFurniture { reference: t.form_id });
        return;
    }
    // A terminal's default activation: its screen opens (`crate::terminal`).
    let terminal = sim.base_of(t.form_id).and_then(|b| sim.forms.get(b)).is_some_and(|b| &b.sig == b"TERM");
    if terminal && matches!(a.outcome, Outcome::Activated) {
        terminals.write(crate::terminal::OpenTerminal { reference: t.form_id });
        return;
    }
    // Locked, without the key: the lockpicking menu, or a line saying why not
    // (`crate::lockpick`, docs/spec-notes/lockpicking.md §4).
    if matches!(a.outcome, Outcome::Locked { .. }) {
        lockpicks.write(crate::lockpick::OpenLockpick { reference: t.form_id });
        return;
    }
    let name_of = |id: FormId| sim.name(id).map_or_else(|| id.to_string(), str::to_string);
    let effects = outcome::effects(&a, t.form_id, &t.name, play.index, name_of);
    let travels = effects.iter().any(|e| matches!(e, Effect::Travel { .. }));
    if matches!(a.outcome, Outcome::Teleported { .. }) && !travels {
        if let (Some((cell, pos, rot)), Ok(p)) = (before, sim.world.get(FormId::PLAYER_REF)) {
            (p.cell, p.pos, p.rot) = (cell, pos, rot);
        }
    }
    for e in effects {
        match e {
            Effect::RefChanged(form_id) => {
                changed.write(RefChanged { form_id });
            }
            Effect::Travel { world, interior, feet, heading_deg } => {
                let to = match (interior, world) {
                    (Some(c), _) => format!("interior 0x{c:08X}"),
                    (None, Some(w)) => format!("worldspace 0x{w:08X}"),
                    (None, None) => "the worldspace".to_string(),
                };
                info!("travel to {to} at {feet:?} heading {heading_deg:.1}");
                travel.write(TravelRequest(Travel { world, interior, feet, heading_deg }));
            }
            Effect::OpenContainer(container) => {
                open.write(OpenContainer { container });
            }
            Effect::Message(m) => {
                info!("message: {m}");
                hud.show(m);
            }
        }
    }
}

// ---- visual sync -----------------------------------------------------------------

type NewVisuals<'w, 's> = Query<'w, 's, (&'static RefVisual, &'static mut Visibility), Added<RefVisual>>;
type AllVisuals<'w, 's> = Query<'w, 's, (&'static RefVisual, &'static mut Visibility)>;

/// Hide or show `RefVisual` entities from the simulation's state: every entity of a
/// reference named by `RefChanged`, and new entities as cells stream in (a taken item
/// must not come back when its cell reloads).
fn sync_visuals(
    sim: Res<SimState>,
    mut changes: MessageReader<RefChanged>,
    mut visuals: ParamSet<(NewVisuals, AllVisuals)>,
) {
    let mut present: HashMap<u32, bool> = HashMap::new();
    for (r, mut v) in &mut visuals.p0() {
        if !*present.entry(r.0).or_insert_with(|| sim.present(r.0)) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    let mut changed: HashMap<u32, usize> = changes.read().map(|m| (m.form_id, 0)).collect();
    if changed.is_empty() {
        return;
    }
    for (r, mut v) in &mut visuals.p1() {
        if let Some(n) = changed.get_mut(&r.0) {
            v.set_if_neq(if sim.present(r.0) { Visibility::Inherited } else { Visibility::Hidden });
            *n += 1;
        }
    }
    for (id, n) in changed {
        info!("reference 0x{id:08X} {}: {n} render entities updated", if sim.present(id) { "present" } else { "gone" });
    }
}
