//! The numbers the rules use (docs/design/combat.md §1 "Tunable in one place").
//!
//! - [`Rules`]: the game settings the documented formulas name, read from the masters.
//!   A setting the masters lack falls back to the default the GECK wiki page gives
//!   [documented: the page named on each field; absent from the masters = engine
//!   default is a hypothesis, docs/spec-notes/combat-data.md §3].
//! - [`CombatTuning`]: NVForever's own values for what no record or setting covers
//!   [design]. Changing one is a tuning change, not a code change.

use serde::Serialize;

use crate::data::CombatForms;

/// Game settings used by the damage, critical, spread, knockdown and knockback rules.
#[derive(Debug, Clone, Serialize)]
pub struct Rules {
    // Weapon Damage Formula
    pub damage_weapon_mult: f32,
    pub damage_skill_base: f32,
    pub damage_skill_mult: f32,
    pub melee_damage_strength_mult: f32,
    pub melee_damage_strength_offset: f32,
    pub unarmed_damage_base: f32,
    pub unarmed_damage_mult: f32,
    pub damage_arm_condition_base: f32,
    pub damage_arm_condition_mult: f32,
    pub sneak_mult_gun: f32,
    pub sneak_mult_melee: f32,
    // Critical Hit Chance Formula
    pub crit_luck_base: f32,
    pub crit_luck_mult: f32,
    pub vats_crit_bonus: f32,
    // Armor
    pub max_armor_rating: f32,
    /// Melee reach base (× actor scale × weapon reach). The wiki gives no default and the
    /// masters lack the setting: 150 units is a placeholder [hypothesis: chosen so a
    /// knife (reach 0.5) reaches 75 units and a sledgehammer (0.75) 112; tunable].
    pub combat_distance: f32,
    // Gun Spread Formula (New Vegas)
    pub spread_unaimed: f32,
    pub spread_walking: f32,
    pub spread_running: f32,
    pub spread_standing: f32,
    pub spread_crippled_arm_1h: f32,
    pub spread_crippled_arm_2h: f32,
    pub spread_crippled_arms_1h: f32,
    pub spread_crippled_arms_2h: f32,
    pub wobble_to_skill: f32,
    pub min_gun_spread: f32,
    pub strength_req_penalty: f32,
    pub skill_req_penalty: f32,
    pub npc_max_gun_wobble_angle: f32,
    // Explosion Knockdown Formula
    pub knockdown_damage_mult: f32,
    pub knockdown_damage_base: f32,
    pub knockdown_agility_mult: f32,
    pub knockdown_agility_base: f32,
    pub knockdown_chance_max: f32,
    pub knockdown_current_health_threshold: f32,
    pub knockdown_base_health_threshold: f32,
    // Melee Knockback Formula
    pub knockback_damage_mult: f32,
    pub knockback_damage_base: f32,
    pub knockback_agility_mult: f32,
    pub knockback_agility_base: f32,
    pub knockback_force_max: f32,
    pub knockback_time: f32,
    // Gun Shell Settings
    pub shell_eject_speed: f32,
    pub shell_lifetime: f32,
    // Item condition (docs/spec-notes/combat-rules.md §17)
    /// Health points a weapon loses per attack.
    pub damage_to_weapon_value: f32,
    /// The share of a hit's damage the armour struck loses as health points.
    pub damage_to_armor_percentage: f32,
    // Power attacks (crate::melee): the damage multiplier of a power attack standing,
    // moving forward, back and to a side [hypothesis: the setting names].
    pub power_attack_stand: f32,
    pub power_attack_forward: f32,
    pub power_attack_back: f32,
    pub power_attack_side: f32,
    // Movement
    /// The share of their speed an actor loses running with a two-handed weapon drawn
    /// (`fMove2HBigPenalty`, 0.1 in FalloutNV.esm [observed]) [hypothesis: the setting's
    /// role, from its name; design: applied to every two-handed weapon, the owner's
    /// playtest list 5 item 61].
    pub move_2h_penalty: f32,
    /// The attack AP cost of each kind of weapon without its own (`fActionPointsAttack…`;
    /// VATS mode, docs/design/vats-mode.md §7.1).
    pub ap_attack: crate::vats::ApAttack,
    /// `fActionPointsRestoreRate`: spent AP comes back at this share of the maximum a
    /// second [hypothesis, vats-mode.md §7.2].
    pub ap_restore_rate: f32,
    /// Settings not found in the load order (their defaults were used).
    pub defaulted: Vec<&'static str>,
}

/// (setting, documented default). Defaults from the GECK wiki pages: Weapon Damage
/// Formula, Critical Hit Chance Formula, Gun Spread Formula (New Vegas), Explosion
/// Knockdown Formula, Melee Knockback Formula, Gun Shell Settings, Armor (owner exports
/// 2026-10-09). `fAVDUnarmedDamage*` have no default on the page; the masters set them.
/// `fDamageToWeaponValue`, `fDamageToArmorPercentage` and the `fDamagePowerAttack…Bonus`
/// settings have no page: their defaults are FalloutNV.esm's values [observed].
const SETTINGS: &[(&str, f32)] = &[
    ("fDamageWeaponMult", 1.0),
    ("fDamageSkillBase", 0.5),
    ("fDamageSkillMult", 0.5),
    ("fAVDMeleeDamageStrengthMult", 0.5),
    ("fAVDMeleeDamageStrengthOffset", 0.0),
    ("fAVDUnarmedDamageBase", 0.5),
    ("fAVDUnarmedDamageMult", 0.05),
    ("fDamageArmConditionBase", 0.2),
    ("fDamageArmConditionMult", 0.8),
    ("fCombatDamageBonusSneakingMult", 2.0),
    ("fCombatDamageBonusMeleeSneakingMult", 5.0),
    ("fAVDCritLuckBase", 0.0),
    ("fAVDCritLuckMult", 1.0),
    ("fVATSCriticalChanceBonus", 15.0),
    ("fMaxArmorRating", 85.0),
    ("fCombatDistance", 150.0),
    ("fUnaimedSpreadPenalty", 0.2),
    ("fWalkingSpreadPenalty", 0.1),
    ("fRunningSpreadPenalty", 0.2),
    ("fStandingSpreadPenalty", 0.1),
    ("fCrippledArm1HSpreadPenalty", 0.2),
    ("fCrippledArm2HSpreadPenalty", 0.4),
    ("fCrippledArms1HSpreadPenalty", 0.4),
    ("fCrippledArms2HSpreadPenalty", 0.6),
    ("fWobbleToSkillConversion", 0.5),
    ("fMinGunSpreadValue", 0.01),
    ("fWeapStrengthReqPenalty", 0.025),
    ("fWeapSkillReqPenalty", 0.01),
    ("fNPCMaxGunWobbleAngle", 15.0),
    ("fKnockdownDamageMult", 0.3),
    ("fKnockdownDamageBase", 0.0),
    ("fKnockdownAgilMult", 1.0),
    ("fKnockdownAgilBase", 0.0),
    ("fKnockdownChance", 0.25),
    ("fKnockdownCurrentHealthThreshold", 50.0),
    ("fKnockdownBaseHealthThreshold", 75.0),
    ("fKnockbackDamageMult", 10.0),
    ("fKnockbackDamageBase", 50.0),
    ("fKnockbackAgilMult", -0.008),
    ("fKnockbackAgilBase", 1.0),
    ("fKnockbackForceMax", 512.0),
    ("fKnockbackTime", 1.0),
    ("fGunShellEjectSpeed", 160.0),
    ("fGunShellLifetime", 5.0),
    ("fDamageToWeaponValue", 0.2),
    ("fDamageToArmorPercentage", 0.35),
    ("fDamagePowerAttackStandBonus", 3.0),
    ("fDamagePowerAttackForwardBonus", 2.0),
    ("fDamagePowerAttackBackBonus", 2.0),
    ("fDamagePowerAttackSideBonus", 2.0),
    ("fMove2HBigPenalty", 0.1),
    // VATS mode's AP (docs/design/vats-mode.md §7): no wiki defaults; FalloutNV.esm's
    // values [observed].
    ("fActionPointsAttackPistol", 20.0),
    ("fActionPointsAttackRifle", 25.0),
    ("fActionPointsAttackHandle", 20.0),
    ("fActionPointsAttackLauncher", 40.0),
    ("fActionPointsAttackOneHandMelee", 15.0),
    ("fActionPointsAttackTwoHandMelee", 30.0),
    ("fActionPointsAttackUnarmed", 22.0),
    ("fActionPointsAttackGrenade", 35.0),
    ("fActionPointsAttackMine", 10.0),
    ("fActionPointsRestoreRate", 0.06),
];

impl Rules {
    /// The documented defaults only (no load order).
    pub fn defaults() -> Rules {
        Rules::with(|_| None)
    }

    /// The load order's settings, defaults where it has none.
    pub fn from_forms(c: &CombatForms) -> Rules {
        Rules::with(|name| c.setting(name))
    }

    fn with(get: impl Fn(&str) -> Option<f32>) -> Rules {
        let mut defaulted = Vec::new();
        let v: Vec<f32> = SETTINGS
            .iter()
            .map(|&(name, d)| {
                get(name).unwrap_or_else(|| {
                    defaulted.push(name);
                    d
                })
            })
            .collect();
        let mut it = v.into_iter();
        let mut n = || it.next().expect("one value per setting");
        Rules {
            damage_weapon_mult: n(),
            damage_skill_base: n(),
            damage_skill_mult: n(),
            melee_damage_strength_mult: n(),
            melee_damage_strength_offset: n(),
            unarmed_damage_base: n(),
            unarmed_damage_mult: n(),
            damage_arm_condition_base: n(),
            damage_arm_condition_mult: n(),
            sneak_mult_gun: n(),
            sneak_mult_melee: n(),
            crit_luck_base: n(),
            crit_luck_mult: n(),
            vats_crit_bonus: n(),
            max_armor_rating: n(),
            combat_distance: n(),
            spread_unaimed: n(),
            spread_walking: n(),
            spread_running: n(),
            spread_standing: n(),
            spread_crippled_arm_1h: n(),
            spread_crippled_arm_2h: n(),
            spread_crippled_arms_1h: n(),
            spread_crippled_arms_2h: n(),
            wobble_to_skill: n(),
            min_gun_spread: n(),
            strength_req_penalty: n(),
            skill_req_penalty: n(),
            npc_max_gun_wobble_angle: n(),
            knockdown_damage_mult: n(),
            knockdown_damage_base: n(),
            knockdown_agility_mult: n(),
            knockdown_agility_base: n(),
            knockdown_chance_max: n(),
            knockdown_current_health_threshold: n(),
            knockdown_base_health_threshold: n(),
            knockback_damage_mult: n(),
            knockback_damage_base: n(),
            knockback_agility_mult: n(),
            knockback_agility_base: n(),
            knockback_force_max: n(),
            knockback_time: n(),
            shell_eject_speed: n(),
            shell_lifetime: n(),
            damage_to_weapon_value: n(),
            damage_to_armor_percentage: n(),
            power_attack_stand: n(),
            power_attack_forward: n(),
            power_attack_back: n(),
            power_attack_side: n(),
            move_2h_penalty: n(),
            ap_attack: crate::vats::ApAttack {
                pistol: n(),
                rifle: n(),
                handle: n(),
                launcher: n(),
                one_hand_melee: n(),
                two_hand_melee: n(),
                unarmed: n(),
                grenade: n(),
                mine: n(),
            },
            ap_restore_rate: n(),
            defaulted,
        }
    }
}

/// NVForever's own combat numbers [design]. Each says what it shapes.
#[derive(Debug, Clone, Serialize)]
pub struct CombatTuning {
    /// Armour never takes more than this share of a hit away (DT floor, §5.3).
    pub dt_floor: f32,
    /// Damage multiplier of a weapon at zero condition; full condition is 1, linear
    /// between (the original's rule is hard-coded and undocumented).
    pub condition_min_mult: f32,
    /// A headshot on a target that is unaware of the attacker is always a critical.
    pub headshot_unaware_crit: bool,
    /// Share of its DT and DR that armour at zero condition still gives; full condition
    /// gives all of it, linear between ([`crate::condition::armor_mult`]).
    pub armor_condition_min_mult: f32,
    /// Spread added per shot, as a share of the weapon's Min Spread (in spread units).
    pub bloom_per_shot: f32,
    /// Bloom (spread units) recovered per second.
    pub bloom_recovery: f32,
    /// Bloom cap (spread units).
    pub bloom_max: f32,
    /// Actor-spread penalty while airborne (spread units, like the documented penalties).
    pub jump_spread: f32,
    /// View kick per shot in degrees: base plus per point of damage per shot.
    pub recoil_base_deg: f32,
    pub recoil_per_damage_deg: f32,
    /// View kick recovery, degrees per second.
    pub recoil_recovery_deg: f32,
    /// Melee damage window after the `Hit` key, seconds.
    pub melee_window: f32,
    /// Rays across a melee swing, and the swing's half-angle in degrees.
    pub melee_rays: u32,
    pub melee_arc_deg: f32,
    /// Explosion damage at the edge of the radius as a share of the centre's.
    pub explosion_edge: f32,
    /// Bounce multiplier for lobbers whose PROJ gives none.
    pub default_bounce: f32,
    /// Reload when the trigger is pulled on an empty magazine and ammo is carried.
    pub auto_reload: bool,
    /// Reload time when no reload clip length is known, seconds.
    pub fallback_reload: f32,
    /// Melee (step C6): holding Use this long starts a power attack (`crate::melee`); a
    /// power attack with no clip to time it takes this much longer than a swing.
    pub power_hold: f32,
    pub power_time: f32,
    /// Share of a melee hit a block stops.
    pub block: f32,
    /// Seconds the player cannot block after a power attack broke its block
    /// ([`crate::damage::against_block`]).
    pub guard_break: f32,
    /// Seconds after an actor's hit reaction ends before a hit on a crippled body part
    /// starts another, so that fast hits cannot hold it in the reaction
    /// ([`crate::damage::staggers`]).
    pub stagger_cooldown: f32,
    /// Weapon jamming (the owner's playtest list 5 item 9; [`crate::condition::jam_chance`]):
    /// below this condition (a share of full) a shot may jam the weapon.
    pub jam_condition: f32,
    /// The chance a shot jams at zero condition; it grows from 0 at
    /// [`Self::jam_condition`] to this, along [`Self::jam_curve`]. Kept low: the owner
    /// tunes it after play sessions.
    pub jam_chance_max: f32,
    /// The power the shortfall below [`Self::jam_condition`] is raised to (1: linear;
    /// above 1: rare until the weapon is well worn).
    pub jam_curve: f32,
    /// How long a jam keeps the weapon from firing when no jam clip is known, seconds.
    pub jam_fallback: f32,
}

impl Default for CombatTuning {
    fn default() -> Self {
        CombatTuning {
            dt_floor: 0.2,
            condition_min_mult: 0.5,
            headshot_unaware_crit: true,
            armor_condition_min_mult: 0.5,
            bloom_per_shot: 0.03,
            bloom_recovery: 1.0,
            bloom_max: 0.25,
            jump_spread: 0.3,
            recoil_base_deg: 0.4,
            recoil_per_damage_deg: 0.03,
            recoil_recovery_deg: 12.0,
            melee_window: 0.1,
            melee_rays: 5,
            melee_arc_deg: 35.0,
            explosion_edge: 0.25,
            default_bounce: 0.3,
            auto_reload: true,
            fallback_reload: 2.0,
            power_hold: 0.4,
            power_time: 1.3,
            block: 0.6,
            guard_break: 1.0,
            stagger_cooldown: 3.0,
            jam_condition: 0.75,
            jam_chance_max: 0.04,
            jam_curve: 1.5,
            jam_fallback: 1.5,
        }
    }
}
