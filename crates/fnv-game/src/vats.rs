//! VATS mode as the scripts see it (docs/design/vats-mode.md §12). NVForever's VATS mode
//! is its own Dead Eye-style mark and fire, not the original's VATS [design]; the engine
//! (`nvf-play` `vats_mode`) tells the runtime each frame which state it is in and which
//! marked shot is leaving ([`Game::set_vats`]), and the VATS functions answer from that:
//!
//! - **GetVATSMode** [documented: GECK wiki GetVATSMode, return values 0 none, 1 target
//!   select, 2 menu, 3 scan, 4 playback]: 0 off, 1 while marking, 4 while the marked
//!   shots are fired [design: the nearest of the original's views; 2 and 3 never].
//! - **GetVATSValue** [documented: GECK wiki GetVATSValue, its list of queries]: about the
//!   shot leaving now while firing, 0 otherwise. The query codes are the page's order
//!   from 0 [hypothesis]; what each returns is in [`get_vats_value`].
//!
//! Not part of the saved state: the mode is never saved (vats-mode.md §11.7).

use crate::functions::{base_of, list_members, Args, Ctx};
use crate::game::Game;
use fnv_script::vm::{ArgValue, Value};
use fnv_sim::FormId;

/// GetVATSMode's values for VATS mode's states.
pub const MODE_OFF: u8 = 0;
pub const MODE_TARGET_SELECT: u8 = 1;
pub const MODE_PLAYBACK: u8 = 4;

/// The marked shot leaving now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VatsShot {
    pub weapon: FormId,
    /// The marked actor's reference.
    pub target: FormId,
    /// From the player to the target, game units.
    pub distance: f64,
    /// The marked body part's condition actor value (25–31; its BPTD part's Actor
    /// Value), `None` for a whole actor or a part without one.
    pub condition: Option<u16>,
}

/// What the engine says of VATS mode.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VatsState {
    pub mode: u8,
    pub shot: Option<VatsShot>,
}

impl Game<'_> {
    /// The engine's VATS mode state and the shot leaving now (only while firing).
    pub fn set_vats(&mut self, mode: u8, shot: Option<VatsShot>) {
        self.vats = VatsState { mode, shot: shot.filter(|_| mode == MODE_PLAYBACK) };
    }
}

pub fn get_vats_mode(g: &mut Game, _ctx: &Ctx, _a: &Args) -> Result<Value, String> {
    Ok(Value::Num(g.vats.mode as f64))
}

/// The query's parameter as a form: a reference value from a script, or the raw FormID a
/// condition's `ptVATSValueParam` carries as an integer (`crate::cond`), taken as a
/// load-order FormID [hypothesis: right for FalloutNV.esm's records, whose local and
/// load-order FormIDs agree].
fn param_form(a: &Args) -> Option<FormId> {
    a.opt_form(1).or_else(|| match a.get(1)? {
        ArgValue::Value(v) => Some(FormId(v.num() as i64 as u32)).filter(|f| f.0 != 0),
        ArgValue::Enum(c) => Some(FormId(*c as u32)),
        _ => None,
    })
}

/// The query's parameter as a number (an animation type, an actor value, an action).
fn param_num(a: &Args) -> Option<i64> {
    match a.get(1)? {
        ArgValue::Enum(c) => Some(*c as i64),
        ArgValue::Value(v) => Some(v.num() as i64),
        _ => None,
    }
}

/// GetVATSValue `query` [`parameter`], while a marked shot leaves (0 otherwise):
///
/// | Code | Query | Returns |
/// |---|---|---|
/// | 0 | WeaponIs | 1 if the shot's weapon is the parameter |
/// | 1 | Weapon In List | 1 if the weapon is in the list |
/// | 2 | Weapon Type Is | 1 if the weapon's DNAM animation type is the parameter |
/// | 3 | Target Is | 1 if the target, or its base, is the parameter |
/// | 4 | Target In List | 1 if the target or its base is in the list |
/// | 5 | Target Distance | units from the player to the target |
/// | 6 | Targeted Part | 1 if the marked part's condition actor value is the parameter |
/// | 7 | VATS Action | 1 for action 0 [hypothesis: Attack is the first action] |
/// | 8 | Is Success | 1: VATS mode's shots never miss (owner) [design] |
/// | 9–15 | critical, fatal, dismemberment, crippling | 0: not known before the shot lands [design] |
pub fn get_vats_value(g: &mut Game, _ctx: &Ctx, a: &Args) -> Result<Value, String> {
    let Some(s) = g.vats.shot else { return Ok(Value::ZERO) };
    let query = a.code(0)?;
    let in_list = |g: &Game, list: Option<FormId>, f: &[FormId]| list.is_some_and(|l| list_members(g, l).iter().any(|m| f.contains(m)));
    let target = [Some(s.target), base_of(g, s.target)].into_iter().flatten().collect::<Vec<_>>();
    let b = match query {
        0 => param_form(a) == Some(s.weapon),
        1 => in_list(g, param_form(a), &[s.weapon]),
        2 => param_num(a).is_some_and(|t| g.data.weapon_anim_types.get(&s.weapon) == Some(&t)),
        3 => param_form(a).is_some_and(|f| target.contains(&f)),
        4 => in_list(g, param_form(a), &target),
        5 => return Ok(Value::Num(s.distance)),
        6 => param_num(a).is_some_and(|c| s.condition.map(i64::from) == Some(c)),
        7 => param_num(a).unwrap_or(0) == 0,
        8 => true,
        _ => false,
    };
    Ok(Value::bool(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conditions_raw_formid_or_a_scripts_reference_is_a_form() {
        // A condition's ptVATSValueParam arrives as an integer (crate::cond).
        let raw = [ArgValue::Enum(0), ArgValue::Value(Value::Num(0x0008F216 as f64))];
        assert_eq!(param_form(&Args(&raw)), Some(FormId(0x0008F216)));
        let script = [ArgValue::Enum(0), ArgValue::Value(Value::Ref(0x0008F216))];
        assert_eq!(param_form(&Args(&script)), Some(FormId(0x0008F216)));
        let none = [ArgValue::Enum(0), ArgValue::Value(Value::Num(0.0))];
        assert_eq!(param_form(&Args(&none)), None);
        assert_eq!(param_form(&Args(&[ArgValue::Enum(5)])), None);
    }

    #[test]
    fn numbers_come_as_codes_or_values() {
        assert_eq!(param_num(&Args(&[ArgValue::Enum(6), ArgValue::Enum(25)])), Some(25));
        assert_eq!(param_num(&Args(&[ArgValue::Enum(2), ArgValue::Value(Value::Num(3.0))])), Some(3));
        assert_eq!(param_num(&Args(&[ArgValue::Enum(7)])), None);
    }
}
