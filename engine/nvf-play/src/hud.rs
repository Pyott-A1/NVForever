//! The HUD of the original: health and compass at the bottom left, action points,
//! weapon condition and ammo at the bottom right (docs/spec-notes/hud.md).
//!
//! The pieces, their sizes and their textures are the game's own interface files'
//! (§2, §3); where they stand is left to the game's code by those files and is set here
//! (§4). Lengths are in the interface's units, 960 to the height of the view.
//!
//! - [`gather`] reads what the HUD shows from the game: the player's health and action
//!   points (the script runtime's actor values), the heading, the equipped weapon's
//!   condition and ammo (`combat`).
//! - [`Places`] sets the pieces out (pure, tested without the game's files).
//! - The pieces are spawned once; after that only the bars' lengths, the compass strip's
//!   place and the ammo's digits change.
//! - Without the game's textures the bars are plain blocks and the brackets plain lines.
//! - The enemy health bar (bottom middle, `crate::hud_target` says whose) and the sneak
//!   word (top middle, while sneaking) are parts of the same HUD (§8, §9).
//! - A bar's pips are set on whole pixels, all as wide as each other, whatever the
//!   view's size (§4).

use crate::combat::{Combat, HudWeapon};
use crate::control::ControlLocks;
use crate::hud_target::HudTarget;
use crate::player::Player;
use crate::reticle::crime_colour;
use crate::script::{with_world, ScriptState};
use crate::sim::SimState;
use crate::interface::{amber, at, bc_supported, face, plain_font, set_line, texture, u, Face};
use crate::{PlaySet, PlayState};
use bevy::image::CompressedImageFormatSupport;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use fnv_sim::FormId;
use fnv_font::Font;
use fnv_game::actor_values::ACTION_POINTS;
use fnv_game::awareness::State as Awake;
use fnv_game::detection::Band;
use nvf_assets::GameData;

// ---- the interface files' numbers [documented: hud.md §2, §3] -------------------------

/// The two corner pieces (`HitPoints`, `ActionPoints`): width and height.
const HP_PANEL: Vec2 = Vec2::new(369.0, 127.0);
const AP_PANEL: Vec2 = Vec2::new(386.0, 127.0);
/// The bracket pictures' room; the pictures are drawn texel for unit.
const BRACKET: Vec2 = Vec2::new(512.0, 256.0);
/// One tick of a bar: the room it takes and its height. (The file's own length for a full
/// bar, 300, is about the 296 measured, §4.)
const TICK: Vec2 = Vec2::new(8.0, 20.0);
/// The tick picture's block: 6 texels wide, then 2 clear [observed: `hud_tick_mark.dds`,
/// its opacity rising from top to bottom].
const TICK_BLOCK: f32 = 6.0;
/// The compass window in the health piece: place and size.
const COMPASS_AT: Vec2 = Vec2::new(20.0, 65.0);
pub const COMPASS: Vec2 = Vec2::new(345.0, 64.0);
/// The whole strip, north to north: 1024 texels for 360 degrees [observed: the texture].
pub const STRIP: Vec2 = Vec2::new(1024.0, 64.0);
/// The solid bar's ground (weapon condition): length and height [documented: the file
/// puts it at x 90 of a frame the code sets; where it stands is measured, below].
const CND_BAR: Vec2 = Vec2::new(60.0, 20.0);
/// The condition's arrows, `CNDArrows`: a 16 x 16 picture, placed by the code
/// [documented: hud_main_menu.xml]; black arrows at the picture's top and bottom edges
/// pointing in, their middle 3.5 texels from its left [observed: the texture].
const CND_ARROWS: &str = "textures\\interface\\shared\\arrow\\repair_meter_pips.dds";
const ARROWS_MIDDLE: f32 = 3.5 / 16.0;
/// Where the arrows stand on the condition bar: below this condition a weapon may jam
/// [owner's playtest list 5, item 73: "the 75% threshold on condition … when jamming
/// occurs"; observed: the arrows at about three quarters on the owner's screenshot].
pub const JAM_CONDITION: f32 = 0.75;
/// The arrows' picture is drawn this much taller than the bar at the top and the bottom
/// and cut to the bar: the arrows' bases are the picture's first and last rows
/// [observed: the texture], and drawn exactly as high as the bar the filtered edge rows
/// left a strip of the bar showing above and below them (owner's playtest 2026-10-16).
const ARROWS_BLEED: f32 = 1.0;

/// The enemy's health piece, `EnemyHealth`, and its bracket picture's place and size in
/// it (`glow_enemy_health_seperator.dds`) [documented: hud_main_menu.xml,
/// hudtemplates.xml].
const ENEMY: Vec2 = Vec2::new(350.0, 52.0);
const ENEMY_BRACKET_AT: Vec2 = Vec2::new(-7.0, 3.0);
const ENEMY_BRACKET: Vec2 = Vec2::new(512.0, 64.0);
/// The sneak piece, `SneakMeter`: width and height [documented: hud_main_menu.xml].
const SNEAK: Vec2 = Vec2::new(356.0, 34.0);

// ---- what the bracket pictures show [observed: the textures, §3] ----------------------

/// The upper edge of the brackets' long line.
const LINE_Y: f32 = 66.0;
/// The left bracket's upright and the right bracket's two (the end and the divider).
const LEFT_UPRIGHT: f32 = 17.0;
const RIGHT_UPRIGHT: f32 = 369.0;
const DIVIDER: f32 = 207.0;
/// The small corner marks on the long line: where a full bar ends on the left piece and
/// starts on the right one.
const LEFT_MARK: f32 = 320.0;
const RIGHT_MARK: f32 = 70.0;
/// The enemy bracket's long line, taken as `LINE_Y` is on the corner pieces (2 rows into
/// the solid line, rows 21-23; `LINE_Y` is 2 into rows 64-66), and the inner edges of its
/// two corner marks, between which its bar stands [observed: the texture]. At 19 (the glow
/// above the line) the line stood 4 units under the corner pieces' and its ticks 7 pixels
/// over it at 1080 lines, where the original's line is level with the corner pieces'
/// upper solid line and its ticks stand on it [observed: `Target healthbar` screenshot;
/// owner's playtest 2026-10-16].
const ENEMY_LINE_Y: f32 = 23.0;
/// The enemy bar's ticks stand right on its solid line (they end at row 21), not
/// `METER_GAP` over it as the player's bars do [observed: `Target healthbar` screenshot;
/// at `METER_GAP` a 3-pixel strip showed at 1080 lines, owner's playtest 2026-10-16].
const ENEMY_METER_GAP: f32 = 2.0;
const ENEMY_MARKS: (f32, f32) = (56.0, 307.0);

// ---- set here; the files leave these to the game's code [hypothesis, §4] --------------

/// From the view's side and bottom edges to the pieces.
const MARGIN_X: f32 = 40.0;
const MARGIN_Y: f32 = 30.0;
/// The bars stand this far above the long line.
const METER_GAP: f32 = 4.0;
/// Room between a label and what it labels.
const LABEL_GAP: f32 = 6.0;
/// The line the texts of the lower row stand on.
const LOWER_BASE: f32 = 106.0;
/// The ground of the condition bar is this much of the bar's own brightness
/// [hypothesis: as `meter.xml`'s ground, a quarter].
const GROUND: f32 = 0.25;

// ---- measured on the owner's 1920 x 1080 screenshot of the original [observed, §4] -----

/// A full bar: 37 ticks, from just inside the bracket's upright to its corner mark (on
/// the left; mirrored on the right).
const PIPS: usize = 37;
const METER_WIDTH: f32 = PIPS as f32 * TICK.x;
/// The bars stand this far left of where they were set from the bracket alone (the
/// corner mark on the left piece, the upright on the right): the first tick starts at
/// the upright's inside edge, the last ends clear of the mark and of the upright
/// [observed: 3 pixels at 1080 lines against the `Target healthbar` screenshot, owner's
/// playtest 2026-10-16]. The action points bar 1 less: its ticks stood 2 pixels left of
/// the original's at 1080 lines, the last one clear of the upright where the original's
/// meets it [observed: the same screenshot, tick edges 1520 … 1844 against 1518 … 1842,
/// 2026-10-16].
const METER_SHIFT: f32 = 3.0;
const AP_METER_SHIFT: f32 = 2.0;
/// "HP" and "AP" stand above their bars near the uprights: the line they stand on, where
/// "HP" starts and where "AP" ends [observed: as above, "HP" 11 and "AP" 8 pixels left of
/// the earlier 33 and 362].
const UPPER_BASE: f32 = 33.0;
const HP_LABEL_X: f32 = 23.0;
const AP_LABEL_END: f32 = 355.0;
/// The condition bar ends this far before the divider, and its bottom stands this far
/// above the line "CND" stands on; "CND" ends this far before the bar [observed: on the
/// `Target healthbar` screenshot the bar starts about 8 pixels after the "D" and is about
/// as high as the letters; owner's playtest 2026-10-16, after 15 / 0 / 6 left 2 pixels
/// between "D" and the bar and the bar lower than the letters].
const CND_BEFORE_DIVIDER: f32 = 10.0;
const CND_RAISE: f32 = 2.0;
const CND_LABEL_GAP: f32 = 11.0;
/// The enemy bar's pips: as many as fit between the bracket's corner marks.
const ENEMY_PIPS: usize = 31;
/// The enemy's name: the line it stands on in its piece, centred.
const ENEMY_NAME_BASE: f32 = 53.0;
/// The sneak word: the line it stands on, from the view's top, centred.
const SNEAK_BASE: f32 = 51.0;

/// The labels [hypothesis: the game takes them from its settings (`sStats…`); the
/// owner's screenshot shows these words].
const HP: &str = "HP";
const AP: &str = "AP";
const CND: &str = "CND";

/// The font of every text on the HUD: font 7 [documented: `hudtemplates.xml`;
/// Fallout_default.ini `sFontFile_7`].
const FONT: &str = "textures\\fonts\\baked-in_monofonto_large.fnt";

const HUD_TEXTURES: &str = "textures\\interface\\hud\\";

/// What the HUD shows.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct HudView {
    /// The HUD is on the screen: the player is in the world, and no script holds the
    /// player's movement (§5).
    pub shown: bool,
    /// Health and action points, 0 to 1 of their full values.
    pub health: f32,
    pub action_points: f32,
    /// Compass heading in degrees (0 north, 90 east).
    pub heading: f32,
    pub weapon: HudWeapon,
    /// The sneak word, while the player sneaks (§9).
    pub sneak: Option<Sneak>,
}

impl Default for HudView {
    fn default() -> Self {
        HudView { shown: false, health: 1.0, action_points: 1.0, heading: 0.0, weapon: HudWeapon::default(), sneak: None }
    }
}

/// The sneak word at the top middle while the player sneaks (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sneak {
    Hidden,
    Detected,
    Caution,
    Danger,
}

impl Sneak {
    /// [observed: the owner's four screenshots of the original.]
    pub fn word(self) -> &'static str {
        match self {
            Sneak::Hidden => "[HIDDEN]",
            Sneak::Detected => "[DETECTED]",
            Sneak::Caution => "[CAUTION]",
            Sneak::Danger => "[DANGER]",
        }
    }

    /// The two warnings are in the HUD's alternative colour, the others in its own
    /// [observed: the screenshots].
    pub fn warning(self) -> bool {
        matches!(self, Sneak::Caution | Sneak::Danger)
    }

    /// Which word: someone fights the player; else someone hostile searches for the
    /// player; else someone detects the player; else hidden [hypothesis: the words'
    /// meaning as players describe them; the original's rule is not documented].
    pub fn of(fighting: bool, searching: bool, detected: bool) -> Sneak {
        if fighting {
            Sneak::Danger
        } else if searching {
            Sneak::Caution
        } else if detected {
            Sneak::Detected
        } else {
            Sneak::Hidden
        }
    }
}

/// The sneak word from the game's detection states (`fnv_game::awareness`) and the
/// latest detection values (`Game::detection`).
fn sneak_now(g: &fnv_game::Game) -> Sneak {
    let me = FormId::PLAYER_REF;
    let (mut fighting, mut searching) = (false, false);
    for (id, a) in &g.awareness.actors {
        if g.is_dead(*id) {
            continue;
        }
        match a.state {
            Awake::Combat => fighting |= a.combat_target().is_some_and(|(t, _)| t == me),
            Awake::Alert | Awake::Lost => searching |= a.targets.contains_key(&me) || a.alarms.iter().any(|x| x.owner == me),
            Awake::Normal => {}
        }
    }
    let detected = g.detection.highest(me).is_some_and(|(_, d)| d.band == Band::Detected);
    Sneak::of(fighting, searching, detected)
}

// ---- layout --------------------------------------------------------------------------

/// How many of a bar's `count` ticks are lit at `value` (0 to 1): a tick for any part of
/// its share [hypothesis], none at 0 or below.
pub fn lit(value: f32, count: usize) -> usize {
    if value.is_finite() {
        ((value.clamp(0.0, 1.0) * count as f32).ceil() as usize).min(count)
    } else {
        0
    }
}

/// The ticks lit of a bar that empties from both ends towards its middle (the enemy's,
/// owner's playtest list 5, item 32): from and to (exclusive), counted from the left. An
/// odd tick left over goes from the right end first.
pub fn lit_middle(value: f32, count: usize) -> (usize, usize) {
    let n = lit(value, count);
    let from = (count - n) / 2;
    (from, from + n)
}

/// A tick's step and width in whole pixels for a view `height` pixels high, so that every
/// tick is as wide as the next.
pub fn tick_pixels(height: f32) -> (f32, f32) {
    let s = height / 960.0;
    let step = (TICK.x * s).round().max(2.0);
    (step, (TICK_BLOCK * s).round().clamp(1.0, step - 1.0))
}

/// Where a row's `count` ticks start, in whole pixels from the row's start, and their
/// width: the first tick at 0 and the last ending `length` pixels on, the ticks all
/// `width` wide (less only where they would touch), the gaps between them differing by a
/// pixel at most. A whole-pixel step of `round(8 x scale)` left the row up to a step and
/// more short of its bracket wherever 8 x scale is not whole (owner's playtest
/// 2026-10-16: "a gap at the end of each").
pub fn tick_lefts(length: f32, width: f32, count: usize) -> (Vec<f32>, f32) {
    if count < 2 {
        return (vec![0.0; count], width);
    }
    let gaps = (count - 1) as f32;
    // A gap of a pixel at least: (length - width) / gaps >= width + 1.
    let width = width.min(((length - gaps) / count as f32).floor()).max(1.0);
    let pitch = (length - width) / gaps;
    ((0..count).map(|i| (i as f32 * pitch).round()).collect(), width)
}

/// Where the doubled compass strip starts in its window (0 or less), so that `heading`
/// (degrees) is in the window's middle (§4).
pub fn strip_left(heading: f32) -> f32 {
    let middle = heading.rem_euclid(360.0) / 360.0 * STRIP.x;
    -(middle - COMPASS.x / 2.0).rem_euclid(STRIP.x)
}

/// The ammo counter's text: rounds in the weapon / rounds carried besides.
pub fn ammo_text(ammo: Option<(u32, u32)>) -> String {
    match ammo {
        Some((loaded, carried)) => format!("{loaded}/{carried}"),
        None => String::new(),
    }
}

/// Where the pieces stand in their corner piece, from its top left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Places {
    /// The "HP" label's start, and the health bar's left end.
    pub hp_label: f32,
    pub hp_meter: f32,
    /// The action points bar's left end (it empties from the left), and the "AP" label's
    /// start.
    pub ap_meter: f32,
    pub ap_label: f32,
    /// The bars' top.
    pub meter_y: f32,
    /// The "CND" label's start, the condition bar's start and top (its bottom on the
    /// label's line), the arrows' left, and where the ammo's text ends.
    pub cnd_label: f32,
    pub cnd_x: f32,
    pub cnd_y: f32,
    pub arrows_x: f32,
    pub ammo_right: f32,
}

impl Places {
    pub fn new(font: &Font) -> Places {
        // [observed, §4] A full bar runs from the upright's inside edge to just short of
        // the bracket's corner mark; its label stands above it, near the upright.
        let hp_meter = LEFT_MARK - METER_WIDTH - METER_SHIFT;
        let ap_meter = RIGHT_MARK - AP_METER_SHIFT;
        let cnd_x = DIVIDER - CND_BEFORE_DIVIDER - CND_BAR.x;
        Places {
            hp_label: HP_LABEL_X,
            hp_meter,
            ap_meter,
            ap_label: AP_LABEL_END - font.width(AP),
            meter_y: LINE_Y - METER_GAP - TICK.y,
            cnd_label: cnd_x - CND_LABEL_GAP - font.width(CND),
            cnd_x,
            cnd_y: LOWER_BASE - CND_RAISE - CND_BAR.y,
            arrows_x: cnd_x + JAM_CONDITION * CND_BAR.x - ARROWS_MIDDLE * (CND_BAR.y + 2.0 * ARROWS_BLEED),
            ammo_right: RIGHT_UPRIGHT - 2.0 * LABEL_GAP,
        }
    }
}

// ---- reading the game ------------------------------------------------------------------

/// Read what the HUD shows. Health and action points come from the script runtime's
/// actor values; until it runs they read full.
#[allow(clippy::too_many_arguments)]
fn gather(
    state: Res<State<PlayState>>,
    player: Option<Res<Player>>,
    combat: Option<Res<Combat>>,
    locks: Option<Res<ControlLocks>>,
    control: Option<Res<crate::control::PlayerControl>>,
    mut script: Option<ResMut<ScriptState>>,
    mut sim: ResMut<SimState>,
    mut view: ResMut<HudView>,
    vats: Option<Res<crate::vats_mode::VatsMode>>,
) {
    // [documented: GECK wiki DisablePlayerControls, the movement flag: "HUD is partially
    // disabled: Action Points, Health, Crosshair".]
    let held = locks.is_some_and(|l| l.movement);
    // Nor over the character creator (its camera is up), whatever state the opening
    // leaves the game in [owner 2026-10-17: "hide the UI so no health bar or AP or
    // dialogue"].
    let creating = control.as_ref().is_some_and(|c| c.portrait.is_some());
    let mut now = HudView { shown: *state.get() == PlayState::Playing && !held && !creating, ..view.clone() };
    if let Some(p) = &player {
        now.heading = p.heading.to_degrees();
    }
    if let Some(c) = &combat {
        now.weapon = c.hud_weapon(&sim);
    }
    let sneaking = control.is_some_and(|c| c.sneak);
    if let Some(rt) = script.as_deref_mut().and_then(|s| s.rt.as_mut()) {
        now.sneak = sneaking.then(|| sneak_now(&rt.game));
        (now.health, now.action_points) = with_world(rt, &mut sim, |rt| {
            // An aid item's temporary modifier (Buffout's Health, Jet's Action Points) raises
            // the maximum with the value [hypothesis: the bar stays a share of the whole].
            let temporary = |g: &fnv_game::Game, code| fnv_game::functions::actor_value_temporary(g, FormId::PLAYER_REF, code);
            let max = rt.game.max_health(FormId::PLAYER_REF) + temporary(&rt.game, fnv_game::actor_values::HEALTH);
            let health = fnv_game::functions::actor_health(&mut rt.game, FormId::PLAYER_REF).unwrap_or(max);
            // Base, permanent and temporary modifiers, damage: the bar is what is left of
            // the first three.
            let ap = match fnv_game::functions::actor_value_parts(&mut rt.game, FormId::PLAYER_REF, ACTION_POINTS) {
                Ok([base, modifier, damage]) => {
                    let whole = base + modifier + temporary(&rt.game, ACTION_POINTS);
                    // VATS mode's marks hold their AP: the bar shows what is left after
                    // them (docs/design/vats-mode.md §8) [design].
                    let reserved = vats.as_ref().map_or(0.0, |v| v.reserved() as f64);
                    if whole > 0.0 {
                        ((whole - damage - reserved) / whole).max(0.0)
                    } else {
                        1.0
                    }
                }
                _ => 1.0,
            };
            (if max > 0.0 { (health / max) as f32 } else { 0.0 }, ap as f32)
        });
    } else {
        now.sneak = sneaking.then_some(Sneak::Hidden);
    }
    if now.sneak != view.sneak && now.shown {
        match now.sneak {
            Some(s) => info!("hud: sneak {}", s.word()),
            None => info!("hud: sneak off"),
        }
    }
    view.set_if_neq(now);
}

// ---- drawing ---------------------------------------------------------------------------

/// The HUD's systems: what it shows is read, then drawn.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HudSet;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudView>()
            .add_plugins((crate::hud_target::HudTargetPlugin, crate::hud_limbs::HudLimbsPlugin))
            .add_systems(Update, (gather, draw, pips, texts).chain().in_set(HudSet).in_set(PlaySet::Ui));
    }
}

/// What the HUD is drawn with.
struct Art {
    font: Face,
    left_bracket: Option<Handle<Image>>,
    right_bracket: Option<Handle<Image>>,
    enemy_bracket: Option<Handle<Image>>,
    strip: Option<Handle<Image>>,
    arrows: Option<Handle<Image>>,
}

fn load_art(data: Option<&GameData>, images: &mut Assets<Image>, bc: bool) -> Art {
    let mut read = |path: &str| data.and_then(|d| texture(path, d, images, bc)).map(|t| t.0);
    let hud = |name: &str| format!("{HUD_TEXTURES}{name}.dds");
    let (left_bracket, right_bracket) = (read(&hud("glow_hud_left_seperatorglow")), read(&hud("glow_hud_right_seperatorglow")));
    let enemy_bracket = read(&hud("glow_enemy_health_seperator"));
    let strip = read(&hud("glow_hud_comp_direction_strip"));
    let arrows = read(CND_ARROWS);
    Art { font: face(FONT, plain_font(36.0, 15.0), data, images, bc), left_bracket, right_bracket, enemy_bracket, strip, arrows }
}

#[derive(Component)]
pub(crate) struct HudRoot;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum Bar {
    Health,
    ActionPoints,
    Enemy,
}

/// The weapon condition's bar.
#[derive(Component)]
struct ConditionBar;

/// A bar's row of ticks: which bar, its colour, and how the ticks stand in it.
#[derive(Component, Clone, Copy)]
struct PipRow {
    bar: Bar,
    color: Color,
    /// The row's width (interface units) and its tick count.
    width: f32,
    count: usize,
}

/// A tick of a bar, `index` from the bar's left end.
#[derive(Component)]
struct Pip {
    bar: Bar,
    index: usize,
}

/// The doubled compass strip in its window.
#[derive(Component)]
struct Strip;

/// The marks over the compass (`crate::compass` fills it): the window's place and size,
/// clipped at its sides only, as the marks reach above and below the strip.
#[derive(Component)]
pub struct CompassMarks;

/// The lower row of the right piece: the condition's label and bar.
#[derive(Component)]
struct ConditionRow;

/// Holds the ammo counter's letters.
#[derive(Component)]
struct AmmoText;

/// The enemy's health piece, and the node that holds its name's letters.
#[derive(Component)]
struct EnemyPiece;
#[derive(Component)]
struct EnemyName;

/// Holds the sneak word's letters.
#[derive(Component)]
struct SneakText;

/// A bar's row of ticks at `x`, `y` (`width` long); the ticks are set in it on whole
/// pixels by [`pips`].
fn spawn_row(p: &mut ChildSpawnerCommands, bar: Bar, x: f32, y: f32, width: f32, count: usize, color: Color) {
    p.spawn((PipRow { bar, color, width, count }, at(x, y, width, TICK.y)));
}

/// A bracket: the game's picture, or plain lines where its lines are.
fn spawn_bracket(p: &mut ChildSpawnerCommands, image: &Option<Handle<Image>>, uprights: &[f32], color: Color) {
    match image {
        Some(image) => {
            p.spawn((at(0.0, 0.0, BRACKET.x, BRACKET.y), ImageNode { color, ..ImageNode::new(image.clone()) }));
        }
        None => {
            p.spawn((at(LEFT_UPRIGHT, LINE_Y, RIGHT_UPRIGHT - LEFT_UPRIGHT, 2.0), BackgroundColor(color)));
            for &x in uprights {
                p.spawn((at(x, if x == DIVIDER { LINE_Y } else { 10.0 }, 2.0, if x == DIVIDER { 55.0 } else { 115.0 }), BackgroundColor(color)));
            }
        }
    }
}

fn spawn_hud(commands: &mut Commands, art: &Art) {
    let places = Places::new(&art.font.font);
    let color = amber(255.0);
    let font = &art.font.font;
    let label = |p: &mut ChildSpawnerCommands, text: &str, x: f32, base: f32| {
        set_line(p, &art.font, &font.lines(text, None).remove(0), x, base, color);
    };
    let screen = Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() };
    commands.spawn((HudRoot, screen, Visibility::Hidden, Pickable::IGNORE)).with_children(|root| {
        // Health and compass, bottom left.
        let piece = Node { position_type: PositionType::Absolute, left: u(MARGIN_X), bottom: u(MARGIN_Y), width: u(HP_PANEL.x), height: u(HP_PANEL.y), ..default() };
        root.spawn(piece).with_children(|p| {
            spawn_bracket(p, &art.left_bracket, &[LEFT_UPRIGHT], color);
            label(p, HP, places.hp_label, UPPER_BASE);
            spawn_row(p, Bar::Health, places.hp_meter, places.meter_y, METER_WIDTH, PIPS, color);
            let mut window = at(COMPASS_AT.x, COMPASS_AT.y, COMPASS.x, COMPASS.y);
            window.overflow = Overflow::clip();
            p.spawn(window).with_children(|w| {
                w.spawn((Strip, at(0.0, 0.0, 2.0 * STRIP.x, STRIP.y))).with_children(|s| {
                    if let Some(image) = &art.strip {
                        for i in 0..2 {
                            s.spawn((at(i as f32 * STRIP.x, 0.0, STRIP.x, STRIP.y), ImageNode { color, ..ImageNode::new(image.clone()) }));
                        }
                    } else {
                        // The eight directions' letters, 45 degrees apart.
                        for i in 0..16 {
                            let name = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][i % 8];
                            let line = font.lines(name, None).remove(0);
                            let x = i as f32 * STRIP.x / 8.0 - line.width / 2.0;
                            set_line(s, &art.font, &line, x, 44.0, color);
                        }
                    }
                });
            });
            let mut marks = at(COMPASS_AT.x, COMPASS_AT.y, COMPASS.x, COMPASS.y);
            marks.overflow = Overflow::clip_x();
            p.spawn((CompassMarks, marks));
        });
        // Action points, condition and ammo, bottom right.
        let piece = Node { position_type: PositionType::Absolute, right: u(MARGIN_X), bottom: u(MARGIN_Y), width: u(AP_PANEL.x), height: u(AP_PANEL.y), ..default() };
        root.spawn(piece).with_children(|p| {
            spawn_bracket(p, &art.right_bracket, &[RIGHT_UPRIGHT, DIVIDER], color);
            spawn_row(p, Bar::ActionPoints, places.ap_meter, places.meter_y, METER_WIDTH, PIPS, color);
            label(p, AP, places.ap_label, UPPER_BASE);
            p.spawn((ConditionRow, at(0.0, 0.0, DIVIDER, AP_PANEL.y), Visibility::Inherited)).with_children(|row| {
                label(row, CND, places.cnd_label, LOWER_BASE);
                row.spawn((at(places.cnd_x, places.cnd_y, CND_BAR.x, CND_BAR.y), BackgroundColor(color.with_alpha(GROUND))));
                row.spawn((ConditionBar, at(places.cnd_x, places.cnd_y, CND_BAR.x, CND_BAR.y), BackgroundColor(color)));
                // The jamming threshold's two arrows, over the bar (§4).
                match &art.arrows {
                    Some(image) => {
                        let mut clip = at(places.cnd_x, places.cnd_y, CND_BAR.x, CND_BAR.y);
                        clip.overflow = Overflow::clip();
                        let side = CND_BAR.y + 2.0 * ARROWS_BLEED;
                        row.spawn(clip).with_children(|c| {
                            c.spawn((at(places.arrows_x - places.cnd_x, -ARROWS_BLEED, side, side), ImageNode::new(image.clone())));
                        });
                    }
                    None => {
                        let x = places.cnd_x + JAM_CONDITION * CND_BAR.x - 2.0;
                        row.spawn((at(x, places.cnd_y, 4.0, 5.0), BackgroundColor(Color::BLACK)));
                        row.spawn((at(x, places.cnd_y + CND_BAR.y - 5.0, 4.0, 5.0), BackgroundColor(Color::BLACK)));
                    }
                }
            });
            p.spawn((AmmoText, at(0.0, 0.0, AP_PANEL.x, AP_PANEL.y)));
        });
        // The enemy's health, bottom middle: its long line level with the corner pieces'
        // (§8).
        let red = crime_colour();
        let enemy_top = HP_PANEL.y - LINE_Y + ENEMY_LINE_Y + ENEMY_BRACKET_AT.y - ENEMY.y;
        let piece = Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            margin: UiRect::left(u(-ENEMY.x / 2.0)),
            bottom: u(MARGIN_Y + enemy_top),
            width: u(ENEMY.x),
            height: u(ENEMY.y),
            ..default()
        };
        root.spawn((EnemyPiece, piece, Visibility::Hidden)).with_children(|p| {
            let (bx, by) = (ENEMY_BRACKET_AT.x, ENEMY_BRACKET_AT.y);
            match &art.enemy_bracket {
                Some(image) => {
                    p.spawn((at(bx, by, ENEMY_BRACKET.x, ENEMY_BRACKET.y), ImageNode { color: red, ..ImageNode::new(image.clone()) }));
                }
                None => {
                    p.spawn((at(bx + ENEMY_MARKS.0 - 20.0, by + ENEMY_LINE_Y, ENEMY_MARKS.1 - ENEMY_MARKS.0 + 40.0, 2.0), BackgroundColor(red)));
                }
            }
            let (from, to) = (bx + ENEMY_MARKS.0, bx + ENEMY_MARKS.1);
            spawn_row(p, Bar::Enemy, from, by + ENEMY_LINE_Y - ENEMY_METER_GAP - TICK.y, to - from, ENEMY_PIPS, red);
            p.spawn((EnemyName, at(0.0, 0.0, ENEMY.x, ENEMY.y)));
        });
        // The sneak word, top middle.
        let piece = Node { position_type: PositionType::Absolute, left: Val::Percent(50.0), margin: UiRect::left(u(-SNEAK.x / 2.0)), top: u(0.0), width: u(SNEAK.x), height: u(SNEAK.y + SNEAK_BASE), ..default() };
        root.spawn((SneakText, piece));
    });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    view: Res<HudView>,
    target: Res<HudTarget>,
    data: Option<Res<GameData>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<CompressedImageFormatSupport>>,
    mut art: Local<Option<Art>>,
    mut root: Query<&mut Visibility, (With<HudRoot>, Without<ConditionRow>, Without<EnemyPiece>)>,
    mut condition: Query<&mut Visibility, (With<ConditionRow>, Without<HudRoot>, Without<EnemyPiece>)>,
    mut enemy: Query<&mut Visibility, (With<EnemyPiece>, Without<HudRoot>, Without<ConditionRow>)>,
    mut bar: Query<&mut Node, (With<ConditionBar>, Without<Strip>)>,
    mut strip: Query<&mut Node, (With<Strip>, Without<ConditionBar>)>,
) {
    if art.is_none() {
        // The first frame the HUD is wanted: read the game's pictures and set it up.
        if view.shown {
            let loaded = load_art(data.as_deref(), &mut images, bc_supported(formats.as_deref()));
            spawn_hud(&mut commands, &loaded);
            *art = Some(loaded);
        }
        return;
    }
    let show = |on: bool| if on { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut root {
        v.set_if_neq(show(view.shown));
    }
    if !view.shown {
        return;
    }
    for mut v in &mut condition {
        v.set_if_neq(show(view.weapon.condition.is_some()));
    }
    for mut v in &mut enemy {
        v.set_if_neq(show(target.0.is_some()));
    }
    let width = u(view.weapon.condition.unwrap_or(0.0).clamp(0.0, 1.0) * CND_BAR.x);
    for mut node in &mut bar {
        if node.width != width {
            node.width = width;
        }
    }
    let left = u(strip_left(view.heading));
    for mut node in &mut strip {
        if node.left != left {
            node.left = left;
        }
    }
}

/// Set the bars' ticks on whole pixels when the view's size changes, and light those the
/// values call for.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pips(
    mut commands: Commands,
    view: Res<HudView>,
    target: Res<HudTarget>,
    data: Option<Res<GameData>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<CompressedImageFormatSupport>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    rows: Query<(Entity, &PipRow)>,
    mut ticks: Query<(&Pip, &mut Visibility)>,
    mut set_for: Local<Option<(f32, f32)>>,
    mut tick: Local<Option<Option<Handle<Image>>>>,
) {
    if !view.shown {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let key = (window.physical_height() as f32, window.scale_factor());
    if rows.is_empty() {
        return;
    }
    if *set_for != Some(key) {
        let image = tick
            .get_or_insert_with(|| data.as_deref().and_then(|d| texture(&format!("{HUD_TEXTURES}hud_tick_mark.dds"), d, &mut images, bc_supported(formats.as_deref()))).map(|t| t.0))
            .clone();
        let (height, sf) = key;
        let (step, width) = tick_pixels(height);
        let px = |v: f32| Val::Px(v / sf);
        for (e, row) in &rows {
            let room = row.width * height / 960.0;
            // A full row runs the whole room but the clear strip after its last tick
            // (§4: at 1080 lines the first tick at the upright, the last 2 pixels short of
            // the mark); the ticks fill from the bar's outer end: from the left for
            // health, from the right for action points; the enemy's stand in the middle.
            let clear = (step - width).max(1.0);
            let (lefts, width) = tick_lefts((room - clear).round(), width, row.count);
            let first = match row.bar {
                Bar::Health => 0.0,
                Bar::ActionPoints => clear,
                Bar::Enemy => clear / 2.0,
            }
            .round();
            commands.entity(e).despawn_related::<Children>().with_children(|r| {
                for (index, left) in lefts.iter().enumerate() {
                    let node = Node { position_type: PositionType::Absolute, left: px(first + left), top: Val::Px(0.0), width: px(width), height: Val::Percent(100.0), ..default() };
                    let pip = Pip { bar: row.bar, index };
                    match &image {
                        // The picture's block: its top part, as high as a tick.
                        Some(image) => r.spawn((pip, node, ImageNode { rect: Some(Rect::new(0.0, 0.0, TICK_BLOCK, TICK.y)), color: row.color, ..ImageNode::new(image.clone()) })),
                        None => r.spawn((pip, node, BackgroundColor(row.color))),
                    };
                }
            });
        }
        *set_for = Some(key);
        return;
    }
    let enemy = target.0.as_ref().map_or(0.0, |t| t.health);
    for (pip, mut v) in &mut ticks {
        let on = match pip.bar {
            Bar::Health => pip.index < lit(view.health, PIPS),
            Bar::ActionPoints => PIPS - 1 - pip.index < lit(view.action_points, PIPS),
            Bar::Enemy => {
                let (from, to) = lit_middle(enemy, ENEMY_PIPS);
                (from..to).contains(&pip.index)
            }
        };
        v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// The texts that change: the ammo counter, the enemy's name, the sneak word.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn texts(
    mut commands: Commands,
    view: Res<HudView>,
    target: Res<HudTarget>,
    data: Option<Res<GameData>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<CompressedImageFormatSupport>>,
    mut face_: Local<Option<Face>>,
    mut shown: Local<(Option<String>, Option<String>, Option<Sneak>)>,
    ammo: Query<Entity, With<AmmoText>>,
    name: Query<Entity, With<EnemyName>>,
    sneak: Query<Entity, With<SneakText>>,
) {
    if !view.shown || ammo.is_empty() {
        return;
    }
    let face_ = face_.get_or_insert_with(|| face(FONT, plain_font(36.0, 15.0), data.as_deref(), &mut images, bc_supported(formats.as_deref())));
    let font = &face_.font;
    let text = ammo_text(view.weapon.ammo);
    if shown.0.as_deref() != Some(text.as_str()) {
        if let Ok(e) = ammo.single() {
            let places = Places::new(font);
            commands.entity(e).despawn_related::<Children>().with_children(|p| {
                if let Some(line) = font.lines(&text, None).into_iter().next() {
                    set_line(p, face_, &line, places.ammo_right - line.width, LOWER_BASE, amber(255.0));
                }
            });
            shown.0 = Some(text);
        }
    }
    let enemy = target.0.as_ref().map(|t| t.name.clone());
    if shown.1 != enemy {
        if let Ok(e) = name.single() {
            commands.entity(e).despawn_related::<Children>().with_children(|p| {
                if let Some(line) = enemy.as_deref().and_then(|n| font.lines(n, None).into_iter().next()) {
                    set_line(p, face_, &line, (ENEMY.x - line.width) / 2.0, ENEMY_NAME_BASE, crime_colour());
                }
            });
            shown.1 = enemy;
        }
    }
    if shown.2 != view.sneak {
        if let Ok(e) = sneak.single() {
            commands.entity(e).despawn_related::<Children>().with_children(|p| {
                if let Some(s) = view.sneak {
                    if let Some(line) = font.lines(s.word(), None).into_iter().next() {
                        let colour = if s.warning() { crime_colour() } else { amber(255.0) };
                        set_line(p, face_, &line, (SNEAK.x - line.width) / 2.0, SNEAK_BASE, colour);
                    }
                }
            });
            shown.2 = view.sneak;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Letters 15 apart, as the HUD's font sets them.
    fn font() -> Font {
        plain_font(36.0, 15.0)
    }

    #[test]
    fn a_bar_lights_a_tick_for_any_part_of_its_share() {
        assert_eq!(lit(1.0, PIPS), 37);
        assert_eq!(lit(0.5, PIPS), 19);
        assert_eq!(lit(0.01, PIPS), 1);
        assert_eq!(lit(0.0, PIPS), 0);
        // Out of range, or not a number: an empty or a full bar, never more.
        assert_eq!(lit(-0.2, PIPS), 0);
        assert_eq!(lit(1.7, PIPS), 37);
        assert_eq!(lit(f32::NAN, PIPS), 0);
    }

    #[test]
    fn the_enemy_bar_empties_from_both_ends() {
        assert_eq!(lit_middle(1.0, 31), (0, 31));
        assert_eq!(lit_middle(0.0, 31), (15, 15));
        // Half: 16 lit, 15 gone, 7 from the left and 8 from the right.
        assert_eq!(lit_middle(0.5, 31), (7, 23));
        for i in 0..=100 {
            let (from, to) = lit_middle(i as f32 / 100.0, 31);
            assert!(to <= 31 && (31 - to) - from <= 1, "{i}: {from}..{to}");
        }
    }

    #[test]
    fn ticks_stand_on_whole_pixels() {
        // The screenshots' sizes: 6 of 8 at 720 lines, 9 at 1080.
        assert_eq!(tick_pixels(720.0), (6.0, 5.0));
        assert_eq!(tick_pixels(1080.0), (9.0, 7.0));
        assert_eq!(tick_pixels(1440.0), (12.0, 9.0));
        // Never a tick without a gap, even tiny.
        let (step, width) = tick_pixels(100.0);
        assert!(width >= 1.0 && width < step);
        // A full bar spans its room at any size: equal ticks, gaps a pixel apart at most.
        for lines in [600, 720, 768, 800, 900, 1009, 1024, 1050, 1080, 1200, 1440, 1600, 2160] {
            let h = lines as f32;
            let (step, width) = tick_pixels(h);
            let length = (METER_WIDTH * h / 960.0 - (step - width)).round();
            let (lefts, w) = tick_lefts(length, width, PIPS);
            assert_eq!(lefts[0], 0.0, "{lines} lines");
            assert_eq!(lefts[PIPS - 1] + w, length, "{lines} lines");
            let gaps: Vec<f32> = lefts.windows(2).map(|p| p[1] - p[0] - w).collect();
            let (lo, hi) = gaps.iter().fold((f32::MAX, 0.0f32), |(a, b), &g| (a.min(g), b.max(g)));
            assert!(lo >= 1.0 && hi - lo <= 1.0, "{lines} lines: gaps {lo}..{hi}");
            assert_eq!(w, width, "{lines} lines: ticks kept their width");
        }
        // At 1080 lines the step is the even 9 of the original's screenshot.
        let (lefts, w) = tick_lefts(331.0, 7.0, PIPS);
        assert!(w == 7.0 && lefts.windows(2).all(|p| p[1] - p[0] == 9.0));
    }

    #[test]
    fn the_sneak_word_is_the_worst_news() {
        assert_eq!(Sneak::of(false, false, false), Sneak::Hidden);
        assert_eq!(Sneak::of(false, false, true), Sneak::Detected);
        assert_eq!(Sneak::of(false, true, true), Sneak::Caution);
        assert_eq!(Sneak::of(true, true, true), Sneak::Danger);
        assert!(Sneak::Danger.warning() && Sneak::Caution.warning() && !Sneak::Detected.warning());
        assert_eq!(Sneak::Hidden.word(), "[HIDDEN]");
    }

    #[test]
    fn the_compass_keeps_the_heading_in_the_middle() {
        // What is in the window's middle, in texels of one strip.
        let middle = |heading: f32| (COMPASS.x / 2.0 - strip_left(heading)).rem_euclid(STRIP.x);
        assert!((middle(0.0) - 0.0).abs() < 1e-3 || (middle(0.0) - 1024.0).abs() < 1e-3);
        assert!((middle(90.0) - 256.0).abs() < 1e-3);
        assert!((middle(225.0) - 640.0).abs() < 1e-3);
        assert!((middle(-90.0) - 768.0).abs() < 1e-3);
        assert!((middle(450.0) - 256.0).abs() < 1e-3);
        // The doubled strip always covers the window.
        for h in 0..720 {
            let left = strip_left(h as f32 * 0.5);
            assert!(left <= 0.0 && left + 2.0 * STRIP.x >= COMPASS.x, "heading {h}: {left}");
        }
        // Turning right moves the strip left.
        assert!(strip_left(100.0) < strip_left(90.0));
    }

    #[test]
    fn ammo_is_loaded_then_carried() {
        assert_eq!(ammo_text(Some((0, 0))), "0/0");
        assert_eq!(ammo_text(Some((6, 42))), "6/42");
        assert_eq!(ammo_text(None), "");
    }

    #[test]
    fn the_pieces_stand_inside_their_brackets() {
        let p = Places::new(&font());
        // A full bar ends at the corner mark on the left and starts at it on the right,
        // and runs to just inside the upright.
        assert_eq!(p.hp_meter + METER_WIDTH, LEFT_MARK - METER_SHIFT);
        assert_eq!(p.ap_meter, RIGHT_MARK - AP_METER_SHIFT);
        assert!(p.hp_meter > LEFT_UPRIGHT && p.hp_meter - LEFT_UPRIGHT < 10.0);
        assert!(p.ap_meter + METER_WIDTH < RIGHT_UPRIGHT && RIGHT_UPRIGHT - (p.ap_meter + METER_WIDTH) < 10.0);
        // The bars stand on the long line, above it; the labels above the bars.
        assert_eq!(p.meter_y + TICK.y + METER_GAP, LINE_Y);
        assert!(UPPER_BASE < p.meter_y);
        assert!(p.hp_label > LEFT_UPRIGHT && p.ap_label + font().width(AP) < RIGHT_UPRIGHT);
        // Condition's label, then its bar next to the divider; the ammo right of it.
        assert_eq!(p.cnd_label + font().width(CND) + CND_LABEL_GAP, p.cnd_x);
        assert_eq!(p.cnd_x + CND_BAR.x + CND_BEFORE_DIVIDER, DIVIDER);
        assert!(p.ammo_right > DIVIDER && p.ammo_right < RIGHT_UPRIGHT);
        // The bar's bottom is on the label's line.
        assert_eq!(p.cnd_y + CND_BAR.y + CND_RAISE, LOWER_BASE);
        assert!(p.cnd_y > LINE_Y);
        // The arrows' middle at three quarters of the bar.
        assert!((p.arrows_x + ARROWS_MIDDLE * (CND_BAR.y + 2.0 * ARROWS_BLEED) - (p.cnd_x + 0.75 * CND_BAR.x)).abs() < 1e-3);
        // The enemy's ticks fit between its bracket's marks.
        assert!(ENEMY_PIPS as f32 * TICK.x - (TICK.x - TICK_BLOCK) <= ENEMY_MARKS.1 - ENEMY_MARKS.0);
    }
}
