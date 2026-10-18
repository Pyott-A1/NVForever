//! The player's controls beyond walking and looking, on the game's bindings
//! ([`crate::bindings`]; docs/design/player-body.md §1.2):
//!
//! - Crouch/Sneak (Left Ctrl) toggles sneaking.
//! - Ready Item (R): with the weapon away, draws it; with it out, a tap reloads and a hold
//!   of [`HOLSTER_HOLD`] puts it away [hypothesis: the game's tap/hold split]. Reloading
//!   is `crate::combat`'s.
//! - Use (left mouse) attacks while held ([`PlayerControl::trigger`], `crate::combat`);
//!   with the weapon away it draws it first [hypothesis].
//! - Ammo Swap (hotkey 2) asks `crate::combat` for the next carried ammo type.
//! - Block (right mouse, Left Alt), held: guns aim down the sights, fists and melee
//!   weapons block.
//! - Always Run (Caps Lock) toggles running as the default (on by default:
//!   `bAlwaysRunByDefault=1`); Run (Left Shift) held does the other; Auto Move (Q) walks
//!   forward until Forward or Back is pressed (player.rs).
//! - Toggle POV (F, middle mouse): a tap switches first and third person; held while the
//!   mouse moves, it circles the camera around the player without turning them
//!   ([`Orbit`]; from first person the view goes to third person meanwhile), and when it
//!   is let go the camera swings back behind, or from first person goes straight back to
//!   the eye [hypothesis: the game's hold-POV camera; design: the owner's playtest list 5
//!   item 23].
//!   The mouse wheel zooms the third-person camera.
//! - VATS (V) is VATS mode's (`crate::vats_mode`, docs/design/vats-mode.md), which also
//!   takes Use, Block and Activate while it is on.
//! - The game's other controls (Rest, the hotkeys, Ammo Swap, Quick Save, Quick Load,
//!   Grab) are bound but not built: pressing one says so on screen.
//! - Scripts turn groups of controls off and on (`DisablePlayerControls`,
//!   `EnablePlayerControls`): [`ControlLocks`], applied to every frame's requests by
//!   [`enforce_locks`].
//!
//! This module holds the requests; what they look like is the player body's job
//! (nvforever `player_body.rs`), which also writes the eye it sees from into
//! [`crate::player::Player::body_eye`]. The camera ([`place_camera`]) sits at that eye in
//! first person, and behind the shoulder in third person, pulled in where collision is in
//! the way.

use crate::activate::HudMessage;
use crate::bindings::{Bindings, Control, ControlInput};
use crate::phys::PhysState;
use crate::player::{MoveIntent, Player};
use crate::sim::SimState;
use crate::{space, PlayState, PlayerCamera};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use fnv_phys::Filter;

/// First- or third-person view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pov {
    #[default]
    First,
    Third,
}

/// The camera circling the player while Toggle POV is held: angles added to the view's
/// heading and pitch for the camera only.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    /// Toggle POV is down.
    pub held: bool,
    /// The mouse moved while it was down (so letting go does not switch the view).
    pub moved: bool,
}

impl Orbit {
    /// Whether the camera is off its place behind the player (circling, or swinging back).
    pub fn active(&self) -> bool {
        (self.held && self.moved) || self.yaw.abs() + self.pitch.abs() > 1e-3
    }

    /// Toggle POV let go after circling, the view being `pov`: from first person the view
    /// goes straight back to the eye; in third person the camera swings back behind the
    /// player (the owner's playtest list 5 item 23: back to first person when the player
    /// was in it) [design: the original's hold-POV camera is not captured].
    pub fn released(self, pov: Pov) -> Orbit {
        match pov {
            Pov::First => Orbit::default(),
            Pov::Third => Orbit { held: false, ..self },
        }
    }
}

/// The camera in front of the player looking back at them, while the character creator
/// is open (`crate::creator`) [design, docs/spec-notes/character-creator.md §5].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Portrait {
    /// Turned about the player from straight in front of them, radians.
    pub yaw: f32,
    /// From the point looked at, game units.
    pub distance: f32,
    /// The point looked at, this far under the eye, game units.
    pub drop: f32,
    /// Looking down (positive) or up, radians.
    pub pitch: f32,
    /// The camera stepped this far to its left, game units, so the player stands right
    /// of the picture's middle (clear of the creator's panel).
    pub shift: f32,
}

impl Portrait {
    /// Where the camera stands and the heading and pitch it looks with, for a player
    /// whose eye is at `eye` and who faces `heading`.
    pub fn spot(&self, eye: [f32; 3], heading: f32) -> ([f32; 3], f32, f32) {
        let target = [eye[0], eye[1], eye[2] - self.drop];
        let out = space::look_dir(heading + self.yaw, self.pitch);
        let look = heading + self.yaw + std::f32::consts::PI;
        // The camera's right, level: (cos h, −sin h) for the heading it looks with.
        let (s, c) = look.sin_cos();
        let at = [target[0] + out[0] * self.distance - c * self.shift, target[1] + out[1] * self.distance + s * self.shift, target[2] + out[2] * self.distance];
        (at, look, -self.pitch)
    }
}

/// What the player asks of their character besides moving.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct PlayerControl {
    /// Crouched (sneaking).
    pub sneak: bool,
    /// Wants the weapon out (fists without one).
    pub weapon_out: bool,
    /// Block held: aiming down the sights, or blocking.
    pub aiming: bool,
    /// Running is the default (Always Run).
    pub always_run: bool,
    /// Walking forward on its own (Auto Move).
    pub auto_move: bool,
    pub pov: Pov,
    /// Third-person camera distance behind the pivot, game units.
    pub distance: f32,
    pub orbit: Orbit,
    /// How long Ready Item has been held with the weapon out (a hold holsters).
    pub ready_held: Option<f32>,
    /// Use held with the weapon out (the trigger; `crate::combat` reads it).
    pub trigger: bool,
    /// A reload was asked for (Ready Item tapped with the weapon out); `crate::combat`
    /// clears it.
    pub reload: bool,
    /// Ammo Swap pressed; `crate::combat` clears it.
    pub ammo_swap: bool,
    /// The third-person camera's line through the crosshair, set by [`place_camera`]
    /// each frame; `None` in first person and while the camera circles. Activation and
    /// the player's shots aim along it ([`Crosshair::ray`]).
    pub crosshair: Option<Crosshair>,
    /// The character creator's camera, while it is open.
    pub portrait: Option<Portrait>,
}

impl Default for PlayerControl {
    fn default() -> Self {
        PlayerControl {
            sneak: false,
            weapon_out: false,
            aiming: false,
            always_run: crate::bindings::ALWAYS_RUN_BY_DEFAULT,
            auto_move: false,
            pov: Pov::First,
            distance: DEFAULT_DISTANCE,
            orbit: Orbit::default(),
            ready_held: None,
            trigger: false,
            reload: false,
            ammo_swap: false,
            crosshair: None,
            portrait: None,
        }
    }
}

impl PlayerControl {
    /// Whether the camera shows the player from outside (third person, or circling).
    pub fn third_person_view(&self) -> bool {
        self.pov == Pov::Third || self.orbit.active() || self.portrait.is_some()
    }
}

/// The third-person camera's view line: the crosshair sits in the middle of the view, so
/// what is under it lies along this line, not along the eye's (the camera is off to the
/// side, over the shoulder: playtest list 4, items 11 and 12).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crosshair {
    /// The camera, game space.
    pub camera: [f32; 3],
    /// The view direction (unit), game space.
    pub dir: [f32; 3],
    /// The pivot the camera was placed from, and where the camera's line passes it:
    /// right of it and up, level with it along the view (the shoulder offset, less what
    /// collision pulled in).
    pub pivot: [f32; 3],
    pub shoulder: [f32; 2],
}

impl Crosshair {
    /// The line of a camera placed at `camera`, looking along `heading` and `pitch`.
    pub fn placed(camera: [f32; 3], heading: f32, pitch: f32, pivot: [f32; 3]) -> Crosshair {
        let dir = space::look_dir(heading, pitch);
        let (s, c) = heading.sin_cos();
        let rel = [camera[0] - pivot[0], camera[1] - pivot[1], camera[2] - pivot[2]];
        let side = rel[0] * c - rel[1] * s;
        // Along the view to where the line is level with the pivot horizontally.
        let ahead = rel[0] * s + rel[1] * c;
        let level = dir[0] * s + dir[1] * c;
        let up = if level > 1e-3 { rel[2] - dir[2] * ahead / level } else { rel[2] };
        Crosshair { camera, dir, pivot, shoulder: [side, up] }
    }

    /// The ray under the crosshair, `(start, dir)`: from the point on the camera's line
    /// level with the eye (so nothing between the camera and the player is picked, and
    /// reaches stay measured from about the player), along the view.
    pub fn ray(this: Option<&Crosshair>, eye: [f32; 3], look: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let Some(c) = this else { return (eye, look) };
        let rel = [eye[0] - c.camera[0], eye[1] - c.camera[1], eye[2] - c.camera[2]];
        let t = (rel[0] * c.dir[0] + rel[1] * c.dir[1] + rel[2] * c.dir[2]).max(0.0);
        ([c.camera[0] + c.dir[0] * t, c.camera[1] + c.dir[1] * t, c.camera[2] + c.dir[2] * t], c.dir)
    }

    /// The direction from `eye` to the point under the crosshair, given where the
    /// crosshair ray first hits (`hit`, `None`: nothing within `reach`): what the
    /// player's shot follows, so it lands under the crosshair as in first person. The view
    /// direction itself when that point is not ahead of the eye.
    pub fn aim_from(eye: [f32; 3], start: [f32; 3], dir: [f32; 3], reach: f32, hit: Option<[f32; 3]>) -> [f32; 3] {
        let p = hit.unwrap_or([start[0] + dir[0] * reach, start[1] + dir[1] * reach, start[2] + dir[2] * reach]);
        let to = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        // Under a hand's breadth ahead, or behind: keep the view's direction.
        if len < 1.0 || (to[0] * dir[0] + to[1] * dir[1] + to[2] * dir[2]) < 0.5 * len {
            return dir;
        }
        [to[0] / len, to[1] / len, to[2] / len]
    }

    /// The heading and pitch that put `point` under the crosshair: from `eye` in first
    /// person (`None`), else turned so the camera's line, offset by the shoulder from the
    /// pivot, passes through it (collision pulling the camera in is left out).
    pub fn aim_at(this: Option<&Crosshair>, eye: [f32; 3], point: [f32; 3]) -> (f32, f32) {
        let (from, side, up) = match this {
            Some(c) => (c.pivot, c.shoulder[0], c.shoulder[1]),
            None => (eye, 0.0, 0.0),
        };
        let (dx, dy, dz) = (point[0] - from[0], point[1] - from[1], point[2] - from[2]);
        let r = dx.hypot(dy).max(1e-3);
        let side = side.clamp(-0.99 * r, 0.99 * r);
        let heading = dx.atan2(dy) - (side / r).asin();
        let along = (r * r - side * side).sqrt();
        (heading, (dz - up).atan2(along))
    }
}

/// The control groups scripts have turned off with `DisablePlayerControls` and not yet
/// back on with `EnablePlayerControls` (docs/spec-notes/controls.md §5): the script
/// runtime's seven flags, copied here after each frame's scripts (`script::sync_locks`).
/// What each stops [documented: GECK wiki DisablePlayerControls, "Flag Definitions"]:
///
/// - `movement`: moving (and jumping, Auto Move [hypothesis]), activating world objects,
///   and the crosshair (`activate.rs`) and health (`combat.rs`) on the HUD.
/// - `pipboy`: opening the Pip-Boy (`menu.rs`).
/// - `fighting`: bringing up a weapon; a weapon that is out is put away.
/// - `pov`: third person; a third-person view goes back to first person.
/// - `looking`: turning the view (`player::mouse_look`).
/// - `rollover`: the prompt on world objects (`activate.rs`).
/// - `sneaking`: sneaking; a sneaking player stands up.
///
/// The keys are still read; [`enforce_locks`] strikes what is locked from the frame's
/// requests, so test aids that press controls are held like the keys.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControlLocks {
    pub movement: bool,
    pub pipboy: bool,
    pub fighting: bool,
    pub pov: bool,
    pub looking: bool,
    pub rollover: bool,
    pub sneaking: bool,
}

/// The groups' places in the functions' parameter lists (and in [`LockStats::held`]).
pub mod group {
    pub const MOVEMENT: usize = 0;
    pub const PIPBOY: usize = 1;
    pub const FIGHTING: usize = 2;
    pub const POV: usize = 3;
    pub const LOOKING: usize = 4;
    pub const ROLLOVER: usize = 5;
    pub const SNEAKING: usize = 6;
    pub const NAMES: [&str; 7] = ["movement", "Pip-Boy", "fighting", "POV", "looking", "rollover text", "sneaking"];
}

impl ControlLocks {
    /// Every group locked.
    pub const ALL: ControlLocks = ControlLocks { movement: true, pipboy: true, fighting: true, pov: true, looking: true, rollover: true, sneaking: true };

    /// From the runtime's flags, in the functions' parameter order: movement, Pip-Boy,
    /// fighting, POV, looking, rollover text, sneaking.
    pub fn from_flags(f: [bool; 7]) -> ControlLocks {
        let [movement, pipboy, fighting, pov, looking, rollover, sneaking] = f;
        ControlLocks { movement, pipboy, fighting, pov, looking, rollover, sneaking }
    }

    pub fn flags(&self) -> [bool; 7] {
        [self.movement, self.pipboy, self.fighting, self.pov, self.looking, self.rollover, self.sneaking]
    }

    pub fn any(&self) -> bool {
        *self != ControlLocks::default()
    }

    /// The locked groups by name, in parameter order.
    pub fn names(&self) -> Vec<&'static str> {
        self.flags().into_iter().zip(group::NAMES).filter(|(on, _)| *on).map(|(_, n)| n).collect()
    }

    /// Strike from the frame's requests what is locked; returns the groups (by
    /// [`group`] index) that had something to strike. Nothing comes back by itself when
    /// a group is enabled again: the weapon stays away, the view in first person and the
    /// player standing [hypothesis: not captured].
    pub fn apply(&self, intent: &mut MoveIntent, control: &mut PlayerControl) -> [bool; 7] {
        let mut struck = [false; 7];
        if self.movement {
            let before = (*intent, *control);
            (intent.forward, intent.right, intent.jump) = (0.0, 0.0, false);
            control.auto_move = false;
            struck[group::MOVEMENT] = (*intent, *control) != before;
        }
        if self.fighting {
            let before = *control;
            // Blocking and aiming go with the weapon [hypothesis].
            control.weapon_out = false;
            control.ready_held = None;
            (control.trigger, control.aiming, control.reload, control.ammo_swap) = (false, false, false, false);
            struck[group::FIGHTING] = *control != before;
        }
        if self.pov {
            let before = *control;
            // The circling camera is a view from outside too.
            control.pov = Pov::First;
            control.orbit = Orbit::default();
            struck[group::POV] = *control != before;
        }
        if self.sneaking {
            struck[group::SNEAKING] = control.sneak || intent.sneak;
            control.sneak = false;
            intent.sneak = false;
        }
        struck
    }
}

/// What the locks did, for the log line at exit.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LockStats {
    /// Times scripts changed which groups are off.
    pub changes: u32,
    /// Per group ([`group`] index): frames in which something the player asked for was
    /// struck (movement), or times a request was turned down (the others).
    pub held: [u32; 7],
}

impl LockStats {
    /// "movement 120, looking 2": the groups that held something back.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self.held.iter().zip(group::NAMES).filter(|(n, _)| **n > 0).map(|(n, name)| format!("{name} {n}")).collect();
        if parts.is_empty() {
            "nothing".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// After the frame's input (keys, mouse, test aids), before anything acts on it: what
/// scripts have locked is struck from it ([`ControlLocks::apply`]). A locked movement
/// control does not get the player up from furniture either [hypothesis].
pub fn enforce_locks(
    locks: Res<ControlLocks>,
    mut stats: ResMut<LockStats>,
    mut intent: ResMut<MoveIntent>,
    mut control: ResMut<PlayerControl>,
    mut seat: ResMut<crate::furniture::PlayerSeat>,
) {
    if !locks.any() {
        return;
    }
    let (mut i, mut c) = (*intent, *control);
    let mut struck = locks.apply(&mut i, &mut c);
    intent.set_if_neq(i);
    control.set_if_neq(c);
    if locks.movement && seat.leave {
        seat.leave = false;
        struck[group::MOVEMENT] = true;
    }
    if struck != [false; 7] {
        for (n, s) in stats.held.iter_mut().zip(struck) {
            *n += u32::from(s);
        }
    }
}

/// At exit, when scripts ever changed the player's controls: how often, what is off now
/// and what was held back.
/// Tell the scripts the camera's view (IsPC1stPerson) and whether the player sneaks
/// (IsSneaking) every frame; movement is told by detection (`nvforever` `detection.rs`).
pub fn tell_scripts(control: Res<PlayerControl>, mut script: Option<ResMut<crate::script::ScriptState>>) {
    let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) else { return };
    let g = &mut rt.game;
    g.set_first_person(!control.third_person_view());
    let mut m = g.motion(fnv_sim::FormId::PLAYER_REF);
    if m.sneaking != control.sneak {
        m.sneaking = control.sneak;
        g.set_motion(fnv_sim::FormId::PLAYER_REF, m);
    }
}

pub fn report_locks(mut exit: MessageReader<AppExit>, locks: Res<ControlLocks>, stats: Res<LockStats>) {
    if exit.read().next().is_none() || stats.changes == 0 {
        return;
    }
    let off = locks.names();
    info!(
        "controls summary: scripts changed the player's controls {} times; disabled at exit: {}; held back: {}",
        stats.changes,
        if off.is_empty() { "none".to_string() } else { off.join(", ") },
        stats.summary()
    );
}

/// Holding Ready Item this long with the weapon out holsters it, seconds [hypothesis].
pub const HOLSTER_HOLD: f32 = 0.5;
/// The circling camera swings back behind the player with this time constant, seconds
/// [engineering choice].
pub const ORBIT_RETURN: f32 = 0.15;
/// Third-person distance range and default, game units [engineering choice].
pub const MIN_DISTANCE: f32 = 60.0;
pub const MAX_DISTANCE: f32 = 450.0;
pub const DEFAULT_DISTANCE: f32 = 170.0;
/// Third-person camera offset from the pivot, to the right and up (game units):
/// FalloutNV.esm's `fOverShoulderPosX` (40) and `fOverShoulderPosZ` (−10) [observed:
/// GMST values; hypothesis: their meaning, from the names], used when the plugins lack
/// them.
pub const DEFAULT_SHOULDER: [f32; 2] = [40.0, -10.0];
/// How close the third-person camera may come to what is behind it, game units
/// [engineering choice].
const WALL_MARGIN: f32 = 8.0;

/// Speed while sneaking as a fraction of walking or running: `fMoveSneakMult`, 0.57 in
/// FalloutNV.esm (used when the plugins lack it) [observed: the player sneaking forward
/// at a run moved 197.7 units/s = 344 × 0.575, physics.md §6.3; hypothesis: the same
/// factor walking, backwards and sideways].
pub const DEFAULT_SNEAK_MULT: f32 = 0.57;

/// Game settings the controls use.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ControlSettings {
    pub sneak_mult: f32,
    /// Right and up offset of the third-person camera.
    pub shoulder: [f32; 2],
}

impl ControlSettings {
    pub fn from_settings(sneak: Option<f32>, x: Option<f32>, z: Option<f32>) -> ControlSettings {
        let ok = |v: Option<f32>| v.filter(|v| v.is_finite());
        ControlSettings {
            sneak_mult: ok(sneak).filter(|v| *v > 0.0).unwrap_or(DEFAULT_SNEAK_MULT),
            shoulder: [ok(x).unwrap_or(DEFAULT_SHOULDER[0]), ok(z).unwrap_or(DEFAULT_SHOULDER[1])],
        }
    }
}

/// The body that shows the player, written by the body each frame (nvforever), read by
/// the camera.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct BodyView {
    /// Third-person pivot (game space): above the body, following crouching.
    pub pivot: Option<[f32; 3]>,
}

impl BodyView {
    /// The drawn body's eye (the pivot is 5 above it, at the feet), for the character
    /// creator's camera.
    pub fn portrait_eye(&self) -> Option<[f32; 3]> {
        self.pivot.map(|p| [p[0], p[1], p[2] - 5.0])
    }
}

pub fn init_settings(mut commands: Commands, sim: Res<SimState>) {
    let f = |n: &str| sim.forms.setting_f32(n);
    let s = ControlSettings::from_settings(f("fMoveSneakMult"), f("fOverShoulderPosX"), f("fOverShoulderPosZ"));
    info!("player controls: the game's bindings; sneak speed × {:.2}, third-person shoulder offset {:?}", s.sneak_mult, s.shoulder);
    commands.insert_resource(s);
}

/// The controls that are bound but not built yet.
const NOT_BUILT: [Control; 10] = [
    Control::Hotkey1,
    Control::Hotkey3,
    Control::Hotkey4,
    Control::Hotkey5,
    Control::Hotkey6,
    Control::Hotkey7,
    Control::Hotkey8,
    Control::QuickSave,
    Control::QuickLoad,
    Control::Grab,
];

/// Toggles, holds, the third-person zoom; see the module notes.
pub fn read_controls(
    input: ControlInput,
    time: Res<Time>,
    scroll: Res<AccumulatedMouseScroll>,
    state: Res<State<PlayState>>,
    mut control: ResMut<PlayerControl>,
    hud: Option<ResMut<HudMessage>>,
) {
    if *state.get() != PlayState::Playing {
        control.aiming = false;
        control.trigger = false;
        control.ready_held = None;
        control.orbit.held = false;
        return;
    }
    let mut message: Option<String> = None;
    let c = &mut *control;
    if input.just_pressed(Control::CrouchSneak) {
        c.sneak = !c.sneak;
        info!("Crouch/Sneak pressed: sneaking {} asked", if c.sneak { "on" } else { "off" });
    }
    if input.just_pressed(Control::AlwaysRun) {
        c.always_run = !c.always_run;
    }
    if input.just_pressed(Control::AutoMove) {
        c.auto_move = !c.auto_move;
    }
    // Ready Item: draw at once; with the weapon out, tap to reload, hold to holster.
    if input.just_pressed(Control::ReadyItem) {
        if c.weapon_out {
            c.ready_held = Some(0.0);
        } else {
            c.weapon_out = true;
        }
    }
    if let Some(t) = c.ready_held.as_mut() {
        *t += time.delta_secs();
        if *t >= HOLSTER_HOLD {
            c.weapon_out = false;
            c.ready_held = None;
        } else if input.just_released(Control::ReadyItem) || !input.pressed(Control::ReadyItem) {
            c.ready_held = None;
            c.reload = true;
        }
    }
    // Use draws the weapon when it is away; held with it out, it is the trigger. The
    // press that draws does not fire.
    if input.just_pressed(Control::Use) && !c.weapon_out {
        c.weapon_out = true;
        c.trigger = false;
    } else {
        c.trigger = c.weapon_out && input.pressed(Control::Use) && (c.trigger || input.just_pressed(Control::Use));
    }
    if input.just_pressed(Control::AmmoSwap) {
        c.ammo_swap = true;
    }
    for ctl in NOT_BUILT {
        if input.just_pressed(ctl) {
            message = Some(format!("{}: not available yet", Bindings::name(ctl)));
        }
    }
    c.aiming = input.pressed(Control::Block);
    // Toggle POV: a tap switches the view; held with the mouse moving, the camera circles
    // (player::mouse_look turns `orbit` instead of the view).
    if input.just_pressed(Control::TogglePov) {
        c.orbit.held = true;
        c.orbit.moved = false;
    }
    if c.orbit.held && input.just_released(Control::TogglePov) {
        c.orbit.held = false;
        if !c.orbit.moved {
            c.pov = match c.pov {
                Pov::First => Pov::Third,
                Pov::Third => Pov::First,
            };
        } else {
            c.orbit = c.orbit.released(c.pov);
        }
    }
    if c.third_person_view() && scroll.delta.y != 0.0 && !input.suppressed() {
        c.distance = zoom(c.distance, scroll.delta.y);
    }
    if let Some(m) = message {
        info!("{m}");
        if let Some(mut h) = hud {
            h.show(m);
        }
    }
}

/// Third-person distance after a wheel motion (+ towards the player).
pub fn zoom(distance: f32, wheel: f32) -> f32 {
    (distance * 0.9f32.powf(wheel)).clamp(MIN_DISTANCE, MAX_DISTANCE)
}

/// Where the third-person camera wants to be: from the pivot, offset right and up, then
/// back along the view by `distance` (game space).
pub fn third_person_spot(pivot: [f32; 3], heading: f32, pitch: f32, shoulder: [f32; 2], distance: f32) -> ([f32; 3], [f32; 3]) {
    let (s, c) = heading.sin_cos();
    let right = [c, -s, 0.0];
    let base = [pivot[0] + right[0] * shoulder[0], pivot[1] + right[1] * shoulder[0], pivot[2] + shoulder[1]];
    let d = space::look_dir(heading, pitch);
    (base, [base[0] - d[0] * distance, base[1] - d[1] * distance, base[2] - d[2] * distance])
}

/// The shoulder offset while circling: it fades to the centre as the camera turns away
/// from behind, so the camera keeps looking at the player [engineering choice].
pub fn orbit_shoulder(shoulder: [f32; 2], yaw: f32) -> [f32; 2] {
    let k = 1.0 - (yaw.abs() / 0.5).min(1.0);
    [shoulder[0] * k, shoulder[1]]
}

/// The camera: at the eye in first person; behind the shoulder in third person (turned by
/// the orbit while circling), in front of whatever collision lies between the pivot and
/// that spot.
#[allow(clippy::too_many_arguments)]
pub fn place_camera(
    time: Res<Time>,
    player: Option<Res<Player>>,
    mut control: ResMut<PlayerControl>,
    settings: Option<Res<ControlSettings>>,
    body: Res<BodyView>,
    mut phys: ResMut<PhysState>,
    mut cams: Query<&mut Transform, With<PlayerCamera>>,
    mut told: Local<Option<bool>>,
) {
    // Say which line the crosshair follows when it changes (smoke tests read it).
    let third = control.crosshair.is_some();
    if *told != Some(third) {
        *told = Some(third);
        match control.crosshair {
            Some(c) => info!("crosshair: the third-person camera's line (shoulder {:?})", c.shoulder),
            None => info!("crosshair: the eye's view"),
        }
    }
    if !control.orbit.held {
        // Swing back behind the player.
        let k = (-time.delta_secs() / ORBIT_RETURN).exp();
        let o = &mut control.orbit;
        (o.yaw, o.pitch) = (o.yaw * k, o.pitch * k);
        if o.yaw.abs() + o.pitch.abs() < 1e-3 {
            (o.yaw, o.pitch) = (0.0, 0.0);
        }
    }
    let (Some(p), Ok(mut t)) = (player, cams.single_mut()) else { return };
    let eye = p.eye();
    control.crosshair = None;
    if let Some(portrait) = control.portrait {
        // No collision: the creator's room is the player's own space [design]. Aimed from
        // the drawn body's eye ([`BodyView::portrait_eye`]): the first-person eye can lag
        // where an idle put it (the opening's wake-up) while the body already stands.
        let (at, heading, pitch) = portrait.spot(body.portrait_eye().unwrap_or(eye), p.heading);
        t.translation = space::to_bevy(at);
        t.rotation = space::look_rotation(heading, pitch);
        return;
    }
    if !control.third_person_view() {
        t.rotation = p.body_view.map_or_else(|| space::look_rotation(p.heading, p.pitch), space::view_rotation);
        t.translation = space::to_bevy(eye);
        return;
    }
    let o = control.orbit;
    let heading = p.heading + o.yaw;
    let pitch = (p.pitch + o.pitch).clamp(-crate::player::MAX_PITCH, crate::player::MAX_PITCH);
    t.rotation = space::look_rotation(heading, pitch);
    let shoulder = orbit_shoulder(settings.map_or(DEFAULT_SHOULDER, |s| s.shoulder), o.yaw);
    let pivot = body.pivot.unwrap_or(eye);
    let (base, want) = third_person_spot(pivot, heading, pitch, shoulder, control.distance);
    // The camera is not pushed in by actors' hit volumes (combat C3).
    let filter = Filter { exclude_layers: fnv_phys::layers::character_ignores().union(fnv_phys::layers::actor_volumes()), ..Default::default() };
    // From the pivot sideways to the shoulder, then back along the view.
    let mut from = pivot;
    let mut to = base;
    let mut out = base;
    for leg in 0..2 {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len > 1e-3 {
            let reach = match phys.ray(from, d, len + WALL_MARGIN, filter.clone()) {
                Some(h) => (h.distance - WALL_MARGIN).max(0.0).min(len),
                None => len,
            };
            out = [from[0] + d[0] / len * reach, from[1] + d[1] / len * reach, from[2] + d[2] / len * reach];
            if reach < len {
                break;
            }
        }
        if leg == 0 {
            (from, to) = (out, want);
        }
    }
    t.translation = space::to_bevy(out);
    // While circling, the camera does not look where the player does: aim from the eye.
    if !control.orbit.active() {
        control.crosshair = Some(Crosshair::placed(out, heading, pitch, pivot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    /// The distance from `p` to the line through `a` along unit `d`.
    fn off_line(p: [f32; 3], a: [f32; 3], d: [f32; 3]) -> f32 {
        let r = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let t = r[0] * d[0] + r[1] * d[1] + r[2] * d[2];
        let q = [r[0] - d[0] * t, r[1] - d[1] * t, r[2] - d[2] * t];
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt()
    }

    #[test]
    fn the_portrait_camera_stands_in_front_and_looks_back() {
        let p = Portrait { yaw: 0.0, distance: 50.0, drop: 10.0, pitch: 0.0, shift: 0.0 };
        // Facing +y (heading 0): the camera is 50 in front, level with the point under the eye.
        let (at, heading, pitch) = p.spot([0.0, 0.0, 120.0], 0.0);
        assert!(at[0].abs() < 1e-4 && (at[1] - 50.0).abs() < 1e-4 && (at[2] - 110.0).abs() < 1e-4, "{at:?}");
        let back = space::look_dir(heading, pitch);
        assert!((back[1] + 1.0).abs() < 1e-5);
        // Turned a quarter: beside the player, still looking at them.
        let (at, heading, _) = Portrait { yaw: FRAC_PI_2, ..p }.spot([0.0, 0.0, 120.0], 0.0);
        assert!((at[0] - 50.0).abs() < 1e-3 && at[1].abs() < 1e-3);
        assert!((space::look_dir(heading, 0.0)[0] + 1.0).abs() < 1e-5);
        // Stepped left: looking back along −y from +y, the camera's left is +x.
        let (at, ..) = Portrait { shift: 10.0, ..p }.spot([0.0, 0.0, 120.0], 0.0);
        assert!((at[0] - 10.0).abs() < 1e-3 && (at[1] - 50.0).abs() < 1e-3, "{at:?}");
    }

    #[test]
    fn the_crosshair_ray_is_the_cameras_line_level_with_the_eye() {
        let (pivot, shoulder) = ([0.0, 0.0, 100.0], [40.0, -10.0]);
        let (_, cam) = third_person_spot(pivot, 0.0, 0.0, shoulder, 170.0);
        let cross = Crosshair { camera: cam, dir: space::look_dir(0.0, 0.0), pivot, shoulder };
        let eye = [0.0, 5.0, 100.0];
        let (start, dir) = Crosshair::ray(Some(&cross), eye, [0.0, 1.0, 0.0]);
        // Right of the eye by the shoulder, level with it along the view.
        assert!((start[0] - 40.0).abs() < 1e-3 && (start[1] - 5.0).abs() < 1e-3 && (start[2] - 90.0).abs() < 1e-3, "{start:?}");
        assert_eq!(dir, cross.dir);
        // First person: the eye's own ray.
        assert_eq!(Crosshair::ray(None, eye, [0.0, 1.0, 0.0]), (eye, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn a_shot_aims_from_the_eye_at_the_point_under_the_crosshair() {
        let (eye, start, dir) = ([0.0, 0.0, 100.0], [40.0, 0.0, 90.0], [0.0, 1.0, 0.0]);
        let hit = [40.0, 500.0, 90.0];
        let aim = Crosshair::aim_from(eye, start, dir, 1000.0, Some(hit));
        // It crosses the crosshair's line at the hit, not 40 units to its left.
        assert!(off_line(hit, eye, aim) < 1e-3, "{aim:?}");
        assert!(aim[0] > 0.0);
        // Nothing hit: towards the end of the reach.
        let far = Crosshair::aim_from(eye, start, dir, 1000.0, None);
        assert!(off_line([40.0, 1000.0, 90.0], eye, far) < 1e-2);
        // A hit beside the eye (not ahead of it): the view's direction.
        assert_eq!(Crosshair::aim_from(eye, start, dir, 1000.0, Some([40.0, 0.5, 90.0])), dir);
    }

    #[test]
    fn a_placed_camera_gives_its_lines_shoulder_offset() {
        let pivot = [10.0, -20.0, 120.0];
        for (h, p, sh) in [(0.3, -0.2, [40.0, -10.0]), (2.0, 0.5, [12.0, -10.0])] {
            let (_, cam) = third_person_spot(pivot, h, p, sh, 150.0);
            let c = Crosshair::placed(cam, h, p, pivot);
            assert!((c.shoulder[0] - sh[0]).abs() < 1e-3 && (c.shoulder[1] - sh[1]).abs() < 1e-3, "{:?}", c.shoulder);
        }
    }

    #[test]
    fn aiming_puts_the_point_on_the_cameras_line() {
        let (pivot, shoulder) = ([10.0, -20.0, 120.0], [40.0, -10.0]);
        for point in [[300.0, 400.0, 80.0], [-200.0, 150.0, 200.0], [60.0, -20.0, 120.0]] {
            let probe = Crosshair { camera: pivot, dir: [0.0, 1.0, 0.0], pivot, shoulder };
            let (h, p) = Crosshair::aim_at(Some(&probe), pivot, point);
            let (base, _) = third_person_spot(pivot, h, p, shoulder, 170.0);
            assert!(off_line(point, base, space::look_dir(h, p)) < 1e-2, "{point:?}");
        }
        // First person: straight at it from the eye.
        let (h, p) = Crosshair::aim_at(None, [0.0, 0.0, 0.0], [100.0, 0.0, 100.0]);
        assert!((h - FRAC_PI_2).abs() < 1e-5 && (p - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
    }

    #[test]
    fn third_person_sits_behind_and_right() {
        // Facing north, level: right is east, behind is south.
        let (base, cam) = third_person_spot([0.0, 0.0, 100.0], 0.0, 0.0, [40.0, -10.0], 170.0);
        assert!((base[0] - 40.0).abs() < 1e-4 && base[1].abs() < 1e-4 && (base[2] - 90.0).abs() < 1e-4);
        assert!((cam[0] - 40.0).abs() < 1e-4 && (cam[1] + 170.0).abs() < 1e-4 && (cam[2] - 90.0).abs() < 1e-4);
        // Facing east and looking down: behind is west and up.
        let (_, cam) = third_person_spot([0.0, 0.0, 0.0], FRAC_PI_2, -0.5, [0.0, 0.0], 100.0);
        assert!(cam[0] < -50.0 && cam[2] > 40.0, "{cam:?}");
        // Circling half way round: in front of the player, centred.
        assert_eq!(orbit_shoulder([40.0, -10.0], std::f32::consts::PI), [0.0, -10.0]);
        assert_eq!(orbit_shoulder([40.0, -10.0], 0.0), [40.0, -10.0]);
    }

    #[test]
    fn zoom_steps_and_clamps() {
        assert!(zoom(170.0, 1.0) < 170.0 && zoom(170.0, -1.0) > 170.0);
        assert_eq!(zoom(170.0, 100.0), MIN_DISTANCE);
        assert_eq!(zoom(170.0, -100.0), MAX_DISTANCE);
    }

    #[test]
    fn settings_fall_back_to_the_masters() {
        let s = ControlSettings::from_settings(None, Some(f32::NAN), Some(-12.0));
        assert_eq!(s, ControlSettings { sneak_mult: DEFAULT_SNEAK_MULT, shoulder: [40.0, -12.0] });
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Bindings>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<PlayerControl>()
            .init_resource::<Time>()
            .insert_state(PlayState::Playing)
            .add_systems(Update, read_controls);
        app
    }

    /// Press and release `key` over two frames (clearing the frame's edges as the input
    /// plugin would).
    fn tap(app: &mut App, key: KeyCode) {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
        app.update();
        let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        k.clear();
        k.release(key);
        app.update();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
    }

    fn control(app: &App) -> PlayerControl {
        *app.world().resource::<PlayerControl>()
    }

    #[test]
    fn game_keys_toggle() {
        let mut app = app();
        tap(&mut app, KeyCode::ControlLeft);
        assert!(control(&app).sneak);
        tap(&mut app, KeyCode::CapsLock);
        assert!(!control(&app).always_run);
        tap(&mut app, KeyCode::KeyQ);
        assert!(control(&app).auto_move);
        // Ready Item draws; a tap with the weapon out does not put it away.
        tap(&mut app, KeyCode::KeyR);
        assert!(control(&app).weapon_out);
        tap(&mut app, KeyCode::KeyR);
        assert!(control(&app).weapon_out && control(&app).ready_held.is_none());
        // Toggle POV tapped without moving the mouse switches the view.
        tap(&mut app, KeyCode::KeyF);
        assert_eq!(control(&app).pov, Pov::Third);
    }

    #[test]
    fn orbit_does_not_switch_the_view() {
        let mut app = app();
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Middle);
        app.update();
        assert!(control(&app).orbit.held);
        // mouse_look's part: the camera circled half round.
        let mut c = app.world_mut().resource_mut::<PlayerControl>();
        (c.orbit.moved, c.orbit.yaw, c.orbit.pitch) = (true, 3.0, 0.3);
        let mut m = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        m.clear();
        m.release(MouseButton::Middle);
        app.update();
        // Let go in first person: straight back to the first-person view (item 23).
        let c = control(&app);
        assert!(!c.orbit.held && c.pov == Pov::First && !c.orbit.active() && !c.third_person_view());
    }

    #[test]
    fn orbit_released_in_third_person_swings_back() {
        let o = Orbit { yaw: 1.0, pitch: 0.2, held: true, moved: true };
        assert_eq!(o.released(Pov::Third), Orbit { held: false, ..o });
        assert!(o.released(Pov::Third).active());
        assert_eq!(o.released(Pov::First), Orbit::default());
    }

    #[test]
    fn locks_follow_the_parameter_order() {
        let l = ControlLocks::from_flags([true, false, true, false, false, false, true]);
        assert_eq!(l, ControlLocks { movement: true, fighting: true, sneaking: true, ..Default::default() });
        assert_eq!(l.names(), ["movement", "fighting", "sneaking"]);
        assert!(l.any() && !ControlLocks::default().any());
        assert_eq!(ControlLocks::from_flags([true; 7]), ControlLocks::ALL);
        assert_eq!(ControlLocks::ALL.names().len(), 7);
    }

    /// Everything a player can ask for at once.
    fn busy() -> (MoveIntent, PlayerControl) {
        let intent = MoveIntent { forward: 1.0, right: -1.0, walk: true, sneak: true, jump: true };
        let control = PlayerControl {
            sneak: true,
            weapon_out: true,
            aiming: true,
            auto_move: true,
            pov: Pov::Third,
            orbit: Orbit { yaw: 1.0, pitch: 0.2, held: true, moved: true },
            ready_held: Some(0.1),
            trigger: true,
            reload: true,
            ammo_swap: true,
            ..Default::default()
        };
        (intent, control)
    }

    #[test]
    fn each_lock_strikes_its_own_group() {
        let (i0, c0) = busy();
        // Nothing locked: nothing changes.
        let (mut i, mut c) = (i0, c0);
        assert_eq!(ControlLocks::default().apply(&mut i, &mut c), [false; 7]);
        assert_eq!((i, c), (i0, c0));
        // Movement: no moving, jumping or Auto Move; the rest is the player's.
        let (mut i, mut c) = (i0, c0);
        let struck = ControlLocks { movement: true, ..Default::default() }.apply(&mut i, &mut c);
        assert_eq!(struck, [true, false, false, false, false, false, false]);
        // Asked again with nothing left to strike: nothing is counted.
        assert_eq!(ControlLocks { movement: true, ..Default::default() }.apply(&mut i, &mut c), [false; 7]);
        assert_eq!(i, MoveIntent { forward: 0.0, right: 0.0, jump: false, ..i0 });
        assert_eq!(c, PlayerControl { auto_move: false, ..c0 });
        // Fighting: the weapon goes away and nothing fires.
        let (mut i, mut c) = (i0, c0);
        ControlLocks { fighting: true, ..Default::default() }.apply(&mut i, &mut c);
        assert_eq!(i, i0);
        assert_eq!(c, PlayerControl { weapon_out: false, aiming: false, ready_held: None, trigger: false, reload: false, ammo_swap: false, ..c0 });
        // POV: back to first person, the camera behind the eyes.
        let (mut i, mut c) = (i0, c0);
        ControlLocks { pov: true, ..Default::default() }.apply(&mut i, &mut c);
        assert_eq!(i, i0);
        assert_eq!(c, PlayerControl { pov: Pov::First, orbit: Orbit::default(), ..c0 });
        assert!(!c.third_person_view());
        // Sneaking: the player stands up.
        let (mut i, mut c) = (i0, c0);
        ControlLocks { sneaking: true, ..Default::default() }.apply(&mut i, &mut c);
        assert_eq!(i, MoveIntent { sneak: false, ..i0 });
        assert_eq!(c, PlayerControl { sneak: false, ..c0 });
        // Pip-Boy, looking and rollover text are held where they are read (menu.rs,
        // player.rs, activate.rs).
        let (mut i, mut c) = (i0, c0);
        assert_eq!(ControlLocks { pipboy: true, looking: true, rollover: true, ..Default::default() }.apply(&mut i, &mut c), [false; 7]);
        assert_eq!((i, c), (i0, c0));
        // Everything at once.
        let (mut i, mut c) = (i0, c0);
        assert_eq!(ControlLocks::ALL.apply(&mut i, &mut c), [true, false, true, true, false, false, true]);
    }

    fn locked_app(locks: ControlLocks) -> App {
        let mut app = app();
        app.init_resource::<MoveIntent>()
            .init_resource::<crate::furniture::PlayerSeat>()
            .init_resource::<LockStats>()
            .insert_resource(locks)
            .add_systems(Update, enforce_locks.after(read_controls));
        app
    }

    #[test]
    fn locked_keys_do_nothing() {
        // DisablePlayerControls with no parameters: movement, Pip-Boy, fighting, POV.
        let mut app = locked_app(ControlLocks::from_flags([true, true, true, true, false, false, false]));
        tap(&mut app, KeyCode::KeyR);
        tap(&mut app, KeyCode::KeyF);
        tap(&mut app, KeyCode::KeyQ);
        let c = control(&app);
        assert!(!c.weapon_out && c.pov == Pov::First && !c.auto_move);
        // Sneaking is still the player's.
        tap(&mut app, KeyCode::ControlLeft);
        assert!(control(&app).sneak);
        // The sneaking flag stands the player up, and Ctrl does nothing.
        app.world_mut().resource_mut::<ControlLocks>().sneaking = true;
        app.update();
        assert!(!control(&app).sneak);
        tap(&mut app, KeyCode::ControlLeft);
        assert!(!control(&app).sneak);
        // Enabled again: the keys work, and nothing came back by itself.
        *app.world_mut().resource_mut::<ControlLocks>() = ControlLocks::default();
        app.update();
        let c = control(&app);
        assert!(!c.sneak && !c.weapon_out && c.pov == Pov::First);
        tap(&mut app, KeyCode::KeyR);
        tap(&mut app, KeyCode::ControlLeft);
        assert!(control(&app).weapon_out && control(&app).sneak);
    }

    #[test]
    fn a_lock_puts_the_weapon_away_and_the_view_inside() {
        let mut app = locked_app(ControlLocks::default());
        tap(&mut app, KeyCode::KeyR);
        tap(&mut app, KeyCode::KeyF);
        assert!(control(&app).weapon_out && control(&app).pov == Pov::Third);
        *app.world_mut().resource_mut::<ControlLocks>() = ControlLocks { fighting: true, pov: true, ..Default::default() };
        app.update();
        assert!(!control(&app).weapon_out && control(&app).pov == Pov::First);
    }

    #[test]
    fn locked_movement_keeps_the_player_seated() {
        let mut app = locked_app(ControlLocks { movement: true, ..Default::default() });
        // A movement key was pressed on furniture and a test aid holds Forward.
        app.world_mut().resource_mut::<crate::furniture::PlayerSeat>().leave = true;
        *app.world_mut().resource_mut::<MoveIntent>() = MoveIntent { forward: 1.0, jump: true, ..Default::default() };
        app.update();
        assert!(!app.world().resource::<crate::furniture::PlayerSeat>().leave);
        assert_eq!(*app.world().resource::<MoveIntent>(), MoveIntent::default());
        let stats = *app.world().resource::<LockStats>();
        assert_eq!(stats.held, [1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(stats.summary(), "movement 1");
        assert_eq!(LockStats::default().summary(), "nothing");
    }
}
