# What is left (2026-10-17)

Everything NVForever still lacks, in one place, ranked by what blocks a full playthrough
of the base game. Drawn from [status.md](status.md) (per system), the script function
coverage table ([design/script-function-coverage.md](design/script-function-coverage.md),
every function the shipped scripts use, probed against the original), the open items of
[owner-review.md](owner-review.md) and the outcome sections of the playtest lists in
[handoff/](handoff/). It states the project's own state as of PR #403; facts about the
original game are tagged in the linked notes. Update it when a PR closes an item.

## 1. Playthrough blockers (systems the quests need)

Ranked by the quests whose scripts, stages, objectives or dialogue use the missing or
wrong functions (the coverage table's §6 count). Fixed since that table: NPC equipment
drawn from the rules (82 quests, #374), detection by and of the player (153, #376, #389),
abilities and worn enchantments (80, #377), corpses stood up again (76, #391), fast
travel (60, #393), knock-outs (essential and fatigue, #403; row 1 below keeps what is
left of knockdowns), used items and the remaining package types and flags (#400),
GetOwnerLastTarget (#403). About 50 functions are still stubs.

| # | System | Quests | What is missing | Functions |
|---|---|---|---|---|
| 1 | Knockdowns (knock-outs done 2026-10-17) | 64 (the intro chain among them) | Knock-outs are built ([unconscious](spec-notes/unconscious.md)): essential actors at zero health and actors out of fatigue drop as ragdolls, stay down (an essential while a fight goes on near it) and get up with the game's clips, and GetKnockedState, GetUnconscious, IsFacingUp and IsLeftUp answer for them. Left: a live actor knocked down without being knocked out (PushActorAway, explosions' knockdown) never lies down; SetUnconscious holds an actor's AI but does not lay it down | PushActorAway, SetUnconscious |
| 2 | Saves from scripts | 29 | Saves exist; the script calls do nothing (cheap) | SystemSave, ForceSave |
| 3 | Look turning the body | 26 | Look's rotate-body flag is stored, never acted on | Look |
| 4 | Animation state told to the rules | 22 | The engine knows attacks, reloads and idles but does not tell the rules | IsPlayerActionActive, IsAnimPlaying, IsIdlePlaying, IsMoving's directions, GetAnimAction for attacks and reloads |
| 5 | Repair menu (done 2026-10-17) | 10 (Raul's quests among them) | Built ([repair](spec-notes/repair.md)): ShowRepairMenu opens a vendor's repair services (worn weapons and armour repaired for caps up to the vendor's skill), and the Pip-Boy's Repair (R on the ITEMS page) uses a like item or one on the item's repair list. Left: the Pip-Boy's repair list is not clickable and does not report MenuMode 1035 | (none) |
| 6 | Barber, plastic surgeon and tutorial menus | 9 | The barber is built: the character creator as the barber, hair, colour and beard only (character-creator.md §10). Not built: the plastic surgeon, the tutorial menu | ShowPlasticSurgeonMenu, ShowTutorialMenu |
| 7 | Pip-Boy reset | 8 | Not built | ResetPipboyManager |
| 8 | Talking activators | 6 | Activators that speak (radios with lines, intercoms) do not talk | IsActorTalkingThroughActivator |
| 9 | Menu codes | 6 | MenuMode does not report every menu's code (the Pip-Boy's 1 and the loading codes) | MenuMode |
| 10 | Caravan | 5 | The card game is not built | ShowCaravanMenu and its functions |
| 11 | Race and sex changes on NPCs | 5 | AgeRace is a stub; SexChange errors on NPCs (the player's works) | AgeRace, SexChange |
| 12 | Placing at the crosshair | 5 | Cheap | PlaceAtReticle |
| 13 | A leveled actor's level re-rolled | 4 | SetLevel sets the level but does not re-roll the NPC's inventory | SetLevel |
| 14 | Cannibal and Mister Sandman | 3 | The perks' activate choices are not built | (perk entry points) |
| 15 | VATS | 3 quests, 250 conditions | VATS mode built (vats-mode.md), awaiting the owner | GetVATSValue and GetVATSMode answer for VATS mode; GetVATSTargetHeight and SetVatsTarget stay stubs |
| 16 | Miscellaneous stats, scale | 3 each | Counters other than ModPCMiscStat's; SetScale not read by the engine | GetPCMiscStat, SetScale |
| 17 | Dismemberment, end videos, cell image space | 2 each | Not built | (dismemberment), PlayBink, SetCellImageSpace |
| 18 | One quest each | 1 each | Screen splatter, AI package codes (Sandbox, Travel-at-goal), AutoDisplayObjectives, navmesh switches, SetPCCanUsePowerArmor, ExitGame | |

Rows the coverage table ranks with 0 quests are not listed. The table's own §3.3 lists
six functions still waiting for an in-game capture to settle their answers.

## 2. Systems not started or partly built

From [status.md](status.md); the playthrough blockers above are not repeated.

- **Gameplay**: VATS mode built for single-shot guns (bursts, melee and thrown weapons not yet); people's power attacks (creatures' are built); melee
  weapon use through UseWeapon; the companion wheel, aggressive/passive companions and
  companions sneaking with the player; perks that change skill points; slot machines; crafting's icon, effects line and making several at
  once; reputation disguises and resets.
- **Animation**: actors' jumps, NPCs' sneaking weapon clips, PickIdle; armour ragdoll
  templates.
- **Rendering**: exterior lighting strength and clouds against the game's frames (1 of 8
  exterior reference frames meets the threshold; all 7 interior ones do); depth of field,
  motion blur, screen splatter; effect shaders' holes and add-on models, ash piles; about
  66 particle-system smokes not drawn (playtest list 4 item 16).
- **Interface**: credits; the ITEMS page's DPS, ammo badge and Mod hint; the +10
  XP line and the Locations Discovered stat; the controller's button pictures and
  per-menu buttons.
- **Audio and video**: reverb, spatial sinks; PlayBink (the end videos).
- **Saves**: deleting saves from the menu, loading into another load order, AI progress.
- **Console**: several statements on a line.
- **World**: skeleton collision still upright (playtest list 5 item 71).
- **AI packages**: Find's Allow Buying, an escort joining its charge's fights (the other
  package types and flags are carried out since #400).

## 3. Decisions and checks waiting for the owner

[owner-review.md](owner-review.md) holds about 146 open items; about 26 of them can change
how quests run, the rest are guessed constants and looks to compare in the game.

**Quest-critical** (each needs a ruling):
- Layer 5: the `L5.script.decode` choice, the `delay_*` scenario inputs, holdouts to
  demote and where `encodings.json` lives (lab rules); the runtime rules still to confirm
  (stage conditions, timers, reputation tolerances, `ref` handling, disabled references,
  Return, script packages).
- Script stubs and the function audit: ResetAI and procedure codes, SetPackDuration,
  SetActorsAI; package durations (about 358 packages restart every duration).
- EnablePlayerControls: a number left out now gives that control back.
- Dialogue: Info Connection's shared lines, the end of a Random pool, actors without a
  greeting; whether walking away or an attack ends a conversation; force-greets; a generic
  Goodbye beside Goodbye lines with scripts.
- AI: where teammates arrive through a door; the shared-faction change that makes NCR and
  Fiends fight; whether an assault turns the victim's whole faction; IsInCombat while
  Alert.
- Sleeping: whether E on a bed opens the sleep menu at once.

**Looks and constants by area** (open, not quest-critical): rendering 29, menus and
interface 27, combat 16, AI 11, dialogue 9, animation 9, world and physics 5, gameplay
3, sound 3; the repository and install 8 (old remote branches to delete, figures to
place in docs).

**About 25 owner-review items are stale** (overtaken by later work: the sky task, the
HUD's screenshot request, the barter "How many?" box, the crosshair's not-built list,
the wait menu's day of the week, and others). They should be struck through so the open
list reads true.

**Not reproduced** from the playtest lists: the first-person shadow flicker after leaving
an interior; the Crimson Caravan crash. **Never checked in the game** after fixing: list 5
item 22 (convict karma); Victor's and Sunny's stop-and-go (list 4 items 2–3). The
`combat-violet` and `combat-dogs` known-failing marks need the owner's word.

## 4. Found by today's comparisons with the original

From [spec-notes/npc-behaviour-compare.md](spec-notes/npc-behaviour-compare.md) §8:
- Fights end far sooner in ours (NCR against Fiends over in 32 s; the original's still
  going after 120 s).
- Melee attackers against the player in god mode or with huge health keep away in the
  original and never swing; ours always close in (the rule is not understood).
- The Strip's crowd was not observed (the `strip` campaign from the owner's save was
  dropped); only Gomorrah's first 12 game hours of the casinos were.
- No spawn aid for creature fights in our engine's test runs.

## 5. Process

- **Smoke scenarios failing on main**: #403's full changed-path run (2026-10-17) found
  `follower-boone`, `crippled-limp-gecko` and `death-cam-first` failing the same way on
  origin/main as on its branch, from the #400 build on. They are not marked
  `known_failing`; each needs a look (a regression from #400–#402, or timing on a loaded
  machine).

- **Game lock queue**: the smoke runner takes the game locks per launch, and the waiter
  that polls fastest wins, so a multi-scenario run is split up and early waiters can
  starve (2026-10-17: a run waited over four hours). A first-come, first-served queue, or a
  lock held for a whole run, would fix it.
- **`scenarios.toml` merges**: git's merge of two branches that both append scenarios at
  the end drops the `]` and `[[scenario]]` lines between them (at least five times on
  2026-10-16/17). After a merge, check the file parses and holds every scenario of both
  sides.
- **The README and this file** go stale fast with several sessions merging a day; update
  them with the PR that changes a system's state.
