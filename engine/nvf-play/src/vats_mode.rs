//! VATS mode (docs/design/vats-mode.md): NVForever's own VATS, a Dead Eye-style slow-time
//! mark and fire, not the original's [design: combat is not a parity target].
//!
//! - **Enter** (§3): V (`Control::Vats`) with a gun in hand, not reloading or jammed, a
//!   round loaded and the AP for a shot. With the weapon away V draws it first and enters
//!   once it is out.
//! - **Marking** (§6, §7): the clock runs at [`MARK_SCALE`] (eased in over the swipe,
//!   [`time_scale_at`]); left click (Use) marks the actor under the crosshair, on the body
//!   part the crosshair's ray strikes; each mark reserves the shot's AP
//!   (`fnv_combat::vats::MarkQueue`: no more marks than the AP left and the rounds loaded
//!   allow). Right click (Block) takes the last mark back, or with none ends the mode; V
//!   again ends it; E (Activate) fires.
//! - **Firing** (§9): the clock runs at [`FIRE_SCALE`] (half speed); every
//!   [`SHOT_INTERVAL`] of game time the next mark in
//!   order: the view snaps to it and `crate::combat` fires one shot straight at it
//!   ([`VatsMode::take_shot`]: no spread, no recoil, no jam, whatever the weapon's
//!   cooldown), and its AP is spent. [`AFTER_LAST`] after the last shot the mode is off
//!   and the clock is back at 1.
//! - **The look** (§5): the swipe, the amber grade and its fading out are four numbers
//!   ([`VatsMode::look`]) the renderer's image-space pass draws (nvforever
//!   `imagespace.rs`); the marks' "x"s are drawn here ([`draw_marks`]).
//! - **Sounds** (§10): the base game's `UIVATS*` SOUN records, found by Editor ID.
//! - **AP** (§7.2): reserved while marking (the HUD's bar shows what is left after the
//!   reservations, [`VatsMode::reserved`]), spent per shot; outside the mode spent AP comes
//!   back at `fActionPointsRestoreRate` × the maximum a second [hypothesis].
//! - **Scripts** (§12): `fnv_game::Game::set_vats` each frame (GetVATSMode, GetVATSValue).
//! - Any menu, death, a script disabling the fighting controls or the weapon leaving the
//!   hand ends the mode (§9.3, §11.6). Nothing of it is saved (§11.7).
//!
//! Log lines (smoke tests read them): `vats mode: on (…)`, `vats mode: refused: …`,
//! `vats mode: marked …`, `vats mode: mark refused: …`, `vats mode: unmarked …`,
//! `vats mode: firing …`, `vats mode: shot k of n …`, `vats mode: off (…)`, and at exit
//! `vats mode summary: …`.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fnv_combat::vats::{restored_damage, MarkQueue, Refusal, Volley, VolleyEvent};
use fnv_game::actor_values::ACTION_POINTS;
use fnv_phys::{layers, Filter, Owner};
use fnv_sim::FormId;
use std::collections::HashMap;

use crate::activate::HudMessage;
use crate::bindings::{Control, ControlInput};
use crate::combat::{Combat, WeaponAnim};
use crate::control::{ControlLocks, Crosshair, PlayerControl};
use crate::hitvol::ActorParts;
use crate::phys::PhysState;
use crate::player::Player;
use crate::script::{with_world, ScriptState};
use crate::sfx::{LoopSound, PlaySound, Sfx};
use crate::sim::SimState;
use crate::{space, PlaySet, PlayState, PlayerCamera};

/// The clock's speed while marking [design: the owner's "very, very slow"].
pub const MARK_SCALE: f32 = 0.05;
/// The swipe across the screen on entering, and the easing of the clock with it, real
/// seconds [design].
pub const ENTER_SWIPE: f32 = 0.35;
/// The wash over the whole screen after the swipe fades over this long, real seconds
/// [design, after the references' flash].
pub const ENTER_WASH: f32 = 0.6;
/// The wash's strength when the swipe has crossed [design].
pub const WASH_START: f32 = 0.5;
/// The band riding the swipe's edge [design].
pub const BAND_STRENGTH: f32 = 0.85;
/// The grade fades out over this long on leaving, real seconds [design].
pub const LEAVE_FADE: f32 = 0.3;
/// The clock's speed while the marked shots are fired [design: owner 2026-10-18, "the
/// firing is too fast": half speed instead of normal].
pub const FIRE_SCALE: f32 = 0.5;
/// Between two marked shots, game seconds: 0.4 s of real time at [`FIRE_SCALE`]
/// [design: owner 2026-10-18; was 0.12 at full speed].
pub const SHOT_INTERVAL: f32 = 0.2;
/// From the last shot to the end of the mode, game seconds (0.6 s real at
/// [`FIRE_SCALE`]) [design].
pub const AFTER_LAST: f32 = 0.3;
/// How far the crosshair's ray looks for a target when the projectile has no range of
/// its own, units [design].
pub const REACH: f32 = 8192.0;
/// A mark's "x": its size as a share of the screen width, and its colour [design, after
/// the references: a small dark-red x].
pub const MARK_SIZE: f32 = 0.019;
const MARK_COLOUR: Color = Color::srgb(0.45, 0.05, 0.03);
/// The most marks drawn at once (more are kept but not drawn) [engineering choice].
const MAX_DRAWN: usize = 32;

/// VATS mode's state (§2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Off,
    /// V was pressed with the weapon away: it is being drawn, and the mode starts once it
    /// is out.
    Drawing,
    Marking,
    Firing,
}

/// What was pressed this frame, for the mode.
#[derive(Debug, Clone, Copy, Default)]
struct Keys {
    toggle: bool,
    mark: bool,
    unmark: bool,
    accept: bool,
}

/// Where a mark is and what it is on.
#[derive(Debug, Clone)]
struct Spot {
    /// Its point now (game space): the part's capsule centre, followed while the actor
    /// moves; the last one seen once it has no volume.
    point: [f32; 3],
    name: String,
    part: String,
    /// The part's condition actor value (GetVATSValue Targeted Part).
    condition: Option<u16>,
}

/// The weapon VATS mode was entered with.
#[derive(Debug, Clone)]
struct InHand {
    id: FormId,
    reach: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VatsStats {
    pub entered: u32,
    pub marks: u32,
    pub shots: u32,
    pub ap_spent: f32,
    pub refused: u32,
}

#[derive(Resource, Default)]
pub struct VatsMode {
    pub phase: Phase,
    queue: MarkQueue,
    spots: Vec<Spot>,
    volley: Option<Volley>,
    weapon: Option<InHand>,
    /// Real seconds (`Time<Real>`) when marking began, and when the look began to go.
    entered_at: Option<f64>,
    left_at: Option<f64>,
    now: f64,
    keys: Keys,
    /// The point the shot `crate::combat` fires this frame aims at.
    shot: Option<[f32; 3]>,
    /// The mark shot last, for the scripts (GetVATSValue).
    shooting: Option<usize>,
    /// This stay's shots and AP spent.
    fired: u32,
    spent: f32,
    /// SOUN FormIDs by Editor ID, found on first use.
    sounds: Option<HashMap<String, FormId>>,
    pub stats: VatsStats,
}

impl VatsMode {
    /// Marking or firing.
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Marking | Phase::Firing)
    }

    /// The mode has Use, Block and Activate for itself (§11.5).
    pub fn holds_input(&self) -> bool {
        self.phase != Phase::Off
    }

    /// The actors marked now (references).
    pub fn marked(&self) -> Vec<u32> {
        self.queue.marks().iter().map(|m| m.target.0).collect()
    }

    /// The shot `crate::combat` is to fire now: the point it goes straight at.
    pub fn take_shot(&mut self) -> Option<[f32; 3]> {
        self.shot.take()
    }

    /// The AP the marks not yet fired hold.
    pub fn reserved(&self) -> f32 {
        if !self.active() {
            return 0.0;
        }
        self.queue.cost * self.queue.len().saturating_sub(self.fired as usize) as f32
    }

    /// The clock's speed the mode asks for (`script::time_multiplier` multiplies it in).
    pub fn time_scale(&self) -> f32 {
        match (self.phase, self.entered_at) {
            (Phase::Marking, Some(t)) => time_scale_at((self.now - t) as f32),
            (Phase::Firing, _) => FIRE_SCALE,
            _ => 1.0,
        }
    }

    /// The look for the image-space pass (§5): the swipe's edge (a share of the screen
    /// width; graded to its left), the band on the edge, the wash over the screen, the
    /// grade's strength.
    pub fn look(&self) -> [f32; 4] {
        let since = |t: Option<f64>| t.map(|t| (self.now - t) as f32);
        look_at(self.active(), since(self.entered_at), since(self.left_at))
    }

    /// The marks not yet fired, on the screen to draw.
    fn unfired(&self) -> impl Iterator<Item = &Spot> {
        let skip = if self.phase == Phase::Firing { self.fired as usize } else { 0 };
        self.spots.iter().skip(skip).take(if self.active() { usize::MAX } else { 0 })
    }

    fn sound(&mut self, sfx: Option<&Sfx>, edid: &str) -> Option<FormId> {
        let sfx = sfx?;
        let by = self.sounds.get_or_insert_with(|| sfx.forms.sounds.iter().filter_map(|(&id, s)| s.edid.clone().map(|e| (e, id))).collect());
        let found = by.get(edid).copied();
        if found.is_none() {
            debug!("vats mode: no sound {edid}");
        }
        found
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The clock's speed `since` real seconds after entering: from 1 down to [`MARK_SCALE`]
/// over the swipe.
pub fn time_scale_at(since: f32) -> f32 {
    1.0 - (1.0 - MARK_SCALE) * smoothstep(since / ENTER_SWIPE)
}

/// The look ([`VatsMode::look`]) while the mode is on (`on`), `entered` real seconds after
/// entering, or `left` seconds after it went off.
pub fn look_at(on: bool, entered: Option<f32>, left: Option<f32>) -> [f32; 4] {
    match (on, entered, left) {
        (true, Some(t), _) if t < ENTER_SWIPE => {
            // The edge crosses from the left edge until the band behind it has left too.
            let edge = t / ENTER_SWIPE * (1.0 + BAND_WIDTH);
            [edge, BAND_STRENGTH, 0.0, 1.0]
        }
        (true, Some(t), _) => [2.0, 0.0, WASH_START * (1.0 - (t - ENTER_SWIPE) / ENTER_WASH).max(0.0), 1.0],
        (true, None, _) => [2.0, 0.0, 0.0, 1.0],
        (false, _, Some(t)) if t < LEAVE_FADE => [2.0, 0.0, 0.0, 1.0 - t / LEAVE_FADE],
        _ => [0.0; 4],
    }
}

/// The band's width behind the edge, a share of the screen width (the shader's
/// `VATS_BAND`) [design].
pub const BAND_WIDTH: f32 = 0.22;

/// The player's Action Points: (current, maximum, damage); `None` without the script
/// runtime. The maximum counts the temporary modifiers, as the HUD's bar does.
fn player_ap(script: Option<&mut ScriptState>, sim: &mut SimState) -> Option<(f32, f32, f32)> {
    let rt = script.and_then(|s| s.rt.as_mut())?;
    with_world(rt, sim, |rt| {
        let [base, modifier, damage] = fnv_game::functions::actor_value_parts(&mut rt.game, FormId::PLAYER_REF, ACTION_POINTS).ok()?;
        let max = base + modifier + fnv_game::functions::actor_value_temporary(&rt.game, FormId::PLAYER_REF, ACTION_POINTS);
        Some(((max - damage) as f32, max as f32, damage as f32))
    })
}

/// The sound side of [`update`].
#[derive(SystemParam)]
struct Sounds<'w> {
    sfx: Option<Res<'w, Sfx>>,
    play: MessageWriter<'w, PlaySound>,
    loops: MessageWriter<'w, LoopSound>,
}

impl Sounds<'_> {
    fn play(&mut self, vats: &mut VatsMode, edid: &str) {
        if let Some(sound) = vats.sound(self.sfx.as_deref(), edid) {
            self.play.write(PlaySound { sound, at: None });
        }
    }

    fn looping(&mut self, vats: &mut VatsMode, on: bool) {
        if let Some(sound) = vats.sound(self.sfx.as_deref(), "UIVATSLoop") {
            self.loops.write(LoopSound { sound, on });
        }
    }
}

/// The keys the mode takes (after the frame's input, test aids and script locks), and
/// what it holds back from the rest of the game (§11.5): the trigger, aiming down the
/// sights, reloading and Ammo Swap while it is on. V with the weapon away draws it.
fn read_keys(input: ControlInput, state: Res<State<PlayState>>, locks: Res<ControlLocks>, mut vats: ResMut<VatsMode>, mut control: ResMut<PlayerControl>) {
    let playing = *state.get() == PlayState::Playing && !input.suppressed();
    let pressed = |c: Control| playing && input.just_pressed(c);
    vats.keys = Keys { toggle: pressed(Control::Vats), mark: pressed(Control::Use), unmark: pressed(Control::Block), accept: pressed(Control::Activate) };
    if vats.phase == Phase::Off && vats.keys.toggle && !control.weapon_out && !locks.fighting {
        control.weapon_out = true;
        vats.phase = Phase::Drawing;
        vats.keys.toggle = false;
        info!("vats mode: drawing the weapon first");
    }
    if vats.holds_input() {
        control.trigger = false;
        control.aiming = false;
        control.reload = false;
        control.ammo_swap = false;
    }
}

/// The mode's work for the frame; before `crate::combat::fire`, which fires the shot asked
/// for ([`VatsMode::take_shot`]).
#[allow(clippy::too_many_arguments)]
fn update(
    (real, time): (Res<Time<Real>>, Res<Time>),
    state: Res<State<PlayState>>,
    mut vats: ResMut<VatsMode>,
    (combat, anim, parts): (Option<Res<Combat>>, Res<WeaponAnim>, Res<ActorParts>),
    mut sim: ResMut<SimState>,
    mut script: Option<ResMut<ScriptState>>,
    mut phys: ResMut<PhysState>,
    mut player: ResMut<Player>,
    control: Res<PlayerControl>,
    locks: Res<ControlLocks>,
    mut hud: ResMut<HudMessage>,
    mut sounds: Sounds,
) {
    let v = &mut *vats;
    v.now = real.elapsed_secs_f64();
    v.shot = None;
    let keys = std::mem::take(&mut v.keys);
    let playing = *state.get() == PlayState::Playing;
    let dead = script.as_deref().and_then(|s| s.rt.as_ref()).is_some_and(|rt| rt.game.is_dead(FormId::PLAYER_REF));
    let Some(combat) = combat else { return };
    // What ends the mode whatever it is doing (§9.3, §11.6).
    let stop = if !playing {
        Some("a menu opened")
    } else if dead {
        Some("the player died")
    } else if locks.fighting {
        Some("a script disabled fighting")
    } else if v.active() && v.weapon.as_ref().map(|w| w.id) != combat.player_weapon().filter(|_| control.weapon_out) {
        Some("the weapon left the hand")
    } else {
        None
    };
    match (v.phase, stop) {
        (Phase::Off, _) => {
            regenerate(script.as_deref_mut(), &mut sim, combat.rules.ap_restore_rate, time.delta_secs());
            if keys.toggle {
                try_enter(v, &combat, script.as_deref_mut(), &mut sim, playing && !dead && !locks.fighting, &mut hud, &mut sounds);
            }
        }
        (_, Some(why)) => end(v, why, &mut sounds),
        (Phase::Drawing, None) => {
            if keys.toggle {
                v.phase = Phase::Off;
                info!("vats mode: entering cancelled");
            } else if anim.ready && control.weapon_out {
                v.phase = Phase::Off;
                try_enter(v, &combat, script.as_deref_mut(), &mut sim, true, &mut hud, &mut sounds);
            }
        }
        (Phase::Marking, None) => {
            follow(v, &phys);
            if keys.toggle {
                end(v, "V pressed again", &mut sounds);
            } else if keys.unmark {
                match v.queue.pop() {
                    Some(m) => {
                        let s = v.spots.pop();
                        info!("vats mode: unmarked {} ({}), {} marks left", m.target, s.map_or_else(String::new, |s| s.part), v.queue.len());
                        sounds.play(v, "UIVATSMove");
                    }
                    None => end(v, "right click with no marks", &mut sounds),
                }
            } else if keys.accept {
                if v.queue.is_empty() {
                    info!("vats mode: nothing marked to fire");
                } else {
                    v.phase = Phase::Firing;
                    v.volley = Some(Volley::new(v.queue.len(), SHOT_INTERVAL, AFTER_LAST));
                    sounds.looping(v, false);
                    sounds.play(v, "UIVATSReady");
                    info!("vats mode: firing {} marked shots", v.queue.len());
                }
            } else if keys.mark {
                mark(v, &combat, &parts, script.as_deref_mut(), &mut sim, &mut phys, &player, &control, &mut hud, &mut sounds);
            }
        }
        (Phase::Firing, None) => {
            follow(v, &phys);
            let event = v.volley.as_mut().and_then(|vo| vo.step(time.delta_secs()));
            match event {
                Some(VolleyEvent::Shot(i)) => shoot(v, i, script.as_deref_mut(), &mut sim, &mut player, &control),
                Some(VolleyEvent::Done) => end(v, "all shots fired", &mut sounds),
                None => {}
            }
        }
    }
    tell_scripts(v, script.as_deref_mut(), &player);
}

/// Spent AP comes back outside the mode (§7.2).
fn regenerate(script: Option<&mut ScriptState>, sim: &mut SimState, rate: f32, dt: f32) {
    let Some(rt) = script.and_then(|s| s.rt.as_mut()) else { return };
    if dt <= 0.0 {
        return;
    }
    with_world(rt, sim, |rt| {
        let g = &mut rt.game;
        let Ok([base, modifier, damage]) = fnv_game::functions::actor_value_parts(g, FormId::PLAYER_REF, ACTION_POINTS) else { return };
        if damage <= 0.0 {
            return;
        }
        let max = base + modifier + fnv_game::functions::actor_value_temporary(g, FormId::PLAYER_REF, ACTION_POINTS);
        let left = restored_damage(damage as f32, max as f32, rate, dt) as f64;
        fnv_game::functions::restore_actor_value_damage(g, FormId::PLAYER_REF, ACTION_POINTS, damage - left);
    });
}

/// V: start marking if the weapon and the AP allow (§3).
#[allow(clippy::too_many_arguments)]
fn try_enter(v: &mut VatsMode, combat: &Combat, script: Option<&mut ScriptState>, sim: &mut SimState, allowed: bool, hud: &mut HudMessage, sounds: &mut Sounds) {
    let ap = player_ap(script, sim);
    let refused = |v: &mut VatsMode, why: &str, hud: &mut HudMessage, sounds: &mut Sounds| {
        info!("vats mode: refused: {why}");
        hud.show(why.to_string());
        v.stats.refused += 1;
        sounds.play(v, "UIVATSEnterFail");
    };
    if !allowed {
        return refused(v, "Fighting is not possible now.", hud, sounds);
    }
    let w = match combat.vats_weapon() {
        Ok(w) => w,
        Err(why) => return refused(v, why, hud, sounds),
    };
    let ap_now = ap.map_or(f32::INFINITY, |a| a.0);
    v.queue = MarkQueue::new(w.cost, w.rounds_per_shot, w.rounds);
    if v.queue.check(ap_now) == Err(Refusal::ActionPoints) {
        return refused(v, Refusal::ActionPoints.message(), hud, sounds);
    }
    v.phase = Phase::Marking;
    v.spots.clear();
    v.volley = None;
    v.fired = 0;
    v.spent = 0.0;
    v.entered_at = Some(v.now);
    v.left_at = None;
    v.stats.entered += 1;
    info!(
        "vats mode: on ({}, {:.0} AP a shot, {} rounds loaded, {} AP; at most {} marks)",
        w.edid,
        w.cost,
        w.rounds,
        ap.map_or_else(|| "unlimited".to_string(), |a| format!("{:.0}", a.0)),
        v.queue.most(ap_now)
    );
    v.weapon = Some(InHand { id: w.id, reach: w.reach });
    sounds.play(v, "UIVATSEnter");
    sounds.looping(v, true);
}

/// Leave the mode (§2): marks dropped, reservations freed, the look fading out.
fn end(v: &mut VatsMode, why: &str, sounds: &mut Sounds) {
    let was = v.phase;
    v.phase = Phase::Off;
    v.volley = None;
    v.shot = None;
    v.shooting = None;
    if matches!(was, Phase::Marking | Phase::Firing) {
        if was == Phase::Marking {
            sounds.looping(v, false);
        }
        sounds.play(v, "UIVATSExit");
        v.left_at = Some(v.now);
        info!("vats mode: off ({} shots fired, {:.0} AP spent; {why})", v.fired, v.spent);
    } else if was == Phase::Drawing {
        info!("vats mode: entering cancelled ({why})");
    }
    v.entered_at = None;
    v.queue = MarkQueue::default();
    v.spots.clear();
}

/// The point to aim at on an actor: its part's capsule centre, the whole capsule's aim
/// point without parts.
fn point_of(phys: &PhysState, actor: u32, part: Option<u8>) -> Option<[f32; 3]> {
    match part {
        Some(p) => phys.world.actor_part_center(actor, p).or_else(|| phys.world.actor_aim_point(actor)),
        None => phys.world.actor_aim_point(actor),
    }
}

/// The marks follow their actors.
fn follow(v: &mut VatsMode, phys: &PhysState) {
    let marks: Vec<_> = v.queue.marks().to_vec();
    for (m, s) in marks.iter().zip(v.spots.iter_mut()) {
        if let Some(p) = point_of(phys, m.target.0, m.part) {
            s.point = p;
        }
    }
}

/// Left click: mark the actor (and body part) under the crosshair (§6, §7.2).
#[allow(clippy::too_many_arguments)]
fn mark(
    v: &mut VatsMode,
    combat: &Combat,
    parts: &ActorParts,
    script: Option<&mut ScriptState>,
    sim: &mut SimState,
    phys: &mut PhysState,
    player: &Player,
    control: &PlayerControl,
    hud: &mut HudMessage,
    sounds: &mut Sounds,
) {
    let (from, dir) = Crosshair::ray(control.crosshair.as_ref(), player.eye(), player.look_dir());
    let reach = v.weapon.as_ref().map_or(REACH, |w| w.reach);
    // Through what the character passes and the actors' walking capsules (their hit
    // volumes are their bodies), as the shots go.
    let mut filter = Filter { exclude_layers: layers::character_ignores(), ..Default::default() };
    filter.exclude.extend(phys.npcs.values().map(|b| b.ch.key()));
    let hit = phys.ray(from, dir, reach, filter);
    let Some((actor, part)) = hit.as_ref().and_then(|h| match h.owner {
        Owner::Actor { form_id, part, .. } => Some((form_id, (part != fnv_phys::actors::WHOLE).then_some(part))),
        _ => None,
    }) else {
        info!("vats mode: nothing to mark under the crosshair");
        return;
    };
    if crate::combat::is_dead(script.as_deref(), actor) {
        info!("vats mode: {actor:08X} is dead: not marked");
        return;
    }
    let ap_now = player_ap(script, sim).map_or(f32::INFINITY, |a| a.0);
    match v.queue.push(FormId(actor), part, ap_now) {
        Ok(k) => {
            let bp = combat.vats_part(parts, actor, part);
            let part_name = match (part, bp.as_ref().and_then(|b| b.name.clone())) {
                (Some(crate::hitvol::WEAPON_PART), _) => "its weapon".to_string(),
                (_, Some(n)) => n,
                (Some(p), None) => format!("part {p}"),
                (None, None) => "whole".to_string(),
            };
            let condition = bp.as_ref().and_then(|b| b.actor_value.as_deref()).and_then(fnv_game::combat::limb_condition);
            let name = sim.base_of(actor).and_then(|b| sim.name(b).map(str::to_string)).unwrap_or_else(|| "?".into());
            let point = point_of(phys, actor, part).or(hit.map(|h| h.point)).unwrap_or(from);
            v.stats.marks += 1;
            info!(
                "vats mode: marked {name} {actor:08X} ({part_name}), mark {k}; {:.0} AP reserved, {} AP left",
                v.queue.reserved(),
                if ap_now.is_finite() { format!("{:.0}", ap_now - v.queue.reserved()) } else { "unlimited".into() }
            );
            v.spots.push(Spot { point, name, part: part_name, condition });
            sounds.play(v, if part.is_some() { "UIVATSSelectTargetPart" } else { "UIVATSSelectTarget" });
        }
        Err(r) => {
            v.stats.refused += 1;
            info!("vats mode: mark refused: {} ({} marks, {:.0} AP reserved)", r.message(), v.queue.len(), v.queue.reserved());
            hud.show(r.message().to_string());
            sounds.play(v, "UIVATSInsufficientAP");
        }
    }
}

/// Fire mark `i` (§9.1): the view snaps to it, `crate::combat` fires at it this frame, its
/// AP is spent.
fn shoot(v: &mut VatsMode, i: usize, script: Option<&mut ScriptState>, sim: &mut SimState, player: &mut Player, control: &PlayerControl) {
    let (Some(m), Some(s)) = (v.queue.marks().get(i).copied(), v.spots.get(i)) else { return };
    let eye = player.eye();
    (player.heading, player.pitch) = Crosshair::aim_at(control.crosshair.as_ref(), eye, s.point);
    v.shot = Some(s.point);
    v.shooting = Some(i);
    v.fired += 1;
    v.stats.shots += 1;
    if let Some(rt) = script.and_then(|sc| sc.rt.as_mut()) {
        with_world(rt, sim, |rt| fnv_game::functions::add_actor_value_damage(&mut rt.game, FormId::PLAYER_REF, ACTION_POINTS, m.cost as f64));
        v.spent += m.cost;
        v.stats.ap_spent += m.cost;
    }
    info!("vats mode: shot {} of {} at {} {} ({}), {:.0} AP spent", i + 1, v.queue.len(), s.name, m.target, s.part, m.cost);
}

/// GetVATSMode and GetVATSValue's view of the mode (§12).
fn tell_scripts(v: &VatsMode, script: Option<&mut ScriptState>, player: &Player) {
    let Some(rt) = script.and_then(|s| s.rt.as_mut()) else { return };
    use fnv_game::vats::{VatsShot, MODE_OFF, MODE_PLAYBACK, MODE_TARGET_SELECT};
    let mode = match v.phase {
        Phase::Marking => MODE_TARGET_SELECT,
        Phase::Firing => MODE_PLAYBACK,
        _ => MODE_OFF,
    };
    let shot = v.shooting.filter(|_| mode == MODE_PLAYBACK).and_then(|i| {
        let (m, s) = (v.queue.marks().get(i)?, v.spots.get(i)?);
        let eye = player.eye();
        let d = ((s.point[0] - eye[0]).powi(2) + (s.point[1] - eye[1]).powi(2) + (s.point[2] - eye[2]).powi(2)).sqrt();
        Some(VatsShot { weapon: v.weapon.as_ref()?.id, target: m.target, distance: d as f64, condition: s.condition })
    });
    if rt.game.vats.mode != mode || rt.game.vats.shot != shot {
        rt.game.set_vats(mode, shot);
    }
}

/// A mark's "x" on the screen: two crossed bars.
#[derive(Component)]
struct MarkCross(usize);

fn spawn_marks(mut commands: Commands) {
    for i in 0..MAX_DRAWN {
        commands
            .spawn((Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, GlobalZIndex(5), MarkCross(i)))
            .with_children(|p| {
                for deg in [45.0f32, -45.0] {
                    p.spawn((
                        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(22.0), top: Val::Percent(39.0), ..default() },
                        BackgroundColor(MARK_COLOUR),
                        UiTransform::from_rotation(Rot2::degrees(deg)),
                    ));
                }
            });
    }
}

/// Put each mark's "x" where its point is on the screen (§8).
fn draw_marks(
    vats: Res<VatsMode>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut crosses: Query<(&MarkCross, &mut Node, &mut Visibility)>,
) {
    let width = windows.iter().next().map_or(1280.0, |w| w.width());
    let size = (MARK_SIZE * width).max(6.0);
    let camera = cameras.single().ok();
    let spots: Vec<[f32; 3]> = vats.unfired().map(|s| s.point).collect();
    for (c, mut node, mut vis) in &mut crosses {
        let at = spots.get(c.0).zip(camera).and_then(|(p, (cam, t))| cam.world_to_viewport(t, space::to_bevy(*p)).ok());
        let want = if at.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if let Some(at) = at {
            node.left = Val::Px(at.x - size / 2.0);
            node.top = Val::Px(at.y - size / 2.0);
            node.width = Val::Px(size);
            node.height = Val::Px(size);
        }
    }
}

fn report(mut exit: MessageReader<AppExit>, vats: Res<VatsMode>) {
    if exit.read().next().is_none() {
        return;
    }
    let s = vats.stats;
    if s.entered == 0 && s.refused == 0 {
        return;
    }
    info!("vats mode summary: entered {} times, {} marks, {} shots, {:.0} AP spent, {} refused", s.entered, s.marks, s.shots, s.ap_spent, s.refused);
}

pub struct VatsModePlugin;

impl Plugin for VatsModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VatsMode>()
            .add_systems(Startup, spawn_marks)
            .add_systems(Update, read_keys.after(PlaySet::Input).after(crate::control::enforce_locks).before(PlaySet::Physics))
            .add_systems(Update, update.in_set(PlaySet::Activation).before(crate::combat::fire))
            .add_systems(Update, draw_marks.after(PlaySet::Activation))
            .add_systems(Last, report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_eases_down_over_the_swipe() {
        assert_eq!(time_scale_at(0.0), 1.0);
        let mid = time_scale_at(ENTER_SWIPE / 2.0);
        assert!(mid < 1.0 && mid > MARK_SCALE, "{mid}");
        assert!((time_scale_at(ENTER_SWIPE) - MARK_SCALE).abs() < 1e-6);
        assert!((time_scale_at(10.0) - MARK_SCALE).abs() < 1e-6);
    }

    #[test]
    fn the_swipe_crosses_then_the_wash_fades_and_the_grade_stays() {
        let start = look_at(true, Some(0.0), None);
        assert_eq!(start, [0.0, BAND_STRENGTH, 0.0, 1.0]);
        let half = look_at(true, Some(ENTER_SWIPE / 2.0), None);
        assert!(half[0] > 0.5 && half[0] < 0.7, "{half:?}");
        let after = look_at(true, Some(ENTER_SWIPE), None);
        assert_eq!(after, [2.0, 0.0, WASH_START, 1.0]);
        assert_eq!(look_at(true, Some(ENTER_SWIPE + ENTER_WASH + 1.0), None), [2.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn leaving_fades_the_grade_out() {
        let l = look_at(false, None, Some(LEAVE_FADE / 2.0));
        assert!((l[3] - 0.5).abs() < 1e-6, "{l:?}");
        assert_eq!(look_at(false, None, Some(LEAVE_FADE)), [0.0; 4]);
        assert_eq!(look_at(false, None, None), [0.0; 4]);
    }

    #[test]
    fn marking_nearly_stops_the_clock_and_firing_halves_it() {
        let mut v = VatsMode { phase: Phase::Marking, entered_at: Some(0.0), now: 5.0, ..Default::default() };
        assert!((v.time_scale() - MARK_SCALE).abs() < 1e-6);
        v.phase = Phase::Firing;
        assert_eq!(v.time_scale(), FIRE_SCALE);
        assert_eq!(FIRE_SCALE, 0.5, "owner 2026-10-18: half speed");
        // The shots come SHOT_INTERVAL / FIRE_SCALE apart in real time.
        assert!((SHOT_INTERVAL / FIRE_SCALE - 0.4).abs() < 1e-6);
        v.phase = Phase::Drawing;
        assert_eq!(v.time_scale(), 1.0);
        v.phase = Phase::Off;
        assert_eq!(v.time_scale(), 1.0);
    }

    #[test]
    fn the_reservation_shrinks_as_shots_are_fired() {
        let mut v = VatsMode { phase: Phase::Marking, queue: MarkQueue::new(26.0, 1, 6), ..Default::default() };
        v.queue.push(FormId(1), None, 92.0).unwrap();
        v.queue.push(FormId(2), Some(1), 92.0).unwrap();
        assert_eq!(v.reserved(), 52.0);
        v.phase = Phase::Firing;
        v.fired = 1;
        assert_eq!(v.reserved(), 26.0);
        v.phase = Phase::Off;
        assert_eq!(v.reserved(), 0.0);
    }
}
