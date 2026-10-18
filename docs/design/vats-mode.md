# VATS mode: slow-time mark and fire

Status 2026-10-17: **design for review; built on `claude/vats-mode`** (the engine's
`nvf-play` `vats_mode.rs`, `fnv_combat::vats`, `fnv_game::vats`; not yet seen in a game
run). Owner request (todo
6): "VATS mode", NVForever's own take on VATS, modelled on Red Dead Redemption's Dead Eye.
Combat is not a parity target (docs/design/combat.md §1: only the quest script hooks must
behave as in the original), so this is deliberately **not** the original's VATS (no
paused target menu, no hit chances, no kill camera). The original's VATS stays "not
started" in docs/status.md; the script functions that ask about VATS answer for this
mode (§12).

Tags: `[documented]` cites a page or file, `[observed]` is something we looked at in the
game data or the references, `[hypothesis]` is unconfirmed, and **`[design]` is our own
choice where the owner's design leaves something open**. The owner's design itself is
quoted as "owner".

## 1. The owner's design

- **Enter** (owner): pressing V slows time almost to a stop ("very, very slow") and a
  different colour swipes across the screen as it enters.
- **Mark** (owner): the player clicks valid actor targets (creatures, NPCs, any actor).
  Left click (Attack) marks the one under the crosshair with an "x" and reserves the
  shot's AP; the AP cost comes from the records. The player can mark body parts.
- **Unmark and exit** (owner): right click removes the last mark; right click with no
  marks left ends VATS mode.
- **Limits** (owner): no more marks than the AP and the loaded ammunition allow (a .357
  with 6 rounds loaded takes at most 6 marks). Single-shot pistols and rifles first.
- **Accept** (owner): E (Activate) fires every marked shot in real time (since 2026-10-18 at half speed, §4), auto-aimed at
  each mark in order, faster than the weapon's normal rate.
- **Accuracy** (owner): 100 %; no shot misses.
- **Sound** (owner): the base game's VATS sounds for entering, marking, accepting and
  leaving.
- **Look** (owner's references): two Dead Eye clips in `C:\fnv-private\VATS` (private,
  never committed). Frames were taken from them with Windows' own MediaComposition
  thumbnails (no ffmpeg on this machine) into `C:\fnv-private\VATS\frames`; §5 says what
  they show.

## 2. States

```
        V (a gun, AP for a shot)            E (one mark or more)
  Off ───────────────────────────▶ Marking ───────────────────▶ Firing ──(last shot + 0.3 s)──▶ Off
   ▲                                │  ▲ │                                 │
   │  right click with no marks,    │  │ └ left click: mark / right click: unmark
   │  V again, a menu, death        │  └────────────────────────────────────┘ (never back)
   └────────────────────────────────┘
```

- **Off**: the game as usual.
- **Marking**: the clock runs at [`MARK_SCALE`](#4-time-scale); the view still turns at
  full speed (the mouse is read per event, not per game second); left click marks, right
  click unmarks, E accepts. The weapon's own trigger, aiming down the sights and
  activation are held (§11.5).
- **Firing**: the clock runs at half speed (`FIRE_SCALE`, §4); the marked shots leave one after another
  (§9); all other input is held until the mode is off [design: the sequence is the
  player's committed action, like the original's VATS playback].
- Leaving takes the grade off over [`LEAVE_FADE`](#5-the-look) (§5); the state is Off
  at once, only the look lingers.

Nothing of the mode is saved (§11.7).

## 3. Entering

V (`Control::Vats`, the game's `Vats` binding 0x002FFF0E, already read by
`engine/nvf-play/src/bindings.rs`; until now it said "not available yet",
`control.rs` `NOT_BUILT`) enters Marking when all of these hold, else it plays
`UIVATSEnterFail` and logs why [design: the masters hold no VATS message strings, and the
owner wants no placeholder text on screen (`activate::HudMessage` keeps the line, not
drawn); the refusal sound tells the player]:

1. The game is playing (`PlayState::Playing`), the player is alive, and no script has
   disabled the fighting controls (`DisablePlayerControls`, `ControlLocks::fighting`).
2. The weapon in use is a gun with a firing cycle: not melee or unarmed, not a thrown
   weapon or mine (spends itself), and it fires a projectile (the detonator and the
   binoculars do not). Message: "VATS mode needs a gun."
3. It is not reloading or jammed. Message: "Not while reloading."
4. It has a round loaded. Message: "Out of ammo."
5. The player has the AP for one shot (§7). Message: "Not enough action points."

With the weapon away, V draws it and enters as soon as it is out [design: a press that
needs no second press; until the weapon is out the mode waits, and a second V cancels].

## 4. Time scale

- `MARK_SCALE = 0.05` [design]: one game second takes 20 real seconds while marking. The
  owner asked for "very, very slow"; the references show people still moving a little
  [observed: Sample 1 and 2, the actors move a step or two over the ~10 s of marking].
- The scale eases from 1 to 0.05 over the swipe (`ENTER_SWIPE = 0.35 s` of real time,
  smoothstep) [design], so the world is seen to slow down.
- Firing runs at `FIRE_SCALE = 0.5`, half speed [design: owner 2026-10-18, "the firing
  is too fast"; first built at 1 after the owner's "in real time"]. The clock jumps from
  0.05 to 0.5 on E and back to 1 when the mode ends after the last shot.
- Mechanism: the engine's own slow motion multiplies SetGlobalTimeMultiplier
  (`script.rs` `time_multiplier`): `ClockScale` (the death camera's) × the VATS mode's
  scale. Everything on the virtual clock slows together: animations, physics, NPC AI,
  projectiles in flight, scripts' frame time and the game clock. Sounds keep their
  pitch (as for the death camera). The test aids' clock is the virtual one, so aids
  timed during marking stretch 20 times in real time (§13).

## 5. The look

What the references show [observed: `C:\fnv-private\VATS\frames`, Sample 2 at
0.9–2.5 s and Sample 1 at 0.6–2.2 s, 24 frames each]:

- Entering: in about 0.1 s the picture first goes pale sepia, then a near-white, creamy
  yellow wash covers it (≈ 0.15 s), which fades over ≈ 0.8 s into a lasting **amber
  grade**: mid tones ≈ (228, 141, 62), shadows a dark orange-brown ≈ (91, 43, 5), the
  corners darker (a vignette). The grade stays for the whole mode and drops back to the
  normal colours when the shots are done.
- Marks: a small dark-red "x", about 2 % of the screen width across, on the spot
  clicked; the crosshair is a small dot.

Ours [design, after the references and the owner's "swipes a different colour across
the screen"]:

- **Swipe** (`ENTER_SWIPE = 0.35 s` real): a soft vertical edge crosses the screen from
  left to right. Behind it (to its left) the picture is in the amber grade; on the edge
  rides a band of the creamy wash ((1.0, 0.95, 0.78), 22 % of the screen wide, strongest
  at the edge). When it has crossed, the whole screen carries a wash of 0.5 that fades
  to nothing over 0.6 s, as the references' flash does.
- **Grade** while the mode is on: saturation drops, the picture goes towards its
  luminance × amber (1.45, 0.92, 0.42) by 0.85 (amber mids from mid grey, as measured),
  and the corners darken by up to 35 %.
- **Leaving** (`LEAVE_FADE = 0.3 s` real): the grade fades out evenly.
- It is drawn by the image-space pass (`engine/nvforever/src/imagespace.rs`, its
  shader's last steps, after the scripts' modifiers and before the fade colour), from
  four numbers the play side publishes each frame (`VatsMode::look`: edge, band, wash,
  grade).

## 6. Marking: targets and body parts

- **What is under the crosshair**: the same line the shots use: from the eye along the
  view in first person, the camera's line through the crosshair in third person
  (`control::Crosshair`). Its reach is the projectile's range (PROJ `DATA` Range) when
  it has one, else 8,192 units [design].
- **A valid target** is a living actor other than the player with a hit volume
  (`crate::hitvol`: creatures, NPCs, anything with an actor reference). The ray meets
  actor hit volumes and the world (walls, rocks stop it); a click on nothing, the world,
  a dead body or an object plays nothing and marks nothing [design]. A friendly or
  neutral actor can be marked: shooting one is a crime as any other shot is (combat,
  crime) [design].
- **Body parts**: an actor whose hit volume has body parts (BPTD, combat step C8,
  `hitvol::ActorParts`) is struck by the ray on one part capsule; the mark is on that
  part (its index in the BPTD, its name from the BPTD's part name). An actor with a single
  capsule is marked whole, aimed at its centre. The weapon an NPC holds
  (`hitvol::WEAPON_PART`) can be marked: the shot then wears that weapon (combat §17)
  [design].
- **The same actor** can be marked more than once, on the same or other parts; each mark
  is one shot (as Dead Eye) [design].
- Each mark keeps the actor, the part, and where on the screen to draw it: the part's
  capsule centre now, followed every frame while the actor moves (`PhysWorld::
  actor_part_center`, the whole capsule's aim point without parts). An actor that loses
  its volume (it died, its ragdoll took over, it unloaded) keeps the last point seen.

## 7. AP and ammunition

### 7.1 The cost of a shot

The weapon's AP cost [observed: FalloutNV.esm WEAP `DNAM`; the "Override - Action
Points" flag (Flags 2) and the `override_action_points` field, `fnv_combat::data::
WeaponStats::action_points`]:

| Weapon | Override AP | Weapon | Override AP |
|---|---|---|---|
| WeapNV357Revolver | 26 | WeapNVVarmintRifle | 35 |
| Weap10mmPistol | 17 | WeapNVCowboyRepeater | 27 |
| WeapNVServiceRifle | 15 | WeapSniperRifle | 27 |
| WeapNVHuntingShotgun | 28 | WeapNVAntiMaterielRifle | 55 |

A weapon without the flag costs the attack setting of its animation type [hypothesis:
the setting each type uses, from the names; values observed in FalloutNV.esm GMST]:

| Animation type (WEAP DNAM) | Setting | Value |
|---|---|---|
| Pistol - Ballistic / Energy (1 Hand) | `fActionPointsAttackPistol` | 20 |
| Rifle - Ballistic / Automatic / Energy (2 Hand) | `fActionPointsAttackRifle` | 25 |
| Handle (2 Hand) | `fActionPointsAttackHandle` | 20 |
| Launcher (2 Hand) | `fActionPointsAttackLauncher` | 40 |
| Melee (1 Hand) | `fActionPointsAttackOneHandMelee` | 15 |
| Melee (2 Hand) | `fActionPointsAttackTwoHandMelee` | 30 |
| Hand to Hand | `fActionPointsAttackUnarmed` | 22 |
| Grenade Throw / Thrown (1 Hand) | `fActionPointsAttackGrenade` | 35 |
| Land Mine / Mine Drop (1 Hand) | `fActionPointsAttackMine` | 10 |

Only guns enter VATS mode (§3), so only the first four rows are used now. Not applied
yet [design, for later]: `fActionPointsSustainedFireMult` (2, automatic fire),
`fActionPointsVATSMeleeMult`, perk entry points that change the cost (the "Action Point
Cost" ingestible effect, Nerves of Steel's regeneration [documented: GECK wiki Misc
"Action Point Cost … Modifies Action Point Cost of Weapons in VATS"]). Weapon mods do not
change the AP cost [hypothesis].

### 7.2 Reserving and spending

- The player's AP is the script runtime's Action Points actor value (code 12: base 65 +
  3 × Agility [documented: `fAVDActionPointsBase`, `fAVDActionPointsMult`, observed 65
  and 3], modifiers, damage), as the HUD's AP bar shows it (`hud.rs`).
- **A mark reserves** the cost; nothing is spent while marking. A mark is refused
  (`UIVATSInsufficientAP`, logged "Not enough action points") when `reserved + cost`
  would exceed the AP left now.
- **Ammunition**: a mark is refused (the same sound, logged "No more rounds loaded") when
  the marks would need more rounds than the magazine holds now: `marks × ammo_use ≤
  magazine` [design: the rounds loaded, not those carried; Dead Eye does not reload
  mid-sequence either]. God mode (a full magazine always) keeps the magazine's size as
  the limit.
- **Unmarking** frees that mark's reservation.
- **Each shot fired spends** its cost (damage on Action Points), so a sequence cut short
  (§9.3) spends only what it fired. Leaving without firing spends nothing.
- **Regeneration** [hypothesis, the setting's name; rate unconfirmed]: outside VATS mode
  the player's spent AP comes back at `fActionPointsRestoreRate` (0.06, FalloutNV.esm) ×
  the AP maximum per game second: a full bar of 92 AP in ≈ 17 s. Nothing regenerated AP
  before this (nothing spent it). Not while marking or firing.

### 7.3 The pure rules

`fnv_combat::vats` holds the headless parts, unit-tested: `shot_ap_cost(weapon,
settings)`, and `MarkQueue` (push with the AP and round limits, pop the last, reserved
AP, rounds needed, the shots in order). The engine module only feeds it.

## 8. Mark visuals and the HUD

- **"x" marks**: each mark draws a dark-red (0.45, 0.05, 0.03) "x" of two crossed bars,
  1.9 % of the screen width across, at its point projected on the screen (§6) [design,
  after the references]. Marks behind the camera are not drawn. A mark that has fired
  goes [design].
- **AP bar**: while marking, the HUD's AP bar shows the AP that is left after the
  reservations (`(AP − reserved) / max`), so it drains mark by mark [design]; firing
  spends it for real, so the bar stays where it was.
- The crosshair stays as it is. The HUD is not hidden [design].
- A refused press is heard (§10) and logged, not written on the screen (§3).

## 9. Firing

### 9.1 The sequence

On E with at least one mark (`UIVATSReady`):

1. The clock goes to `FIRE_SCALE` (0.5, half speed) at once; the grade stays.
2. Every `SHOT_INTERVAL = 0.2 s` of game time, 0.4 s of real time at half speed [design:
   owner 2026-10-18, the first 0.12 s at full speed was too fast; still faster than a
   single-shot weapon's own rate, so the owner's "very quickly"] the next mark in the
   order it was made:
   - the view turns to it: heading and pitch from the eye to the mark's point (the
     part's capsule centre now), at once [design: a snap, as Dead Eye's camera does];
   - one shot leaves through combat's own path (`combat.rs` `fire`): the cycle is made to
     fire now whatever its cooldown or wind-up (the fire-rate override), it takes its
     rounds from the magazine as any shot, plays the weapon's sounds, muzzle flash,
     casing and OnFire blocks, wears the weapon, and counts in the stats;
   - the shot leaves along the line from the eye to the point with **no spread, bloom or
     recoil** (owner: 100 % accuracy) and never jams [design];
   - the shot's AP is spent (§7.2).
3. After the last shot, `AFTER_LAST = 0.3 s` of game time (0.6 s real) [design], the
   mode is Off, the clock back at 1
   (`UIVATSExit`) and the grade fades out.

### 9.2 What a hit is

The projectile still flies through the physics world (`fnv_phys` rays) and hits what it
meets: 100 % accuracy means the shot leaves exactly at the mark, not that it passes
walls. Something that moved between the eye and the mark after marking (another actor,
a door) takes the shot instead [design]. A hit is an ordinary hit (combat C5/C8):
damage, limb damage, crippling, death, OnHit, crime, XP. The hit counts as a VATS
attack for the critical chance: `Attacker::in_vats` adds `fVATSCriticalChanceBonus`
(15, FalloutNV.esm) [documented: GECK wiki Critical Hit, the VATS bonus;
docs/spec-notes/combat-rules.md §2] [design: kept, as the original's VATS has it].

### 9.3 Cut short

The sequence stops (Off, `UIVATSExit`, the rest of the marks dropped and their AP not
spent) when the player dies, a menu opens, a script disables the fighting controls, or
the weapon leaves the hand (it broke on a shot, a script unequipped it). A mark whose
actor died before its turn is still shot at its last point [design: the body is a
target, as a shot at a corpse is].

## 10. Sounds

The base game's VATS SOUN records [observed: FalloutNV.esm SOUN Editor IDs; which
moment each belongs to is a hypothesis from its name unless said otherwise]:

| Moment | SOUN | FormID |
|---|---|---|
| Entering | `UIVATSEnter` | 0x00015630 |
| While marking (a loop) | `UIVATSLoop` | 0x0015511B |
| A mark on an actor without parts | `UIVATSSelectTarget` | 0x0002764D |
| A mark on a body part | `UIVATSSelectTargetPart` | 0x00014C35 |
| Unmarking [design: a camera move in the original] | `UIVATSMove` | 0x00015633 |
| A refused mark (AP or rounds) | `UIVATSInsufficientAP` | 0x00022E16 |
| Accepting (E) | `UIVATSReady` | 0x00015632 |
| Leaving (after firing or unmarked) | `UIVATSExit` | 0x00015631 |
| Entering refused (§3) | `UIVATSEnterFail` | 0x000039F7 |
| Not used | `UIVATSCameraIn`, `UIVATSCameraOut` | 0x000158D3, 0x000158D4 |

They play at the listener (`PlaySound { at: None }`, the UI category), found by Editor ID
in `Sfx::forms` (no FormID is written in the code). The loop starts on entering and
stops on accepting or leaving.

## 11. How it meets the rest

### 11.1 Combat

- `combat.rs` gets the mode's request each frame (a shot due now, and its point) and
  fires it as §9.1 says; the player's own trigger is not pulled while the mode is on.
- `By` (who fired a projectile) carries `vats: true` for these shots, which `attacker()`
  passes as `in_vats` when the hit is resolved (§9.2). The `aiming_vats` parameter of
  `attacker()` existed for this and was always false.
- Recoil is not added for these shots; bloom is not grown [design].
- Weapon wear (§17 of combat-rules.md) applies; `fVATSDamageToWeaponMult` (1.0) is not
  applied [design: it is 1].
- Other actors' attacks are not changed: during marking they are slowed with the world;
  during firing they act as usual. `fVATSPlayerDamageMult` (the original's damage the
  player takes in VATS) is not applied [design].

### 11.2 Detection and AI

Nothing special: every shot is a gunshot with its noise and its hit, as any of the
player's (docs/design/detection.md). Entering and marking make no noise [design]. The AI
runs on the slowed clock (§4), so actors keep doing what they were doing, slowly.

### 11.3 The HUD

§8: the AP bar shows AP minus reservations; the marks are drawn by `vats_mode`'s own
overlay, under the menus.

### 11.4 The time scale and the death camera

§4: the two scales multiply. Dying ends the mode first (§9.3), so in practice they do
not overlap.

### 11.5 Controls

- `Control::Vats` (V) leaves `NOT_BUILT`; it enters, and in Marking leaves without
  firing [design: a second V is "never mind", as a right click with no marks].
- In Marking and Firing the mode takes Use (left mouse: mark), Block (right mouse:
  unmark) and Activate (E: accept) for itself: `PlayerControl::trigger` and
  `PlayerControl::aiming` are cleared and activation is held (`activate.rs`) [design].
  Movement, looking, sneaking and the camera (Toggle POV too) keep working, slowed with
  the world. Ready Item's reload and Ammo Swap are held while the mode is on [design: no
  reload or ammunition change mid-mode].
- Controller: the same controls on their pad bindings (`Bindings::pad`), nothing extra.
- `DisablePlayerControls` with the fighting group off refuses entering and ends the mode
  (§3, §9.3).

### 11.6 Menus and pause

Any menu (`PlayState` other than Playing: the pause menu, the Pip-Boy, dialogue,
loading) ends the mode at once without firing; marks are dropped and nothing is spent
[design: simplest, and the slowed world would otherwise sit under the menu].

### 11.7 Saving

The mode is never saved and is Off after a load. A save made while marking (only
possible through the pause menu, which ends the mode first, §11.6) holds the AP as it is,
reservations being nothing spent. Spent AP is the Action Points damage the save already
keeps. The VATS clock scale is not part of the save.

## 12. Script functions

| Function | Opcode | Conditions in the masters | Returns with VATS mode |
|---|---|---|---|
| GetVATSMode | 0x10E0 | — | 0 Off; **1** (`VATS_TARGET_SELECT`) while marking; **4** (`VATS_PLAYBACK`) while firing [documented: GECK wiki GetVATSMode return values 0–4] [design: our states mapped to the nearest; 2 menu and 3 scan are never returned] |
| GetVATSValue | 0x1198 | 250 | While firing, about the shot leaving now; 0 otherwise (below) |
| GetVATSTargetHeight | 0x10EB | — | stays a stub (0) [design] |
| SetVatsTarget | 0x11EB | — | stays a stub (no effect) [design] |

GetVATSValue's queries [documented: GECK wiki GetVATSValue lists the queries; the
query codes are the page's order from 0 [hypothesis]]. The player's current VATS shot is
the one firing now (set before the shot leaves, cleared when the mode is off), so the
conditions of camera paths and perk entry points evaluated while a shot leaves see it:

| Code | Query | Returns |
|---|---|---|
| 0 | WeaponIs (a WEAP) | 1 if the shot's weapon is it |
| 1 | Weapon In List (a FLST) | 1 if the weapon is in the list |
| 2 | Weapon Type Is (an animation type) | 1 if the weapon's DNAM animation type is it |
| 3 | Target Is (an NPC_/CREA) | 1 if the target's base (or the reference itself) is it |
| 4 | Target In List (a FLST) | 1 if the target's base or reference is in the list |
| 5 | Target Distance | the distance from the player to the target, units |
| 6 | Targeted Part (a body part's condition actor value) | 1 if the mark's part names that condition: the BPTD part's own Actor Value, as combat's limb hits read it (`fnv_game::combat::limb_condition`, 25–31) |
| 7 | VATS Action | 1 for the code of the attack action (0) [hypothesis: Attack is the first] |
| 8 | Is Success | 1 (owner: no shot misses) [design] |
| 9–15 | Is Critical, Crit Effect Is, Crit Effect In List, Is Fatal, Explode Part, Dismember Part, Cripple Part | 0: none of them is known before the shot lands [design] |

The parameter of codes 0, 1, 3 and 4 arrives as a raw FormID (`ptVATSValueParam` is
read as an integer, `fnv_game::cond`): it is taken as a load-order FormID, right for
records of FalloutNV.esm, whose local and load-order FormIDs agree [hypothesis: the
masters' conditions name FalloutNV.esm records].

The engine tells the script runtime: `Game::vats` (mode and shot) is set by `vats_mode`
each frame. The `in_vats` flag in `fnv_combat::damage::Attacker` is the hit side (§9.2).

## 13. Test aids and smoke tests

- `NVF_PLAY_TEST_KEYS` presses V, the left mouse button and E through the bindings
  (`v`, `mouse-left`, `mouse-right`, `e`), so the real key path is tested.
- New `NVF_PLAY_TEST_CONTROL` press **`aimvats`**: turn the view (and the third-person
  crosshair) to the nearest living actor with a hit volume that has no VATS mark yet, at
  its aim point (as `aimactor`), so a click marks it.
- Aid times are on the virtual clock (§4): an aid 0.05 s after another while marking
  comes about 1 s later in real time.
- Log lines (smoke tests read them):
  - `vats mode: on (<weapon EDID>, <n> AP a shot, <m> rounds loaded, <ap> AP; at most <k> marks)`
  - `vats mode: refused: <why>` (entering)
  - `vats mode: marked <name> <ref> (<part>), mark <k>; <r> AP reserved, <left> AP left`
  - `vats mode: mark refused: <why>`
  - `vats mode: unmarked <ref> (<part>), <n> marks left`
  - `vats mode: firing <n> marked shots`
  - `vats mode: shot <k> of <n> at <name> <ref> (<part>), <cost> AP spent`
  - `vats mode: off (<n> shots fired, <ap> AP spent; <why>)`
  - at exit: `vats mode summary: entered <e> times, <m> marks, <s> shots, <ap> AP spent, <r> refused`
- Scenarios in `tools/smoke/scenarios.toml`: `vats-mode-two-marks` (the Goodsprings dogs,
  a .357: enter, mark both, accept, two shots, 52 AP), `vats-mode-unmark` (mark, unmark,
  right click again: off with nothing fired or spent) and `vats-mode-ammo-limit` (one
  round loaded: the second mark is refused).

## 14. Not built, for later

- Automatic weapons' bursts (several shots per mark, `fActionPointsSustainedFireMult`),
  shotguns as one shot already work; melee and thrown weapons in VATS mode.
- Perk entry points on the AP cost and the "Action Point Cost" effect; the original's
  hit chances and the perks that change them (owner: 100 % for now).
- A kill camera, the original's VATS menu, GetVATSTargetHeight, SetVatsTarget.
- Tuning by eye: `MARK_SCALE`, the swipe and grade colours, `SHOT_INTERVAL`, the mark's
  size, all [design] and owner-checkable in one run.

## 15. Evidence

| Fact | Source |
|---|---|
| Weapon AP overrides (.357 26, 10mm 17, …) | [observed] `fnvtool combat form` on FalloutNV.esm, DNAM `override_action_points` with the Flags 2 bit |
| `fActionPointsAttack*` 20/25/20/40/15/30/22/35/10, `fActionPointsSustainedFireMult` 2, `fActionPointsRestoreRate` 0.06, `fVATSCriticalChanceBonus` 15, `fVATSPlayerTimeUpdateMult` 6 | [observed] FalloutNV.esm GMST (`fnvtool esm decode`) |
| VATS SOUN Editor IDs | [observed] FalloutNV.esm SOUN records |
| GetVATSMode return values, GetVATSValue queries | [documented] GECK wiki pages, local export `C:\fnv-private\geckwiki\functions.xml` |
| The look of Dead Eye | [observed] the owner's two clips, frames in `C:\fnv-private\VATS\frames` (private) |
