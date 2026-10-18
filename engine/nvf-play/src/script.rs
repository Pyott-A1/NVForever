//! Layer 5 in the game (workstream 5D, docs/design/layer5-play.md): the `fnv-game`
//! runtime runs next to the simulation in `nvforever --play`.
//!
//! - [`ScriptState`] holds the `fnv_game::Game` (script instances, quests, dispatch).
//!   The Layer 4 world stays in [`SimState`]; for each script run it is swapped into the
//!   game and back ([`with_world`]), so every other part of the game keeps reading
//!   `SimState::world` as before.
//! - The script data (`fnv_game::GameData`: every unit decoded, quests, messages) is
//!   built on its own thread at the start; scripts start once it is ready and the player
//!   stands in the world. The new-game state starts every quest flagged "Start game
//!   enabled" (`Game::new_game`).
//! - Each frame (`PlaySet::Scripts`, after activation, before the menus): inventory
//!   changes made by the menus become OnAdd / OnEquip / OnUnequip / OnDrop; a new player
//!   cell loads cells (OnLoad); trigger volumes give OnTriggerEnter / OnTrigger /
//!   OnTriggerLeave ([`crate::script_trigger`]); then `Game::tick` with the frame's real
//!   time: quest scripts by their delay, then GameMode (or MenuMode while a menu or a
//!   message box is open) of the scripted references in loaded cells, then the dialogue
//!   lines being said.
//! - The trace events of the frame become engine effects
//!   ([`crate::script_events::effects`]): `RefChanged` for enabled / disabled / deleted
//!   references (and their enable children), `Travel` when a script moves the player or
//!   activates a door for them, `OpenContainer`, lines on screen, the text of message
//!   boxes and dialogue lines for the overlay ([`crate::script_hud`]).
//! - Menus and message boxes go through the runtime's API (scripts.md §7.2, §7.7): the
//!   container and inventory menus call [`engine_menu_opened`] / [`engine_menu_closed`]
//!   (`Game::open_menu` / `close_menu` with 1008 / 1002); a script's message box or
//!   character-creation menu stays open until the player closes it
//!   ([`ScriptState::answer`], keys in [`crate::script_hud`]); the player is held in
//!   `PlayState::Menu` meanwhile ([`hold_player`]). The engine no longer sets
//!   `Game::menu` itself.
//! - Activation by hand goes through [`activate_by_hand`] (called by `activate.rs`): a
//!   reference whose script has an OnActivate block runs it *instead of* the default
//!   action, which then runs only if the script calls `Activate` (`fnv_game`'s rule).
//! - Conversations (`crate::dialogue`, docs/design/dialogue-play.md): activating an actor
//!   whose default action is to talk starts one ([`Runtime::talk_after`]: by hand, from an
//!   OnActivate block's `Activate` and from any script's `Activate player`); scripts start
//!   them with StartConversation. The dialogue screen reads `Game::dialogue` and acts on
//!   it between ticks through [`Runtime::player_action`].
//! - `--new-game` ([`NewGame`]): the opening starts as the New Game menu starts it (VCG00
//!   at stage 0, [`Runtime::start_opening`]); the player starts at
//!   `VCG01PlayerStartMarkerREF` ([`new_game_start`]).
//! - `DisablePlayerControls` / `EnablePlayerControls` hold the player: after each frame's
//!   scripts the runtime's seven flags go to `control::ControlLocks` ([`sync_locks`]),
//!   which the input, activation, the HUD and the inventory key obey
//!   (docs/spec-notes/controls.md §5).
//!
//! Switches and test aids (environment):
//! - `NVF_PLAY_SCRIPTS=0` turns scripts off (Layer 4 play as before).
//! - `NVF_PLAY_SCRIPT_TRACE=<path>` writes the trace events as JSON lines (one
//!   `fnv.script.trace/1` event per line, with `t` = seconds since scripts started and `gt`
//!   = the runtime's game time, `Game::time`), the test aids (`aid`), the player's
//!   answers to menus (`menu_answer`) and the control groups scripts have off whenever
//!   they change (`controls`), and a `final` line at exit (quests touched, unhandled and
//!   stubbed calls, tick times).
//! - `NVF_PLAY_SCRIPT_FUNCTIONS=<functions.json>` overrides the function catalogue
//!   (default: the repository's `schemas/script/functions.json`).
//! - `NVF_PLAY_TEST_SCRIPT=<when>:<op>[,...]`: stand-ins for Layer 6 and the player
//!   ([`crate::script_events::ScriptAid`]; `<when>` is seconds after the start, or
//!   `<seconds>@<quest>=<stage>`, seconds after the stage was first seen done; when the
//!   scripts first run after the first timed step's time, the timed steps all come that
//!   much later, [`script_events::delay_late_aids`]):
//!   `start:<quest>`, `stop:<quest>`, `stage:<quest>:<n>`, `objective:<quest>:<n>`,
//!   `objective_done:<quest>:<n>`, `info:<INFO>[:<speaker>]`, `button:<n>`, `close`,
//!   `message:<MESG>`, `menu:<code>`,
//!   `var:<form>:<variable>:<value>`, `walk:<reference>`, `activate:<reference>`,
//!   `enable:<reference>`, `combat:<actor>:<target>`, `moveto:<reference>:<marker>[:x:y:z]`,
//!   `say_to:<speaker>:<target>:<topic>[:<noTargetLookFlag>]`.
//! - `NVF_PLAY_TEST_SCRIPT_MENUS=<seconds>|off`: every menu or message box a script opens
//!   is closed (OK / Done, button 0) after it has been on screen that long. Unattended
//!   runs (any `NVF_PLAY_TEST_*` variable, or `--auto-walk`) default to
//!   [`UNATTENDED_MENU_SECONDS`], so a check that does not answer the boxes is not held by
//!   them [decision]; other runs to off.
//! - `nvforever --play --no-messages` ([`NoMessages`]): a message box that asks nothing
//!   (an OK button only) closes as soon as it is on screen, as OK would; a box with two
//!   or more buttons stays for the player [decision: the flag never chooses an answer].

use crate::activate::outcome::{self, Destination, Effect};
use crate::control::ControlLocks;
use crate::menu::model::Mode;
use crate::menu::ActiveMenu;
use crate::phys::PhysState;
use crate::player::Player;
use crate::script_events::{self, Capsule, EffectCtx, InvView, ScriptAid, ScriptEffect, TimedAid};
use crate::script_hud::ScriptHud;
use crate::sim::SimState;
use crate::loading::TravelRequest;
use crate::{ItemUsed, Location, OpenContainer, PlayData, PlaySet, PlayState, PlayWorld, RefChanged};
use bevy::prelude::*;
use fnv_game::game::{event, menus, ActMode};
use fnv_game::trace::{Event, EventKind};
use fnv_game::{Game, GameConfig, GameData, InstKey};
use fnv_script::vm::{ArgValue, Value};
use fnv_script::FunctionTable;
use fnv_sim::world::{Activation, Outcome};
use fnv_sim::{FormId, RefTable, World};
use fnv_world::WorldIndex;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::Instant;

/// MenuMode code while a message box is open (`fnv_game::game::menus::MESSAGE`): shipped
/// scripts read the chosen button in `Begin MenuMode 1001` blocks [observed: FalloutNV
/// SCPT 0x00105236, the Goodsprings chemistry set] as well as in GameMode (SCPT
/// 0x0011F291).
pub const MENU_MESSAGE: u16 = menus::MESSAGE;
/// The SayTo function's opcode [observed: schemas/script/functions.json].
const SAY_TO: u16 = 0x1034;
/// `MoveTo` [observed: schemas/script/functions.json, 0x109E].
const MOVE_TO: u16 = 0x109E;
/// `PlayGroup` [observed: activators.md §5].
const PLAY_GROUP: u16 = 0x1013;
/// The container menu [documented: GECK wiki MenuMode, as used in
/// docs/spec-notes/xnvse-capture.md and simulation.md captures].
pub const MENU_CONTAINER: u16 = 1008;
/// The inventory (Pip-Boy) menu [hypothesis: the GECK wiki's list as remembered; not
/// checked against a capture].
pub const MENU_INVENTORY: u16 = 1002;

/// The Sleep / Wait menu (`crate::wait`) [observed: `Begin MenuMode 1012` blocks of six
/// FalloutNV.esm scripts, one commented as the rest menu; docs/spec-notes/wait-menu.md
/// §5].
pub const MENU_SLEEP_WAIT: u16 = 1012;

/// The barter menu (`crate::menu`), which scripts open with ShowBarterMenu
/// (`fnv_game::game::menus::BARTER`).
pub const MENU_BARTER: u16 = menus::BARTER;

/// The MenuMode code of an engine menu.
pub fn menu_code(mode: Mode) -> u16 {
    match mode {
        Mode::Container(_) => MENU_CONTAINER,
        Mode::Inventory => MENU_INVENTORY,
        Mode::Barter(_) => MENU_BARTER,
    }
}

/// A menu the scripts opened (a message box, a character-creation menu), as opposed to
/// the engine's container, inventory, barter, wait, recipe and roulette menus: the script overlay shows it
/// and the player closes it there.
pub fn is_script_menu(code: u16) -> bool {
    code != MENU_CONTAINER
        && code != MENU_INVENTORY
        && code != MENU_SLEEP_WAIT
        && code != MENU_BARTER
        && code != crate::terminal::MENU_HACKING
        && code != crate::terminal::MENU_TERMINAL
        && code != crate::lockpick::MENU_LOCKPICK
        && code != crate::blackjack::MENU_BLACKJACK
        && code != fnv_game::crafting::RECIPE_MENU
        && code != fnv_game::repair::REPAIR_SERVICES_MENU
        && code != crate::roulette::MENU_ROULETTE
}

/// Default of `NVF_PLAY_TEST_SCRIPT_MENUS` in unattended runs, seconds.
pub const UNATTENDED_MENU_SECONDS: f32 = 0.25;

/// The game's periodic package choice, seconds [documented: GECK wiki; packages.md §3].
pub const PACKAGE_INTERVAL: f64 = 10.0;

/// `nvforever --play --no-messages`: message boxes that ask nothing close at once
/// (inserted before the play plugin).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct NoMessages;

/// `nvforever --play --new-game`: the game starts as a new game does (inserted before
/// the play plugin).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct NewGame;

/// The interior a new game starts in: VCG00 stage 0 moves the player to
/// `VCG01PlayerStartMarkerREF` [observed in the masters], which the new-game save `nvfnew`
/// has in Doc Mitchell's house, cell 0x00103DF9 [observed: layer5-captures.md §10.2].
pub const NEW_GAME_CELL: &str = "GSDocMitchellHouse";

/// Where `--new-game` puts the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewGameStart {
    pub interior: u32,
    pub feet: [f32; 3],
    pub heading_deg: f32,
}

/// The new game's start: the marker the opening moves the player to
/// (`fnv_game::runner::NEW_GAME_START`) in [`NEW_GAME_CELL`].
pub fn new_game_start(lo: &fnv_esm::LoadOrder, schema: &fnv_esm::Schema, index: &WorldIndex) -> Result<NewGameStart, String> {
    let cell = index.cell_by_name(NEW_GAME_CELL).filter(|c| c.world.is_none()).ok_or_else(|| format!("no interior cell {NEW_GAME_CELL}"))?;
    for &r in &cell.refs {
        let Some((pi, rec)) = fnv_world::fields::winner(lo, r) else { continue };
        let Ok(f) = fnv_world::fields::Fields::new(rec, pi, schema) else { continue };
        if f.string(b"EDID").is_some_and(|e| e.eq_ignore_ascii_case(fnv_game::runner::NEW_GAME_START)) {
            let p = fnv_world::placed::placement(lo, schema, r).ok_or_else(|| format!("{r:08X}: no placement"))?;
            return Ok(NewGameStart { interior: cell.form_id, feet: p.position, heading_deg: outcome::heading_deg(p.rotation) });
        }
    }
    Err(format!("no {} in {NEW_GAME_CELL}", fnv_game::runner::NEW_GAME_START))
}

/// Where the function catalogue is when `NVF_PLAY_SCRIPT_FUNCTIONS` is not set.
const DEFAULT_FUNCTIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/script/functions.json");

type Loaded = Result<(&'static GameData, f64), String>;

/// The Layer 5 runtime of the game being played.
#[derive(Resource)]
pub struct ScriptState {
    loader: Option<Mutex<Receiver<Loaded>>>,
    pub rt: Option<Runtime>,
    /// Why scripts are not running (switched off, data failed), for the log.
    pub off: Option<String>,
    trace: Option<TraceLog>,
    aids: Vec<TimedAid>,
    /// The timed aids were looked at in a frame the scripts ran
    /// ([`script_events::delay_late_aids`] then).
    aids_started: bool,
    /// Gated aids: when each (quest, stage) was first seen done (seconds since start).
    gates: HashMap<(String, i32), f32>,
    /// `NVF_PLAY_TEST_SCRIPT_MENUS`: close a script's menu after this many seconds.
    auto_close: Option<f32>,
    /// `--no-messages`: close a message box without a choice at once.
    no_messages: bool,
    /// The script menu on screen (code, front box) and since when (seconds).
    on_screen: Option<(u16, Option<FormId>, f32)>,
    /// `walk:` test aids due: where to put the player's feet.
    walks: Vec<(String, [f32; 3])>,
    new_game: bool,
    /// Aid items the player used that the runtime has not taken yet (used before the
    /// scripts were ready, or under a loading screen or the pause menu).
    used: Vec<FormId>,
    /// Console lines for the next script frame (`crate::console`).
    pub console: Vec<crate::console::ConsoleCall>,
    /// What the script frame answered to them.
    pub console_out: Vec<crate::console::ConsoleReply>,
    /// A save to restore when the runtime is made (`crate::save`, nvforever `--load`).
    pending_game: Option<PendingGame>,
}

/// A save's game snapshot for the runtime to restore when it is made (`crate::save`).
#[derive(Resource)]
pub struct PendingGame(pub Box<fnv_game::game::GameSnapshot>, pub PathBuf);

impl ScriptState {
    /// The scripts are being loaded: not running yet, and not switched off.
    pub fn loading(&self) -> bool {
        self.rt.is_none() && self.off.is_none()
    }

    /// A save is waiting to be restored into the runtime.
    pub fn pending_game_waiting(&self) -> bool {
        self.pending_game.is_some()
    }
}

/// The running game and what the engine side remembers between frames.
pub struct Runtime {
    pub game: Game<'static>,
    /// The cell last handed to `Game::player_entered`.
    player_cell: Option<FormId>,
    /// The player's inventory after the last script run (changes since are the menus').
    inventory: InvView,
    /// The newest created reference seen (a newer one of a removed item is a drop).
    created_seen: Option<FormId>,
    /// References seen marked for delete (MarkForDelete has no trace event).
    marked: BTreeSet<FormId>,
    /// Indices into `game.events` whose effects the caller applied (activation by hand).
    skip: BTreeSet<usize>,
    /// The player answered the open script menu: it closes after this frame's tick (so a
    /// `MenuMode 1001` block still runs once with the button pressed). The menu and the
    /// box that was in front.
    closing: Option<(u16, Option<FormId>)>,
    /// `--new-game`: the opening starts on the first frame scripts run.
    opening_pending: bool,
    /// Where the player was last frame, and frames left whose time does not count (a
    /// door's loading screen).
    location: Option<Location>,
    frozen_frames: u32,
    pub triggers: crate::script_trigger::Triggers,
    /// Enable parent → references that follow it.
    children: HashMap<FormId, Vec<FormId>>,
    started: Instant,
    pub stats: TickStats,
    /// What the player's aid items did (`crate::aid`).
    pub aid: crate::aid::AidStats,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TickStats {
    pub frames: u64,
    pub total_ms: f64,
    pub worst_ms: f64,
    pub events: u64,
}

impl ScriptState {
    fn new(d: PlayData, refs: &'static RefTable, new_game: bool, unattended: bool, no_messages: bool) -> ScriptState {
        let off = match std::env::var("NVF_PLAY_SCRIPTS").as_deref() {
            Ok("0") | Ok("off") | Ok("false") => Some("NVF_PLAY_SCRIPTS=0".to_string()),
            _ => None,
        };
        let aids = match std::env::var("NVF_PLAY_TEST_SCRIPT") {
            Ok(v) => match script_events::parse_aids(&v) {
                Ok(a) => a,
                Err(e) => {
                    warn!("NVF_PLAY_TEST_SCRIPT: {e}");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let auto_close = match std::env::var("NVF_PLAY_TEST_SCRIPT_MENUS").as_deref() {
            Ok("off") | Ok("false") => None,
            Ok(v) => match v.trim().parse::<f32>() {
                Ok(s) if s >= 0.0 => Some(s),
                _ => {
                    warn!("NVF_PLAY_TEST_SCRIPT_MENUS={v}: expected seconds or off");
                    None
                }
            },
            Err(_) => (unattended || d.auto_walk > 0.0).then_some(UNATTENDED_MENU_SECONDS),
        };
        if let Some(s) = auto_close {
            info!("test aid: menus and message boxes that scripts open close themselves after {s} s");
        }
        if no_messages {
            info!("--no-messages: message boxes without a choice close at once");
        }
        let trace = std::env::var_os("NVF_PLAY_SCRIPT_TRACE").map(PathBuf::from).and_then(|p| match TraceLog::create(&p) {
            Ok(t) => Some(t),
            Err(e) => {
                warn!("NVF_PLAY_SCRIPT_TRACE {}: {e}", p.display());
                None
            }
        });
        let loader = off.is_none().then(|| Mutex::new(start_loading(d, refs)));
        ScriptState { loader, rt: None, off, trace, aids, aids_started: false, gates: HashMap::new(), auto_close, no_messages, on_screen: None, walks: Vec::new(), new_game, used: Vec::new(), console: Vec::new(), console_out: Vec::new(), pending_game: None }
    }

    /// The player answers the script's open menu: `Some(b)` picks button b (0 = first) of
    /// the front message box, `None` is OK / Done. The menu closes after this frame's
    /// scripts ran (`Game::close_menu`; the next queued box then opens). `how` is for the
    /// trace. Returns whether there was a script menu to answer.
    pub fn answer(&mut self, button: Option<i32>, how: &str) -> bool {
        let Some(rt) = self.rt.as_mut() else { return false };
        let menu = rt.game.menu;
        let ok = rt.answer(button);
        if ok {
            if let Some(log) = self.trace.as_mut() {
                log.write(&serde_json::json!({"kind": "menu_answer", "frame": rt.game.frame, "menu": menu, "button": button, "by": how,
                    "t": secs(rt.started), "gt": rt.game.time}));
            }
        }
        ok
    }

    /// A new game whose opening has not started yet (the scripts are loading, or wait for
    /// the player to stand in the world).
    fn before_opening(&self) -> bool {
        match self.rt.as_ref() {
            Some(rt) => rt.opening_pending,
            None => self.new_game && self.loader.is_some(),
        }
    }

    /// The control groups scripts have off now (`Game::player.controls_disabled`). A new
    /// game holds every control until its opening has started: VCG00's first frame turns
    /// the controls off, and the scripts take about a second to load [decision: the
    /// player must not walk off meanwhile].
    pub fn locks(&self) -> ControlLocks {
        match self.rt.as_ref() {
            _ if self.before_opening() => ControlLocks::ALL,
            // The player is dead: nothing more to do [design] (`crate::combat`).
            Some(rt) if rt.game.is_dead(FormId::PLAYER_REF) => ControlLocks::ALL,
            Some(rt) => ControlLocks::from_flags(rt.game.player.controls_disabled),
            None => ControlLocks::default(),
        }
    }

    /// A line of the engine's own in the trace log (a test aid's step, the player's
    /// input on the dialogue screen), with `frame`, `t` and `gt` added.
    pub(crate) fn trace_note(&mut self, mut v: serde_json::Value) {
        let (Some(rt), Some(log)) = (self.rt.as_ref(), self.trace.as_mut()) else { return };
        v["frame"] = serde_json::json!(rt.game.frame);
        v["t"] = serde_json::json!(secs(rt.started));
        v["gt"] = serde_json::json!(rt.game.time);
        log.write(&v);
        let _ = log.out.flush();
    }
}

fn secs(since: Instant) -> f64 {
    (since.elapsed().as_secs_f64() * 1000.0).round() / 1000.0
}

/// The container or inventory menu opened (menu.rs): it is the runtime's open menu
/// (`Game::open_menu`), MenuMode 1008 / 1002 from the next frame.
pub fn engine_menu_opened(script: Option<&mut ScriptState>, mode: Mode) {
    if let Some(rt) = script.and_then(|s| s.rt.as_mut()) {
        rt.game.open_menu(menu_code(mode));
    }
}

/// The container or inventory menu closed (menu.rs): `Game::close_menu` when it is still
/// the runtime's open menu (a message box shown meanwhile then opens).
pub fn engine_menu_closed(script: Option<&mut ScriptState>, mode: Mode) {
    if let Some(rt) = script.and_then(|s| s.rt.as_mut()) {
        if rt.game.menu == Some(menu_code(mode)) {
            rt.game.close_menu();
        }
    }
}

/// Build the script data on its own thread (decoding every unit takes about a second;
/// render streaming and physics start meanwhile).
fn start_loading(d: PlayData, refs: &'static RefTable) -> Receiver<Loaded> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("nvf-scripts".into()).spawn(move || {
        let t = Instant::now();
        let path = std::env::var_os("NVF_PLAY_SCRIPT_FUNCTIONS").map_or_else(|| PathBuf::from(DEFAULT_FUNCTIONS), PathBuf::from);
        let r = (|| {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            let table = FunctionTable::load(&path).map_err(|e| e.to_string())?;
            let data: &'static GameData = Box::leak(Box::new(GameData::build(d.lo, d.schema, table, &json, refs)));
            Ok((data, t.elapsed().as_secs_f64()))
        })();
        let _ = tx.send(r);
    });
    if let Err(e) = spawned {
        let (tx2, rx2) = mpsc::channel();
        let _ = tx2.send(Err(format!("cannot start the script loader thread: {e}")));
        return rx2;
    }
    rx
}

/// Run `f` with the simulation's world inside the game (and put it back after).
pub fn with_world<R>(rt: &mut Runtime, sim: &mut SimState, f: impl FnOnce(&mut Runtime) -> R) -> R {
    std::mem::swap(&mut rt.game.world, &mut sim.world);
    let r = f(rt);
    std::mem::swap(&mut rt.game.world, &mut sim.world);
    r
}

impl Runtime {
    fn new(data: &'static GameData, sim: &mut SimState, new_game: bool, packages: bool) -> Runtime {
        // Message boxes are menus until the player closes them (the default,
        // `GameConfig::modal_message_boxes`). With actors living by their packages, the
        // packages' scripts and idles run from the first frame (the residents ask for the
        // same interval when they start).
        let cfg = GameConfig { frame_seconds: 1.0 / 60.0, package_interval: packages.then_some(PACKAGE_INTERVAL), ..GameConfig::default() };
        let game = Game::new(data, World::new(sim.forms, sim.refs, sim.world.config()), cfg);
        // The simulation's world is the one played (`with_world`): its placed actors carry
        // their template's items too (template flag "Inventory").
        sim.world.set_inventory_sources(data.inventory_sources(sim.refs));
        let mut children: HashMap<FormId, Vec<FormId>> = HashMap::new();
        for (id, p) in &sim.refs.refs {
            if let Some(ep) = p.enable_parent {
                children.entry(ep.reference).or_default().push(*id);
            }
        }
        let mut rt = Runtime {
            game,
            player_cell: None,
            inventory: InvView::default(),
            created_seen: None,
            marked: BTreeSet::new(),
            skip: BTreeSet::new(),
            closing: None,
            opening_pending: new_game,
            location: None,
            frozen_frames: 0,
            triggers: Default::default(),
            children,
            started: Instant::now(),
            stats: TickStats::default(),
            aid: crate::aid::AidStats::default(),
        };
        with_world(&mut rt, sim, |rt| {
            rt.game.new_game();
            rt.snapshot_inventory();
        });
        // Placed actors with Base Health 0 lie dead from the start (docs/spec-notes/dead-on-spawn.md).
        let corpses = rt.game.extra.values().filter(|e| e.dead).count();
        info!("new game: {corpses} placed actors start dead (Base Health 0)");
        rt
    }

    /// Put a save's game state in place of the new game's (`crate::save`): the
    /// simulation's world and the game's own state. The random numbers start from a
    /// fresh position unless `NVF_PLAY_TEST_SAVE_SAME_ROLLS=1` (docs/design/save-load.md
    /// §5 q3). Returns the references the save changed (to redraw).
    pub(crate) fn restore_save(&mut self, sim: &mut SimState, mut s: fnv_game::game::GameSnapshot) -> Result<Vec<FormId>, String> {
        if std::env::var_os("NVF_PLAY_TEST_SAVE_SAME_ROLLS").is_none_or(|v| v == "0") {
            let jump = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) << 16;
            for pos in [&mut s.rng_word_pos, &mut s.sim.rng_word_pos] {
                let p: u128 = pos.parse().map_err(|_| format!("bad rng_word_pos {pos:?}"))?;
                *pos = p.wrapping_add(jump).to_string();
            }
        }
        let world = World::restore(sim.forms, sim.refs, &s.sim)?;
        let changed: Vec<FormId> = s.sim.refs.keys().copied().collect();
        let data = self.game.data;
        let cfg = self.game.cfg;
        self.game = Game::restore(data, World::new(sim.forms, sim.refs, world.config()), cfg, &s)?;
        sim.world = world;
        sim.world.set_inventory_sources(data.inventory_sources(sim.refs));
        self.opening_pending = false;
        self.player_cell = None;
        with_world(self, sim, |rt| rt.snapshot_inventory());
        Ok(changed)
    }

    /// The New Game menu starts VCG00 [documented: GECK wiki SetStage notes, scripts.md
    /// §5]; VCG00's own MenuMode block sets its stage 0 while the stage is below 5
    /// [observed in the masters: SCPT 0x00102035], which happens in the main menu as the
    /// game starts [hypothesis]. Stage 0 moves the player to the start marker and goes on
    /// to stage 90 and its 3 s countdown [observed in the masters].
    pub fn start_opening(&mut self) {
        let g = &mut self.game;
        match g.data.by_edid("VCG00").filter(|q| g.data.is_quest(*q)) {
            Some(q) => {
                g.start_quest(q);
                g.set_stage(q, 0);
                info!("new game: VCG00 started at stage 0 (now at stage {})", g.quest_state(q).stage());
            }
            None => warn!("new game: no quest VCG00"),
        }
    }

    /// See [`ScriptState::answer`].
    pub fn answer(&mut self, button: Option<i32>) -> bool {
        if self.closing.is_some() {
            return false;
        }
        let Some(menu) = self.game.menu.filter(|&m| is_script_menu(m)) else { return false };
        let front = if menu == MENU_MESSAGE { self.game.boxes.first().copied() } else { None };
        if let Some(f) = front {
            if let Some(m) = self.game.message_box.as_mut().filter(|m| m.message == f && m.pressed.is_none()) {
                let b = button.unwrap_or(0);
                if b < 0 || b as usize >= m.buttons {
                    return false;
                }
                m.pressed = Some(b);
            }
        }
        info!(
            "script: menu {menu}{} answered ({})",
            front.map_or_else(String::new, |f| format!(" (message {})", self.game.data.name(f))),
            button.map_or("OK".to_string(), |b| format!("button {b}"))
        );
        self.closing = Some((menu, front));
        true
    }

    /// After the tick: close the menu the player answered, if it is still the one open.
    fn finish_answer(&mut self) {
        if let Some((menu, front)) = self.closing.take() {
            if self.game.menu == Some(menu) && (menu != MENU_MESSAGE || self.game.boxes.first().copied() == front) {
                self.game.close_menu();
            }
        }
    }

    /// The player's inventory and the newest created reference, as scripts left them.
    fn snapshot_inventory(&mut self) {
        let w = &self.game.world;
        self.inventory = InvView::of(w.peek(FormId::PLAYER_REF).and_then(|s| s.inventory.as_ref()));
        self.created_seen = w.created().map(|(id, _)| id).last().or(self.created_seen);
    }

    /// Inventory changes since the last script run (menus): OnAdd for items added,
    /// OnEquip / OnUnequip, OnDrop for items that left as new created references
    /// [hypothesis: the item's script runs on the player's copy, per change, not per
    /// unit; items stored into a container run nothing here].
    fn observe_inventory(&mut self) {
        let g = &mut self.game;
        let now = InvView::of(g.world.peek(FormId::PLAYER_REF).and_then(|s| s.inventory.as_ref()));
        let changes = script_events::inventory_changes(&self.inventory, &now);
        if changes.is_empty() {
            return;
        }
        let seen = self.created_seen;
        let dropped: BTreeSet<FormId> = g.world.created().filter(|(id, _)| seen.is_none_or(|s| *id > s)).map(|(_, s)| s.base).collect();
        let player = Some(Value::Ref(FormId::PLAYER_REF.0));
        for (item, n) in &changes.added {
            debug!("script: {n} x {item} reached the player's inventory outside scripts");
            g.item_event(FormId::PLAYER_REF, *item, event::ON_ADD, player);
        }
        for item in &changes.equipped {
            g.item_event(FormId::PLAYER_REF, *item, event::ON_EQUIP, player);
        }
        for item in &changes.unequipped {
            g.item_event(FormId::PLAYER_REF, *item, event::ON_UNEQUIP, player);
        }
        for (item, _) in changes.removed.iter().filter(|(i, _)| dropped.contains(i)) {
            g.item_event(FormId::PLAYER_REF, *item, event::ON_DROP, player);
        }
        self.snapshot_inventory();
    }

    /// The player is in a new cell: the cells around it load (OnLoad).
    fn follow_player(&mut self) {
        let cell = self.game.world.peek(FormId::PLAYER_REF).and_then(|s| s.cell);
        if cell.is_some() && cell != self.player_cell {
            self.player_cell = cell;
            self.game.player_entered(cell);
            debug!("script: player in cell {:?}; {} cells loaded for scripts", cell, self.game.loaded_cells.len());
        }
    }

    /// References newly marked for delete (no trace event for it).
    fn newly_marked(&mut self) -> Vec<FormId> {
        let now: Vec<FormId> = self.game.extra.iter().filter(|(_, e)| e.marked_for_delete).map(|(r, _)| *r).collect();
        now.into_iter().filter(|r| self.marked.insert(*r)).collect()
    }

    fn by_hand(&mut self, target: FormId) -> Result<ByHand, String> {
        self.observe_inventory();
        let player = FormId::PLAYER_REF;
        let from = self.game.events.len();
        let g = &mut self.game;
        let has_block = g.ref_script(target).and_then(|s| g.data.script(s)).is_some_and(|u| u.has_block(event::ON_ACTIVATE));
        let r = if has_block {
            info!("script: activating {target} runs its OnActivate block instead of the default action");
            g.activate(player, target, ActMode::Hand).map(|_| ByHand::Script)
        } else if g.is_dead(target) {
            // A dead actor opens its inventory (the loot menu), `fnv_game`'s default
            // activation rule (combat C5).
            let a = fnv_sim::world::Activation { outcome: Outcome::OpenedContainer { container: target }, unlocked_with: None, script: None };
            self.skip.insert(g.events.len());
            g.emit(EventKind::Activated { reference: target, by: player, outcome: serde_json::to_value(&a).ok() });
            Ok(ByHand::Default(a))
        } else {
            match g.world.activate(player, target) {
                Ok(mut a) => {
                    self.skip.insert(g.events.len());
                    g.emit(EventKind::Activated { reference: target, by: player, outcome: serde_json::to_value(&a).ok() });
                    if let Outcome::PickedUp { item, count, .. } = a.outcome {
                        self.skip.insert(g.events.len());
                        g.emit(EventKind::ItemAdded { to: player, item, count });
                        g.item_event(player, item, event::ON_ADD, Some(Value::Ref(player.0)));
                    }
                    // Through a load door: the player's teammates come along
                    // (`fnv_game::followers`).
                    if matches!(a.outcome, Outcome::Teleported { .. }) {
                        let brought = g.bring_teammates();
                        if !brought.is_empty() {
                            info!("script: {} teammate(s) come along through the door", brought.len());
                        }
                    }
                    // A script without OnActivate does not run on activation; the
                    // default action was all there is.
                    a.script = None;
                    Ok(ByHand::Default(a))
                }
                Err(e) => Err(e.to_string()),
            }
        };
        // An actor whose default action ran (no OnActivate block, or its block called
        // `Activate`) is talked to, if it has a greeting.
        let r = match r {
            Ok(done) => Ok(self.talk_after(from).unwrap_or(done)),
            r => r,
        };
        self.snapshot_inventory();
        r
    }

    /// The player's activations among the events from index `from` on whose default
    /// action is to talk (`Outcome::Talk`: a living actor): the conversation starts with
    /// the actor's greeting (`Game::start_dialogue`). An actor with nothing to say starts
    /// none, and the activation keeps the on-screen line it had ("… has nothing to say").
    /// An actor whose greeting has the Goodbye flag says it at the player and starts none
    /// either (the runtime's rule, docs/spec-notes/dialogue.md §5): the line is heard and
    /// shown like a SayTo line and the player stays free. Returns [`ByHand::Talk`] when a
    /// conversation started, else [`ByHand::Greeted`] when a greeting was said.
    fn talk_after(&mut self, from: usize) -> Option<ByHand> {
        let player = FormId::PLAYER_REF;
        let wanted: Vec<(usize, FormId)> = self
            .game
            .events
            .iter()
            .enumerate()
            .skip(from)
            .filter_map(|(i, e)| match &e.kind {
                EventKind::Activated { by, outcome: Some(o), .. } if *by == player => match script_events::activation_from_json(o)?.outcome {
                    Outcome::Talk { actor } => Some((i, actor)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let mut done = None;
        for (i, actor) in wanted {
            let g = &mut self.game;
            match g.start_dialogue(actor, None) {
                Ok(info) => {
                    // The activation's own effect (the "nothing to say" line) is void.
                    self.skip.insert(i);
                    if g.dialogue.is_some() {
                        info!("dialogue: the player talks to {}: its greeting is {}", g.data.name(actor), g.data.name(info));
                        done = Some(ByHand::Talk);
                    } else {
                        info!("dialogue: {} says its greeting {} at the player (a Goodbye line: no conversation)", g.data.name(actor), g.data.name(info));
                        done = done.or(Some(ByHand::Greeted));
                    }
                }
                Err(e) => {
                    // Unconscious: the corner says so instead of "nothing to say"
                    // (`EventKind::TalkedToUnconscious`, docs/spec-notes/unconscious.md §3).
                    if g.is_out_cold(actor) {
                        self.skip.insert(i);
                    }
                    info!("dialogue: no conversation: {e}");
                }
            }
        }
        done
    }

    /// The Sleep menu opens for the player lying on `bed`: a bed with an OnActivate
    /// block is activated by the player again, without its default action (the player
    /// is on it already). Returns whether a block ran.
    ///
    /// [engineering choice] The game's bed script (FalloutNV.esm SCPT 0x000B5431) notes
    /// the activation in a variable that its next GameMode frame clears, and gives Well
    /// Rested only if it sees the player sleeping in a menu before that: in the original
    /// the menu must therefore open with the activation [hypothesis]. In NVForever the
    /// player lies down first and the menu opens seconds later, so the activation is
    /// told again when it does (docs/spec-notes/wait-menu.md §8).
    pub(crate) fn bed_activated_for_sleep(&mut self, bed: FormId) -> bool {
        let g = &mut self.game;
        if !g.ref_script(bed).and_then(|s| g.data.script(s)).is_some_and(|u| u.has_block(event::ON_ACTIVATE)) {
            return false;
        }
        let from = g.events.len();
        if let Err(e) = g.activate(FormId::PLAYER_REF, bed, ActMode::Hand) {
            warn!("wait menu: the bed's OnActivate block: {e}");
        }
        let again = g.events.iter().enumerate().skip(from).filter(|(_, e)| matches!(&e.kind, EventKind::Activated { reference, .. } if *reference == bed)).map(|(i, _)| i);
        self.skip.extend(again);
        true
    }

    /// Something the player does to the game between ticks (a choice on the dialogue
    /// screen, a skipped response): inventory changes the menus made are seen first, and
    /// what the action's scripts change is the scripts' own.
    pub(crate) fn player_action<R>(&mut self, f: impl FnOnce(&mut Game<'static>) -> R) -> R {
        self.observe_inventory();
        let r = f(&mut self.game);
        self.snapshot_inventory();
        r
    }

    /// The player activates `r` as E does, wherever the crosshair is (test aids). Nobody
    /// else applies a default action's outcome here: its trace events become effects like
    /// a script's.
    pub(crate) fn activate_as_player(&mut self, r: FormId) -> Result<ByHand, String> {
        let from = self.game.events.len();
        let done = self.by_hand(r)?;
        if !matches!(done, ByHand::Talk | ByHand::Greeted) {
            self.skip.retain(|&i| i < from);
        }
        Ok(done)
    }

    /// One test-aid step (`walk:` is the caller's: it needs physics).
    fn aid(&mut self, a: &ScriptAid) -> Result<(), String> {
        use fnv_game::runner::resolve;
        let g = &mut self.game;
        match a {
            ScriptAid::Start(q) => {
                let q = resolve(g, q)?;
                g.start_quest(q);
            }
            ScriptAid::Combat(a, t) => {
                let (a, t) = (resolve(g, a)?, resolve(g, t)?);
                g.start_combat(a, t);
            }
            ScriptAid::MoveTo(r, m, off) => {
                let (r, m) = (resolve(g, r)?, resolve(g, m)?);
                let ctx = fnv_game::functions::Ctx { unit: "test aid".into(), opcode: MOVE_TO, on: Some(r), self_ref: Some(r), action_ref: None, quest: None, inst: None, in_condition: false };
                let args: Vec<ArgValue> = std::iter::once(ArgValue::Value(Value::Ref(m.0))).chain(off.iter().map(|v| ArgValue::Value(Value::Num(f64::from(*v))))).collect();
                g.call_function(&ctx, &args);
            }
            ScriptAid::Stop(q) => {
                let q = resolve(g, q)?;
                g.stop_quest(q);
            }
            ScriptAid::Stage(q, s) => {
                let q = resolve(g, q)?;
                g.set_stage(q, *s);
            }
            ScriptAid::Objective(q, i) => {
                let q = resolve(g, q)?;
                g.set_objective(q, *i, Some(true), None);
            }
            ScriptAid::ObjectiveDone(q, i) => {
                let q = resolve(g, q)?;
                g.set_objective(q, *i, None, Some(true));
            }
            ScriptAid::Info(i, speaker) => {
                let i = resolve(g, i)?;
                let sp = speaker.as_deref().map(|s| resolve(g, s)).transpose()?;
                g.run_info(i, sp, true)?;
            }
            ScriptAid::Button(b) => {
                if !self.answer(Some(*b)) {
                    return Err("no script menu or message box to answer with that button".into());
                }
            }
            ScriptAid::Close => {
                if !self.answer(None) {
                    return Err("no script menu or message box is open".into());
                }
            }
            ScriptAid::Message(m) => {
                let m = resolve(g, m)?;
                let def = g.data.messages.get(&m).ok_or_else(|| format!("{} is not a message", g.data.name(m)))?;
                let (text, buttons, is_box) = (def.text.clone(), def.buttons.clone(), def.message_box);
                let hidden = g.hidden_buttons(m, None);
                g.emit(EventKind::MessageShown { message: m, text, buttons: buttons.clone(), hidden });
                if is_box || !buttons.is_empty() {
                    g.show_box(m);
                }
            }
            ScriptAid::Menu(code) => g.open_menu(*code),
            ScriptAid::Activate(r) => {
                let r = resolve(g, r)?;
                if let ByHand::Default(a) = self.activate_as_player(r)? {
                    debug!("test aid: activated {r}: {:?}", a.outcome);
                }
            }
            ScriptAid::Kill(r) => {
                let r = resolve(g, r)?;
                if !g.kill(r, Some(FormId::PLAYER_REF)) {
                    return Err(format!("{r} is dead already"));
                }
                info!("test aid: the player killed {r}");
            }
            ScriptAid::Enable(r) => {
                let r = resolve(g, r)?;
                let ctx = fnv_game::functions::Ctx { unit: "test aid".into(), opcode: 0x1021, on: Some(r), self_ref: None, action_ref: None, quest: None, inst: None, in_condition: false };
                g.call_function(&ctx, &[]);
            }
            ScriptAid::SayTo(speaker, target, topic, no_look) => {
                let (speaker, target, topic) = (resolve(g, speaker)?, resolve(g, target)?, resolve(g, topic)?);
                let ctx = fnv_game::functions::Ctx { unit: "test aid".into(), opcode: SAY_TO, on: Some(speaker), self_ref: Some(speaker), action_ref: None, quest: None, inst: None, in_condition: false };
                let flag = if *no_look { 1.0 } else { 0.0 };
                g.call_function(&ctx, &[ArgValue::Value(Value::Ref(target.0)), ArgValue::Value(Value::Ref(topic.0)), ArgValue::Value(Value::Num(0.0)), ArgValue::Value(Value::Num(flag))]);
                if !g.speech.iter().any(|s| s.speaker == speaker && s.target == Some(target)) {
                    return Err(format!("{} has no line of {} to say to {}", g.data.name(speaker), g.data.name(topic), g.data.name(target)));
                }
            }
            ScriptAid::PlayGroup(r, group, flag) => {
                let r = resolve(g, r)?;
                let code = match group.parse::<u16>() {
                    Ok(c) => c,
                    Err(_) => (0..=u16::from(u8::MAX))
                        .find(|c| g.anim_group_name(*c).is_some_and(|n| n.eq_ignore_ascii_case(group)))
                        .ok_or_else(|| format!("{group} is not an animation group"))?,
                };
                let ctx = fnv_game::functions::Ctx { unit: "test aid".into(), opcode: PLAY_GROUP, on: Some(r), self_ref: Some(r), action_ref: None, quest: None, inst: None, in_condition: false };
                g.call_function(&ctx, &[ArgValue::Enum(code), ArgValue::Value(Value::Num(f64::from(*flag)))]);
            }
            ScriptAid::PlayIdle(r, idle) => {
                let (r, idle) = (resolve(g, r)?, resolve(g, idle)?);
                if !g.ask_idle(r, idle, fnv_game::idles::IdleSource::Script) {
                    return Err(format!("{} does not play {}", g.data.name(r), g.data.name(idle)));
                }
            }
            ScriptAid::Call(on, func, args) => {
                let opcode = g.data.table.by_name(func).ok_or_else(|| format!("{func} is not a script function"))?;
                let on = if on == "-" { None } else { Some(resolve(g, on)?) };
                let mut values = Vec::new();
                for a in args {
                    values.push(match a.parse::<f64>() {
                        Ok(n) => ArgValue::Value(Value::Num(n)),
                        // An axis (Rotate, SetAngle): X, Y or Z.
                        Err(_) if matches!(a.as_str(), "x" | "y" | "z" | "X" | "Y" | "Z") => ArgValue::Axis(a.chars().next().unwrap_or('Z').to_ascii_uppercase()),
                        // A string ("…" in the aid; no comma or colon inside).
                        Err(_) if a.len() >= 2 && a.starts_with('"') && a.ends_with('"') => ArgValue::Str(a[1..a.len() - 1].to_string()),
                        Err(_) => ArgValue::Value(Value::Ref(resolve(g, a)?.0)),
                    });
                }
                let ctx = fnv_game::functions::Ctx { unit: "test aid".into(), opcode, on, self_ref: on, action_ref: None, quest: None, inst: None, in_condition: false };
                let from = g.events.len();
                let value = g.call_function(&ctx, &values);
                if let Some(EventKind::CallFailed { error, .. }) = g.events[from..].iter().map(|e| &e.kind).find(|k| matches!(k, EventKind::CallFailed { .. })) {
                    return Err(format!("{func}: {error}"));
                }
                // What it returned, for the smoke checks of what functions answer.
                let value = match value {
                    Value::Num(x) => format!("{x}"),
                    Value::Ref(f) => format!("{f:08X}"),
                };
                info!("script: test aid calls {func}{} = {value}", on.map(|r| format!(" on {}", g.data.name(r))).unwrap_or_default());
            }
            ScriptAid::Damage(r, value, amount) => {
                let r = resolve(g, r)?;
                let code = fnv_game::functions::av_code(g, value)?;
                fnv_game::functions::add_actor_value_damage(g, r, code, *amount);
                let now = fnv_game::functions::actor_value(g, r, code)?;
                info!("test aid: {} {value} damaged by {amount}: now {now:.1}", g.data.name(r));
            }
            ScriptAid::SetAv(r, value, v) => {
                let r = resolve(g, r)?;
                let code = fnv_game::functions::av_code(g, value)?;
                fnv_game::functions::set_actor_value_current(g, r, code, *v)?;
                info!("test aid: {} {value} set to {v}", g.data.name(r));
            }
            ScriptAid::Hardcore(on) => {
                g.player.hardcore = Some(*on);
                info!("test aid: Hardcore mode {}", if *on { "on" } else { "off" });
            }
            ScriptAid::Perk(p) => {
                let p = resolve(g, p)?;
                if !g.data.perks.contains_key(&p) {
                    return Err(format!("{} is not a perk", g.data.name(p)));
                }
                *g.player.perks.entry(p).or_default() += 1;
                info!("test aid: the player has the perk {}", g.data.name(p));
            }
            ScriptAid::Walk(_) => unreachable!("handled by the caller"),
            ScriptAid::Var(of, var, value) => {
                let f = resolve(g, of)?;
                let (key, script) = g.instance_of(f).ok_or_else(|| format!("{of} runs no script"))?;
                let index = g.data.script(script).and_then(|c| c.var_index(var)).ok_or_else(|| format!("{of} has no variable {var}"))?;
                g.store(key, Some(script), index, Value::Num(*value), "test aid", false);
                if let InstKey::Quest { quest } = key {
                    debug!("script: test aid set {}.{var} = {value}", g.data.name(quest));
                }
            }
        }
        Ok(())
    }
}

/// What an activation by hand did.
pub enum ByHand {
    /// The target's OnActivate block ran instead of the default action (its effects
    /// arrive with the frame's script events).
    Script,
    /// The default action ran (no OnActivate block, or scripts are off).
    Default(Activation),
    /// The target is an actor with a greeting: a conversation with it started
    /// (`crate::dialogue` shows it).
    Talk,
    /// The target is an actor whose greeting has the Goodbye flag: it said the line at
    /// the player and no conversation started (`Runtime::talk_after`).
    Greeted,
}

/// The player activates `target` by hand (E): through the scripts when they run.
pub fn activate_by_hand(script: Option<&mut ScriptState>, sim: &mut SimState, target: u32) -> Result<ByHand, String> {
    match script.and_then(|s| s.rt.as_mut()) {
        Some(rt) => with_world(rt, sim, |rt| rt.by_hand(FormId(target))),
        None => sim.world.activate(FormId::PLAYER_REF, FormId(target)).map(ByHand::Default).map_err(|e| e.to_string()),
    }
}

// ---- effects context -----------------------------------------------------------------

struct PlayCtx<'a> {
    sim: &'a SimState,
    data: &'a GameData,
    index: &'a WorldIndex,
    children: &'a HashMap<FormId, Vec<FormId>>,
    quests: &'a std::collections::BTreeMap<FormId, fnv_game::quest::QuestState>,
}

impl PlayCtx<'_> {
    fn display(&self, form: FormId) -> String {
        let base = self.sim.base_of(form.0).unwrap_or(form);
        self.sim.name(base).map(str::to_string).unwrap_or_else(|| self.data.name(form))
    }
}

impl EffectCtx for PlayCtx<'_> {
    fn name(&self, form: FormId) -> String {
        self.display(form)
    }

    fn setting_text(&self, name: &str) -> Option<String> {
        match self.sim.forms.settings.get(&name.to_ascii_lowercase()) {
            Some(fnv_sim::settings::Setting::Str(s)) => Some(s.clone()),
            _ => None,
        }
    }

    fn objective_text(&self, quest: FormId, index: i32) -> Option<String> {
        self.data.quests.get(&quest)?.objectives.get(&index)?.text.clone()
    }

    fn quest_title(&self, quest: FormId) -> Option<String> {
        self.data.quests.get(&quest)?.name.clone().filter(|n| !n.trim().is_empty())
    }

    fn quest_listed(&self, quest: FormId, besides: &[i32]) -> bool {
        let Some(q) = self.quests.get(&quest) else { return false };
        q.objectives.iter().any(|(i, o)| (o.displayed || o.completed) && !besides.contains(i))
    }

    fn is_message_box(&self, message: FormId) -> bool {
        self.data.messages.get(&message).is_some_and(|m| m.message_box)
    }

    fn message_look(&self, message: FormId) -> (Option<String>, Option<f32>) {
        let Some(m) = self.data.messages.get(&message) else { return (None, None) };
        (m.icon.and_then(|i| self.data.menu_icons.get(&i).cloned()), m.display_seconds.map(|s| s as f32))
    }

    fn enable_children(&self, r: FormId) -> Vec<FormId> {
        let mut out = Vec::new();
        let mut todo = vec![r];
        let mut seen = BTreeSet::from([r]);
        while let Some(p) = todo.pop() {
            for &c in self.children.get(&p).map_or(&[][..], |v| v.as_slice()) {
                if seen.insert(c) {
                    out.push(c);
                    todo.push(c);
                }
            }
        }
        out
    }

    fn player_heading_deg(&self) -> f32 {
        self.sim.world.peek(FormId::PLAYER_REF).map_or(0.0, |s| s.rot[2].to_degrees().rem_euclid(360.0))
    }

    fn destination(&self, cell: Option<FormId>) -> Destination {
        outcome::destination(self.index, cell)
    }

    fn activation_effects(&self, a: &Activation, target: FormId) -> Vec<Effect> {
        outcome::effects(a, target.0, &self.display(target), self.index, |id| self.display(id))
    }

    fn is_furniture(&self, r: FormId) -> bool {
        self.sim.base_of(r.0).and_then(|b| self.sim.forms.get(b)).is_some_and(|b| matches!(b.data, fnv_sim::forms::BaseData::Furniture))
    }
}

/// The response texts (NAM1) of a dialogue line, in order.
/// The response texts (NAM1) of a dialogue line, in the record's order.
pub(crate) fn responses(play: &PlayWorld, info: FormId) -> Vec<String> {
    let Some((pi, rec)) = fnv_world::fields::winner(play.lo, info.0) else { return Vec::new() };
    let Ok(f) = fnv_world::fields::Fields::new(rec, pi, play.schema) else { return Vec::new() };
    f.subs.iter().filter(|s| &s.sig == b"NAM1").filter_map(|s| f.decode(s)).filter_map(|v| v.as_str().map(str::to_string)).collect()
}

// ---- trace log -------------------------------------------------------------------------

struct TraceLog {
    out: std::io::BufWriter<std::fs::File>,
    path: PathBuf,
    lines: u64,
}

impl TraceLog {
    fn create(path: &std::path::Path) -> std::io::Result<TraceLog> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        let head = serde_json::json!({"format": "nvforever.play.script_log/1",
            "events": "one fnv.script.trace/1 event per line (frame, kind, ...), with t = seconds since scripts started and gt = game time (Game::time); aid, menu_answer and controls lines; a final line at exit"});
        writeln!(out, "{head}")?;
        Ok(TraceLog { out, path: path.to_path_buf(), lines: 0 })
    }

    fn write(&mut self, v: &serde_json::Value) {
        if writeln!(self.out, "{v}").is_ok() {
            self.lines += 1;
        }
    }
}

// ---- plugin and systems --------------------------------------------------------------

pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        let d = app.world().resource::<PlayWorld>().0;
        let refs = app.world().resource::<SimState>().refs;
        let new_game = app.world().contains_resource::<NewGame>();
        let unattended = crate::test_aid::Unattended::from_env().0;
        let no_messages = app.world().contains_resource::<NoMessages>();
        app.insert_resource(ScriptState::new(d, refs, new_game, unattended, no_messages))
            .init_resource::<ClockScale>()
            .add_plugins((crate::script_hud::ScriptHudPlugin, crate::script_draw::ScriptDrawPlugin, crate::dialogue::DialoguePlugin))
            .add_systems(Update, (take_pending_game, crate::script_hud::menu_keys, run_scripts, sync_locks, walk_player, hold_player, time_multiplier).chain().in_set(PlaySet::Scripts))
            .add_systems(Last, (crate::aid::report, write_final));
    }
}

/// A save handed over by `crate::save` waits in the script state until the runtime is
/// made.
fn take_pending_game(world: &mut bevy::prelude::World) {
    if let Some(p) = world.remove_resource::<PendingGame>() {
        world.resource_mut::<ScriptState>().pending_game = Some(p);
    }
}

/// SetGlobalTimeMultiplier (`UiRequests::time_multiplier`): the virtual clock runs at that
/// speed, so everything driven by `Time` (animations, physics, NPCs, the scripts' frame
/// time, the game clock) slows down or speeds up together [documented: GECK wiki
/// SetGlobalTimeMultiplier, "0.5 is slow mo"]. Sounds keep their speed and pitch
/// [engineering choice: the shipped slow motion asks for that, QUST 0x00102037 stage 27].
/// Pausing (loading screens, menus) is the clock's own state and is not touched. The
/// engine's own slow motion ([`ClockScale`]) multiplies it, and so does VATS mode's
/// (`crate::vats_mode`, docs/design/vats-mode.md §4).
fn time_multiplier(
    script: Res<ScriptState>,
    scale: Option<Res<ClockScale>>,
    vats: Option<Res<crate::vats_mode::VatsMode>>,
    mut clock: ResMut<Time<Virtual>>,
    mut told: Local<Option<f32>>,
) {
    let Some(rt) = script.rt.as_ref() else { return };
    let asked = rt.game.ui.time_multiplier.unwrap_or(1.0) as f32;
    let before = told.replace(asked).unwrap_or(1.0);
    if before != asked {
        info!("time multiplier: {before} -> {asked} (SetGlobalTimeMultiplier)");
    }
    let want = asked * scale.map_or(1.0, |s| s.0.max(0.0)) * vats.map_or(1.0, |v| v.time_scale());
    if (clock.relative_speed() - want).abs() > 1e-6 {
        clock.set_relative_speed(want);
    }
}

/// The engine's own slow motion on top of SetGlobalTimeMultiplier (1 for none): the
/// death camera slows the game while the player dies (`nvforever` `death_cam.rs`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ClockScale(pub f32);

impl Default for ClockScale {
    fn default() -> Self {
        ClockScale(1.0)
    }
}

/// The test aids due now: timed ones, and gated ones `at` seconds after their stage was
/// first seen done.
fn due_aids(st: &mut ScriptState, now: f32) -> Vec<ScriptAid> {
    let Some(rt) = st.rt.as_ref() else { return Vec::new() };
    // The first frame the scripts run: steps already due then (the script data took
    // long to load) keep their distances instead of all running in this frame.
    if !std::mem::replace(&mut st.aids_started, true) {
        if let Some(late) = script_events::delay_late_aids(&mut st.aids, now) {
            info!("test aid: the scripts run from {now:.1} s on, after the first script step's time: the timed script steps come {late:.1} s later, as far apart as asked");
        }
    }
    let g = &rt.game;
    for a in &st.aids {
        if let Some(gate) = &a.after {
            if !st.gates.contains_key(gate) {
                let done = fnv_game::runner::resolve(g, &gate.0).ok().is_some_and(|q| g.quests.get(&q).is_some_and(|s| s.stages_done.contains(&gate.1)));
                if done {
                    st.gates.insert(gate.clone(), now);
                }
            }
        }
    }
    let gates = &st.gates;
    let mut due = Vec::new();
    st.aids.retain(|a| {
        let start = match &a.after {
            None => Some(0.0),
            Some(gate) => gates.get(gate).copied(),
        };
        if start.is_some_and(|s| now >= s + a.at) {
            due.push(a.aid.clone());
            false
        } else {
            true
        }
    });
    due
}

/// One frame of scripts, then their effects.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_scripts(
    time: crate::pause::AidTime,
    play: Res<PlayWorld>,
    phys: Res<PhysState>,
    location: Res<Location>,
    mut sim: ResMut<SimState>,
    mut script: ResMut<ScriptState>,
    mut hud: ResMut<ScriptHud>,
    mut corner: ResMut<crate::corner_messages::CornerMessages>,
    mut quest_lines: ResMut<crate::quest_messages::QuestMessages>,
    mut changed: MessageWriter<RefChanged>,
    mut travel: MessageWriter<TravelRequest>,
    mut open: MessageWriter<OpenContainer>,
    mut used: MessageReader<ItemUsed>,
    mut sit: MessageWriter<crate::furniture::UseFurniture>,
    paused: Res<crate::pause::Paused>,
    (loading, bindings): (Res<crate::loading::Loading>, Option<Res<crate::bindings::Bindings>>),
) {
    // Items the player used up in the inventory menu (the menu already removed them),
    // kept until the runtime runs.
    let st = &mut *script;
    st.used.extend(used.read().map(|u| FormId(u.item)));
    if st.rt.is_none() {
        let Some(rx) = st.loader.as_ref() else { return };
        let got = rx.lock().ok().and_then(|r| r.try_recv().ok());
        match got {
            None => return,
            Some(Err(e)) => {
                warn!("scripts are off: {e}");
                st.off = Some(e);
                st.loader = None;
                return;
            }
            Some(Ok((data, secs))) => {
                st.loader = None;
                let mut rt = Runtime::new(data, &mut sim, st.new_game, play.0.packages);
                if let Some(PendingGame(snap, path)) = st.pending_game.take() {
                    match rt.restore_save(&mut sim, *snap) {
                        Ok(changed_refs) => {
                            info!("load: restored {}: {} changed references, {} quests, {} script instances", path.display(), changed_refs.len(), rt.game.quests.len(), rt.game.instances.len());
                            // The drawn world follows the restored simulation.
                            for id in changed_refs {
                                changed.write(RefChanged { form_id: id.0 });
                            }
                        }
                        Err(e) => warn!("load: {}: {e}; the game goes on from the plugins", path.display()),
                    }
                }
                let running = rt.game.quests.values().filter(|q| q.running).count();
                info!(
                    "scripts ready in {secs:.2} s: {} units, {} scripts, {} quests ({running} start-game-enabled quests running)",
                    data.units.len(),
                    data.scripts.len(),
                    data.quests.len()
                );
                st.rt = Some(rt);
            }
        }
    }
    // Scripts wait until the player stands in the world (its cell is known), and do not
    // run under the pause menu (`crate::pause`): no MenuMode code of the original's pause
    // menu is known, so script time simply stops there [decision]. Nor do they run under
    // a loading screen (`crate::loading`) [observed: loading-screen time does not count,
    // layer5-captures.md §8.3 e].
    if paused.0 || loading.active() {
        return;
    }
    let Some(character) = phys.player.as_ref() else { return };
    let player_capsule = Some(Capsule { feet: character.feet, radius: character.params.radius, height: character.params.height });
    // The aids' clock, which goes on while a menu holds the picture (`crate::pause`).
    let now = time.elapsed_secs();
    let mut due = due_aids(st, now);
    let used = std::mem::take(&mut st.used);
    let console = std::mem::take(&mut st.console);
    let mut console_out = Vec::new();
    // A script's menu or box left open by an unattended run: answered with OK / Done.
    let on_screen = st.rt.as_ref().and_then(|rt| {
        let m = rt.game.menu.filter(|&m| is_script_menu(m))?;
        Some((m, if m == MENU_MESSAGE { rt.game.boxes.first().copied() } else { None }))
    });
    match (on_screen, st.on_screen) {
        (Some((m, f)), Some((m0, f0, _))) if (m, f) == (m0, f0) => {}
        (Some((m, f)), _) => st.on_screen = Some((m, f, now)),
        (None, _) => st.on_screen = None,
    }
    // A reputation's box (`crate::reputation`) is the engine's, not a plugin's message.
    let reputation_box = |st: &ScriptState, f: FormId| st.rt.as_ref().is_some_and(|rt| crate::reputation::is_reputation(&rt.game, f));
    if let (true, Some((MENU_MESSAGE, Some(front), _))) = (st.no_messages, st.on_screen.filter(|o| !o.1.is_some_and(|f| reputation_box(st, f)))) {
        let buttons = st.rt.as_ref().and_then(|rt| rt.game.data.messages.get(&front)).map_or(0, |m| m.buttons.len());
        if buttons <= 1 && st.answer(None, "--no-messages") {
            info!("--no-messages: closed message box {front}");
            st.on_screen = None;
        }
    }
    if let (Some(limit), Some((m, _, since))) = (st.auto_close, st.on_screen) {
        if now - since >= limit && st.answer(None, "test aid NVF_PLAY_TEST_SCRIPT_MENUS") {
            info!("test aid: closed menu {m} after {:.2} s", now - since);
            // The next box may show the same message: its time starts when it opens.
            st.on_screen = None;
        }
    }
    let Some(rt) = st.rt.as_mut() else { return };
    // Loading-screen time does not count [observed: layer5-captures.md §8.3 e]: the
    // runtime runs no script under a loading screen (`game.loading`): here, the two
    // frames after the player's place changed (the destination loads on the spot in one
    // of them). docs/spec-notes/scripts.md §7.2. Menus are the runtime's own
    // (`Game::menu`, opened by scripts and by the engine's menus).
    if rt.location.is_some_and(|l| l != *location) {
        rt.frozen_frames = 2;
    }
    rt.location = Some(*location);
    let loading = rt.frozen_frames > 0;
    rt.frozen_frames = rt.frozen_frames.saturating_sub(1);
    // The frame's time; real time while the virtual clock stands still under a menu
    // (`--menu-pause original`), so MenuMode scripts and a conversation's lines go on.
    let dt = time.delta_secs_f64();
    let t0 = Instant::now();
    let mut marked = Vec::new();
    let mut aid_lines = Vec::new();
    let mut walks = Vec::new();
    with_world(rt, &mut sim, |rt| {
        rt.observe_inventory();
        let aid_from = rt.game.events.len();
        for &item in &used {
            let r = crate::aid::use_item(rt, item);
            info!("script: the player used {item}: {} effect(s) started", r.effects.len());
        }
        rt.follow_player();
        if std::mem::take(&mut rt.opening_pending) {
            rt.start_opening();
        }
        for a in &due {
            let r = match a {
                ScriptAid::Walk(target) => fnv_game::runner::resolve(&rt.game, target).and_then(|r| {
                    let pos = rt.game.world.peek(r).map(|s| s.pos).or_else(|| rt.game.world.placed.get(r).map(|p| p.pos));
                    let pos = pos.ok_or_else(|| format!("{target} has no position"))?;
                    walks.push((target.clone(), pos));
                    Ok(())
                }),
                a => rt.aid(a),
            };
            match &r {
                Ok(()) => info!("test aid: script {a:?}"),
                Err(e) => warn!("test aid: script {a:?}: {e}"),
            }
            aid_lines.push(serde_json::json!({"kind": "aid", "frame": rt.game.frame, "op": format!("{a:?}"), "error": r.err(),
                "t": secs(rt.started), "gt": rt.game.time}));
        }
        due.clear();
        for c in &console {
            console_out.push(crate::console::run_call(rt, c));
        }
        let from = rt.game.events.len();
        // A conversation with the player pauses the game: nobody moves, and no trigger
        // block runs (OnTrigger would, every frame) until it is over.
        if rt.game.dialogue.is_none() {
            crate::script_trigger::update(rt, &play, player_capsule);
        }
        rt.game.loading = loading;
        rt.game.cfg.frame_seconds = dt;
        rt.game.tick();
        crate::aid::after_tick(rt, aid_from);
        // A script's `Activate player` on an actor: the player talks to it.
        rt.talk_after(from);
        rt.finish_answer();
        rt.snapshot_inventory();
        marked = rt.newly_marked();
    });
    st.walks.extend(walks);
    st.console_out.extend(console_out);
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    rt.stats.frames += 1;
    rt.stats.total_ms += ms;
    if ms > rt.stats.worst_ms {
        rt.stats.worst_ms = ms;
        if rt.stats.frames > 1 && ms > 4.0 {
            debug!("scripts took {ms:.1} ms this frame (the longest so far)");
        }
    }
    // Effects of everything since the last frame (this frame's tick and any activation
    // by hand earlier in the frame).
    let events = std::mem::take(&mut rt.game.events);
    let skip = std::mem::take(&mut rt.skip);
    rt.stats.events += events.len() as u64;
    let ctx = PlayCtx { sim: &sim, data: rt.game.data, index: play.index, children: &rt.children, quests: &rt.game.quests };
    let mut effects = script_events::effects(&events, &skip, &ctx);
    for r in marked {
        info!("script: {r} marked for delete");
        effects.push(ScriptEffect::RefChanged(r.0));
        effects.extend(ctx.enable_children(r).into_iter().map(|c| ScriptEffect::RefChanged(c.0)));
    }
    let t = secs(rt.started);
    let gt = rt.game.time;
    for e in &events {
        log_event(rt.game.data, e);
        if let EventKind::LineStarted { info, .. } = &e.kind {
            if !hud.knows_line(*info) {
                hud.remember_line(*info, responses(&play, *info));
            }
        }
    }
    if let Some(log) = st.trace.as_mut() {
        for l in &aid_lines {
            log.write(l);
        }
        for e in &events {
            if let Ok(mut v) = serde_json::to_value(e) {
                v["t"] = serde_json::json!(t);
                v["gt"] = serde_json::json!(gt);
                log.write(&v);
            }
        }
        if !events.is_empty() || !aid_lines.is_empty() {
            let _ = log.out.flush();
        }
    }
    let mut sent = BTreeSet::new();
    let feet = phys.player.as_ref().map(|c| c.feet);
    for e in effects {
        match e {
            ScriptEffect::RefChanged(id) => {
                if sent.insert(id) {
                    changed.write(RefChanged { form_id: id });
                }
            }
            ScriptEffect::Travel(t) => {
                // A move to where the player already stands (VCG00 stage 0's MoveTo to the
                // new game's start) needs no loading.
                let here = t.destination(*location) == *location
                    && feet.is_some_and(|f| (f[0] - t.feet[0]).hypot(f[1] - t.feet[1]) < 32.0 && (f[2] - t.feet[2]).abs() < 64.0);
                if here {
                    info!("script: the player is moved to where they stand ({:?}); no travel", t.feet);
                } else {
                    info!("script: travel to {:?} / {:?} at {:?}", t.interior, t.world, t.feet);
                    travel.write(TravelRequest(t));
                }
            }
            ScriptEffect::OpenContainer(c) => {
                open.write(OpenContainer { container: c });
            }
            ScriptEffect::Notify(mut note) => {
                // Key strings (`&sUActnVats;`) name the keys bound now.
                if let Some(b) = bindings.as_deref() {
                    note.text = crate::lockpick::help::expand_keys(&note.text, b);
                }
                info!("on screen: {}", note.text.trim().replace('\n', " / "));
                corner.push(note);
            }
            ScriptEffect::Quest(n) => {
                info!("quest message: {}", crate::quest_messages::describe(&n));
                quest_lines.push(n);
            }
            ScriptEffect::MessageBox(mut b) => {
                if let Some(k) = bindings.as_deref() {
                    b.text = crate::lockpick::help::expand_keys(&b.text, k);
                }
                hud.message_box(b)
            }
            ScriptEffect::UseFurniture(reference) => {
                info!("script: the player uses furniture {reference:08X} (a script's Activate)");
                sit.write(crate::furniture::UseFurniture { reference });
            }
        }
    }
}

/// The control groups scripts turned off or on this frame reach the player's controls
/// (`control::ControlLocks`; the next frame's input obeys them), the log and the trace
/// (a `controls` line with the groups now disabled).
fn sync_locks(mut script: ResMut<ScriptState>, mut locks: ResMut<ControlLocks>, mut stats: ResMut<crate::control::LockStats>) {
    let now = script.locks();
    if *locks == now {
        return;
    }
    *locks = now;
    let names = now.names();
    if script.before_opening() {
        info!("player controls: all held until the new game's opening starts");
        return;
    }
    if script.rt.as_ref().is_some_and(|rt| rt.game.is_dead(FormId::PLAYER_REF)) {
        info!("player controls: all held, the player is dead");
        return;
    }
    stats.changes += 1;
    if names.is_empty() {
        info!("player controls: all enabled");
    } else {
        info!("player controls: {} disabled by scripts", names.join(", "));
    }
    script.trace_note(serde_json::json!({"kind": "controls", "disabled": names}));
}

/// `walk:` test aids: the player's feet on the floor under the point (a stand-in for
/// walking there), as `NVF_PLAY_TEST_GOTO` does.
fn walk_player(mut script: ResMut<ScriptState>, mut phys: ResMut<PhysState>, mut player: ResMut<Player>) {
    if script.walks.is_empty() || phys.player.is_none() {
        return;
    }
    for (target, at) in std::mem::take(&mut script.walks) {
        let hit = phys.place_player(at, 60.0, 1000.0);
        let feet = phys.player.as_ref().map_or(at, |c| c.feet);
        player.feet = feet;
        info!("test aid: walk to {target} ({:.0}, {:.0}, {:.0}): feet ({:.1}, {:.1}, {:.1}) on {:?}", at[0], at[1], at[2], feet[0], feet[1], feet[2], hit.map(|h| h.owner));
    }
}

/// The engine's own menus that hold `PlayState::Menu` themselves (`hold_player`).
type EngineMenus<'w> = (
    Res<'w, crate::terminal::TerminalScreen>,
    Res<'w, crate::lockpick::LockpickScreen>,
    Res<'w, crate::console::Console>,
    Option<Res<'w, crate::crafting::RecipeMenu>>,
    Option<Res<'w, crate::roulette::RouletteScreen>>,
    Option<Res<'w, crate::repair::RepairServices>>,
);

/// While a script's menu or message box is open the player is in `PlayState::Menu`: no
/// moving, looking or activating, and the game clock stops (sim.rs) [observed: the pack
/// boxes held the world clock, layer5-captures.md §10.1]. The engine's own menus manage
/// the state themselves (menu.rs).
fn hold_player(
    script: Res<ScriptState>,
    active: Res<ActiveMenu>,
    (paused, wait): (Res<crate::pause::Paused>, Res<crate::wait::WaitMenu>),
    (terminal, lockpick, console, recipe, roulette, repair): EngineMenus,
    (blackjack, poker): (Option<Res<crate::blackjack::BlackjackScreen>>, Res<crate::poker::PokerScreen>),
    state: Res<State<PlayState>>,
    mut next: ResMut<NextState<PlayState>>,
) {
    // The pause menu is also a `PlayState::Menu` without an engine menu: it is its own.
    // So are the wait menu (`crate::wait`), a terminal (`crate::terminal`), the
    // lockpicking menu (`crate::lockpick`), the console (`crate::console`), the recipe
    // menu (`crate::crafting`), the repair services (`crate::repair`), the blackjack table (`crate::blackjack`), the roulette
    // table (`crate::roulette`) and the poker table (`crate::poker`).
    let table = blackjack.is_some_and(|b| b.0.is_some()) || roulette.is_some_and(|r| r.0.is_some()) || poker.0.is_some();
    if active.0.is_some() || paused.0 || wait.is_open() || terminal.0.is_some() || lockpick.0.is_some() || console.open || recipe.is_some_and(|r| r.is_open()) || repair.is_some_and(|r| r.is_open()) || table {
        return;
    }
    let held = script.rt.as_ref().and_then(|rt| rt.game.menu).is_some_and(is_script_menu);
    match (*state.get(), held) {
        (PlayState::Playing, true) => next.set(PlayState::Menu),
        (PlayState::Menu, false) => next.set(PlayState::Playing),
        _ => {}
    }
}

/// The trace events worth a log line.
fn log_event(data: &GameData, e: &Event) {
    let n = |f: FormId| data.name(f);
    match &e.kind {
        EventKind::StageSet { quest, stage, entries } => info!("script: {} stage {stage} set (entries {entries:?})", n(*quest)),
        EventKind::StageIgnored { quest, stage, reason } => debug!("script: {} stage {stage} ignored ({reason})", n(*quest)),
        EventKind::QuestCompleted { quest } => info!("script: quest {} completed", n(*quest)),
        EventKind::QuestStopped { quest } => debug!("script: quest {} stopped", n(*quest)),
        EventKind::ObjectiveDisplayed { quest, index, on } => info!("script: {} objective {index} displayed {on}", n(*quest)),
        EventKind::ObjectiveCompleted { quest, index, on } => info!("script: {} objective {index} completed {on}", n(*quest)),
        EventKind::MessageShown { message, text, buttons, hidden } => {
            let hidden = if hidden.is_empty() { String::new() } else { format!(" (hidden by their conditions: {hidden:?})") };
            info!("script: message {} {:?}{}{hidden}", n(*message), text.as_deref().unwrap_or(""), if buttons.is_empty() { String::new() } else { format!(" buttons {buttons:?}") })
        }
        EventKind::RefEnabled { reference } => info!("script: {} enabled", n(*reference)),
        EventKind::RefDisabled { reference } => info!("script: {} disabled", n(*reference)),
        EventKind::Moved { reference, cell, pos } => info!("script: {} moved to {cell:?} {pos:?}", n(*reference)),
        EventKind::Activated { reference, by, outcome } => {
            info!("script: {} activated by {}: {}", n(*reference), n(*by), outcome.as_ref().map_or_else(String::new, |o| o.to_string()))
        }
        EventKind::BlockRun { unit, block, on } if block != "OnTrigger" => info!("script: {block} of {unit} ran on {on:?}"),
        EventKind::MenuOpened { menu } => info!("script: menu {menu} open"),
        EventKind::MenuClosed { menu } => info!("script: menu {menu} closed"),
        EventKind::LineStarted { speaker, info, seconds, .. } => info!("script: {} says {} ({seconds:.2} s)", n(*speaker), n(*info)),
        EventKind::LineEnded { speaker, info, cut } => info!("script: {} {} line {}", n(*speaker), if *cut { "cut" } else { "ended" }, n(*info)),
        EventKind::ItemAdded { to, item, count } => debug!("script: {count} x {} added to {}", n(*item), n(*to)),
        EventKind::ItemRemoved { from, item, count } => debug!("script: {count} x {} removed from {}", n(*item), n(*from)),
        EventKind::Unhandled { name, unit, .. } => debug!("script: unhandled {name} in {unit}"),
        EventKind::ScriptError { unit, error } => warn!("script error in {unit}: {error}"),
        EventKind::CallFailed { name, unit, error, .. } => debug!("script: {name} failed in {unit}: {error}"),
        _ => crate::dialogue::log_event(data, e),
    }
}

/// At exit: the trace's final line and a summary in the log.
fn write_final(mut exit: MessageReader<AppExit>, mut script: ResMut<ScriptState>) {
    if exit.read().next().is_none() {
        return;
    }
    let st = &mut *script;
    let Some(rt) = st.rt.as_ref() else {
        info!("scripts: never ran ({})", st.off.as_deref().unwrap_or("not ready before exit"));
        return;
    };
    let g = &rt.game;
    let s = rt.stats;
    let name = |op: &u16| g.data.table.get(*op).map(|f| f.name.clone()).unwrap_or_default();
    let counts = |m: &std::collections::BTreeMap<u16, u64>| -> serde_json::Map<String, serde_json::Value> {
        m.iter().map(|(op, n)| (format!("0x{op:04X}"), serde_json::json!({"name": name(op), "calls": n}))).collect()
    };
    // Quests something happened to (most of the hundreds started at the new game only run).
    let quests: serde_json::Map<String, serde_json::Value> = g
        .quests
        .iter()
        .filter(|(_, q)| !q.stages_done.is_empty() || q.completed || !q.objectives.is_empty() || !q.running)
        .map(|(id, q)| {
            let objectives: Vec<serde_json::Value> =
                q.objectives.iter().map(|(i, o)| serde_json::json!({"index": i, "displayed": o.displayed, "completed": o.completed})).collect();
            (
                id.to_string(),
                serde_json::json!({"edid": g.data.edid_of.get(id), "running": q.running, "completed": q.completed,
                    "stage": q.stage(), "stages_done": q.stages_done, "objectives": objectives}),
            )
        })
        .collect();
    let mean = if s.frames > 0 { s.total_ms / s.frames as f64 } else { 0.0 };
    info!(
        "scripts: {} frames, {:.3} ms per frame on average, longest {:.1} ms; {} events; {} quests running; menu {:?}, {} boxes waiting; unhandled calls {}, stubbed {}",
        s.frames,
        mean,
        s.worst_ms,
        s.events,
        g.quests.values().filter(|q| q.running).count(),
        g.menu,
        g.boxes.len(),
        g.unhandled.values().sum::<u64>(),
        g.stubbed.values().sum::<u64>()
    );
    if let Some(log) = st.trace.as_mut() {
        let v = serde_json::json!({"kind": "final", "frame": g.frame, "game_seconds": g.time, "frames": s.frames,
            "tick_ms": {"mean": mean, "worst": s.worst_ms}, "events": s.events,
            "quests_running": g.quests.values().filter(|q| q.running).count(), "menu": g.menu, "boxes": g.boxes,
            "quests": quests, "player": g.player, "unhandled": counts(&g.unhandled), "stubbed": counts(&g.stubbed)});
        log.write(&v);
        let _ = log.out.flush();
        info!("script trace: {} lines in {}", log.lines, log.path.display());
    }
}
