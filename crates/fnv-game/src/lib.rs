//! Layer 5 game rules over the Layer 4 world (docs/design/layer5.md §3.1): script
//! instances and their variables, event dispatch, the quest model, condition (`CTDA`)
//! evaluation, the base-game function implementations, and the headless script runner
//! with its trace (`fnv.script.trace/1`). The interpreter itself is
//! `fnv_script::vm`; this crate is the `Host` it runs against.
//!
//! - [`aid`]: aid items (ingestibles) and what magic effects do to actor values
//! - [`data`]: compiled scripts, quests, dialogue/package results, messages, form lists
//! - [`quest`]: quest definitions and run-time state
//! - [`cond`]: the conditions of `CTDA` records
//! - [`package`]: AI packages as typed data (docs/spec-notes/packages.md)
//! - [`ai`]: which package an actor runs and when it chooses again
//! - [`game`]: the game state, instances, dispatch, SetStage, activation, snapshots
//! - [`functions`]: function implementations by opcode
//! - [`trace`]: trace events
//! - [`terminal`]: whether a terminal opens, must be hacked or is locked out; its menu items
//! - [`detection`]: how well one actor perceives another (hidden, noticed, detected)
//! - [`dialogue`]: conversations between the player and an actor (Layer 6, step D2)
//! - [`hacking`]: the terminal hacking game (words, likeness, bracket pairs, lockout)
//! - [`lockpick`]: the lockpicking game (sweet spot, torque, bobby pins, Force Lock)
//! - [`combat_lines`]: Combat-tab lines (Hit, Attack, PowerAttack, Death …) and effort sounds
//! - [`crime`]: karma, ownership, theft, trespass, assault and murder
//! - [`idles`]: the idle animations scripts, packages and dialogue lines ask for
//! - [`chargen`]: the character-creation menus' rules (name, S.P.E.C.I.A.L., tag skills, traits)
//! - [`intro`]: the intro chain's units (coverage)
//! - [`runner`]: `fnv.script.actions/1` → `fnv.script.trace/1`
//! - [`respawn`]: cell resets: respawning actors back alive after `iHoursToRespawnCell`
//! - [`voice`]: the hook that gives a dialogue response the length of its voice file
//! - [`weather`]: the weather scripts ask for and the one the sky shows
//!
//! Behaviour notes with confidence tags: docs/spec-notes/scripts.md §4 and later.

pub mod abilities;
pub mod actor_values;
pub mod aggro;
pub mod aid;
pub mod ai;
pub mod anims;
pub mod awareness;
pub mod blackjack;
pub mod casino;
pub mod chargen;
pub mod combat;
pub mod combat_group;
pub mod combat_lines;
pub mod combat_style;
pub mod cond;
pub mod crafting;
pub mod crime;
pub mod data;
pub mod detection;
pub mod followers;
pub mod dialogue;
pub mod experience;
pub mod functions;
pub mod hacking;
pub mod game;
pub mod idles;
pub mod intro;
pub mod levelup;
pub mod lockpick;
pub mod needs;
pub mod package;
pub mod perks;
pub mod poker;
pub mod provocation;
pub mod quest;
pub mod radio;
pub mod repair;
pub mod respawn;
pub mod resurrect;
pub mod rest;
pub mod roulette;
pub mod sound;
pub mod runner;
pub mod sight;
pub mod templates;
pub mod terminal;
pub mod trace;
pub mod unconscious;
pub mod use_weapon;
pub mod vats;
pub mod voice;
pub mod weather;

pub use data::GameData;
pub use game::{Game, GameConfig, InstKey};
