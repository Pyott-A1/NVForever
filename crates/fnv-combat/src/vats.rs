//! VATS mode's headless rules (docs/design/vats-mode.md): NVForever's own Dead Eye-style
//! VATS, not the original's [design: combat is not a parity target, combat.md §1].
//!
//! - [`shot_ap_cost`]: a shot's AP cost from the records (§7.1): the weapon's "Override -
//!   Action Points" (WEAP `DNAM`) when it has one, else the attack setting of its
//!   animation type ([`ApAttack`], `fActionPointsAttack…`) [hypothesis: which setting each
//!   type uses, from the names].
//! - [`MarkQueue`]: the marks in the order they were made, each reserving its AP; no more
//!   than the AP left and the rounds loaded allow (§7.2).
//! - [`Volley`]: the marked shots leaving one after another at a fixed interval, then the
//!   end of the mode a moment after the last (§9.1).
//! - [`restored_damage`]: spent AP coming back outside the mode (§7.2) [hypothesis].

use fnv_sim::FormId;
use serde::Serialize;

use crate::data::WeaponStats;

/// The attack AP cost of each kind of weapon (`fActionPointsAttack…`), for weapons
/// without their own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ApAttack {
    pub pistol: f32,
    pub rifle: f32,
    pub handle: f32,
    pub launcher: f32,
    pub one_hand_melee: f32,
    pub two_hand_melee: f32,
    pub unarmed: f32,
    pub grenade: f32,
    pub mine: f32,
}

/// The AP one shot of `w` costs: its override when the record sets the flag, else its
/// animation type's attack setting. An unknown type costs as a rifle [design].
pub fn shot_ap_cost(w: &WeaponStats, a: &ApAttack) -> f32 {
    if let Some(ap) = w.action_points {
        return ap.max(0.0);
    }
    match w.anim_type.as_deref() {
        Some("Hand to Hand") => a.unarmed,
        Some("Melee (1 Hand)") => a.one_hand_melee,
        Some("Melee (2 Hand)") => a.two_hand_melee,
        Some("Pistol - Ballistic (1 Hand)") | Some("Pistol - Energy (1 Hand)") => a.pistol,
        Some("Handle (2 Hand)") => a.handle,
        Some("Launcher (2 Hand)") => a.launcher,
        Some("Grenade Throw (1 Hand)") | Some("Thrown (1 Hand)") => a.grenade,
        Some("Land Mine (1 Hand)") | Some("Mine Drop (1 Hand)") => a.mine,
        _ => a.rifle,
    }
}

/// One marked shot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Mark {
    /// The actor's reference.
    pub target: FormId,
    /// The body part (its index in the actor's BPTD), `None` for the whole actor.
    pub part: Option<u8>,
    /// The AP it reserves.
    pub cost: f32,
}

/// Why a mark was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Refusal {
    /// The AP left after the reservations does not cover another shot.
    ActionPoints,
    /// The rounds loaded do not cover another shot.
    Rounds,
}

impl Refusal {
    /// The HUD's line.
    pub fn message(self) -> &'static str {
        match self {
            Refusal::ActionPoints => "Not enough action points.",
            Refusal::Rounds => "No more rounds loaded.",
        }
    }
}

/// Lets two reservations that add up to the AP exactly pass despite rounding.
const AP_EPSILON: f32 = 1e-3;

/// The marks of one stay in VATS mode, in order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MarkQueue {
    marks: Vec<Mark>,
    /// AP per shot.
    pub cost: f32,
    /// Rounds one shot takes from the magazine (the weapon's Ammo Use, at least 1).
    pub rounds_per_shot: u32,
    /// Rounds in the magazine.
    pub rounds: u32,
}

impl MarkQueue {
    pub fn new(cost: f32, rounds_per_shot: u32, rounds: u32) -> MarkQueue {
        MarkQueue { marks: Vec::new(), cost: cost.max(0.0), rounds_per_shot: rounds_per_shot.max(1), rounds }
    }

    /// Whether one more mark fits with `ap_now` AP (current, nothing spent yet): the AP
    /// is asked first.
    pub fn check(&self, ap_now: f32) -> Result<(), Refusal> {
        if self.reserved() + self.cost > ap_now + AP_EPSILON {
            return Err(Refusal::ActionPoints);
        }
        if self.rounds_needed() + self.rounds_per_shot > self.rounds {
            return Err(Refusal::Rounds);
        }
        Ok(())
    }

    /// Mark `target` (on `part`): the number of marks after it, or why not.
    pub fn push(&mut self, target: FormId, part: Option<u8>, ap_now: f32) -> Result<usize, Refusal> {
        self.check(ap_now)?;
        self.marks.push(Mark { target, part, cost: self.cost });
        Ok(self.marks.len())
    }

    /// Take the last mark back.
    pub fn pop(&mut self) -> Option<Mark> {
        self.marks.pop()
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// The AP the marks hold.
    pub fn reserved(&self) -> f32 {
        self.marks.iter().map(|m| m.cost).sum()
    }

    /// The rounds the marks need.
    pub fn rounds_needed(&self) -> u32 {
        self.marks.len() as u32 * self.rounds_per_shot
    }

    /// The most marks `ap_now` and the rounds allow at all.
    pub fn most(&self, ap_now: f32) -> u32 {
        let by_rounds = self.rounds / self.rounds_per_shot;
        let by_ap = if self.cost > 0.0 { ((ap_now + AP_EPSILON) / self.cost).floor().max(0.0) as u32 } else { u32::MAX };
        by_rounds.min(by_ap)
    }
}

/// What a [`Volley`] step brings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolleyEvent {
    /// Fire the mark at this index now.
    Shot(usize),
    /// All are fired and the pause after the last is over: the mode ends.
    Done,
}

/// The marked shots leaving at a fixed interval: the first at once, at most one a step
/// (so each shot gets its own frame), then the end `after` seconds after the last.
#[derive(Debug, Clone, PartialEq)]
pub struct Volley {
    pub shots: usize,
    pub fired: usize,
    interval: f32,
    after: f32,
    /// Seconds since the last shot (the start, before the first).
    since: f32,
    done: bool,
}

impl Volley {
    pub fn new(shots: usize, interval: f32, after: f32) -> Volley {
        Volley { shots, fired: 0, interval: interval.max(0.0), after: after.max(0.0), since: f32::INFINITY, done: false }
    }

    pub fn step(&mut self, dt: f32) -> Option<VolleyEvent> {
        if self.done {
            return None;
        }
        self.since += dt.max(0.0);
        if self.fired < self.shots {
            if self.since >= self.interval {
                self.since = 0.0;
                self.fired += 1;
                return Some(VolleyEvent::Shot(self.fired - 1));
            }
            return None;
        }
        if self.since >= self.after || self.shots == 0 {
            self.done = true;
            return Some(VolleyEvent::Done);
        }
        None
    }

    pub fn done(&self) -> bool {
        self.done
    }
}

/// The Action Points damage left after `dt` seconds of regeneration: `rate` × `max` a
/// second comes back, down to none [hypothesis: the role of `fActionPointsRestoreRate`].
pub fn restored_damage(damage: f32, max: f32, rate: f32, dt: f32) -> f32 {
    (damage - max.max(0.0) * rate.max(0.0) * dt.max(0.0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AP: ApAttack =
        ApAttack { pistol: 20.0, rifle: 25.0, handle: 20.0, launcher: 40.0, one_hand_melee: 15.0, two_hand_melee: 30.0, unarmed: 22.0, grenade: 35.0, mine: 10.0 };

    fn weapon(anim: &str, over: Option<f32>) -> WeaponStats {
        WeaponStats { anim_type: Some(anim.into()), action_points: over, ..Default::default() }
    }

    #[test]
    fn the_override_wins_else_the_animation_type() {
        // WeapNV357Revolver: Override - Action Points 26 [observed].
        assert_eq!(shot_ap_cost(&weapon("Pistol - Ballistic (1 Hand)", Some(26.0)), &AP), 26.0);
        assert_eq!(shot_ap_cost(&weapon("Pistol - Ballistic (1 Hand)", None), &AP), 20.0);
        assert_eq!(shot_ap_cost(&weapon("Pistol - Energy (1 Hand)", None), &AP), 20.0);
        assert_eq!(shot_ap_cost(&weapon("Rifle - Automatic (2 Hand)", None), &AP), 25.0);
        assert_eq!(shot_ap_cost(&weapon("Launcher (2 Hand)", None), &AP), 40.0);
        assert_eq!(shot_ap_cost(&weapon("Thrown (1 Hand)", None), &AP), 35.0);
        assert_eq!(shot_ap_cost(&weapon("Mine Drop (1 Hand)", None), &AP), 10.0);
        assert_eq!(shot_ap_cost(&WeaponStats::default(), &AP), 25.0, "no type: as a rifle");
    }

    #[test]
    fn marks_reserve_ap_until_it_runs_out() {
        // 92 AP, 26 a shot: three marks (78), the fourth would need 104.
        let mut q = MarkQueue::new(26.0, 1, 6);
        let t = FormId(0x10);
        assert_eq!(q.push(t, Some(0), 92.0), Ok(1));
        assert_eq!(q.push(t, Some(1), 92.0), Ok(2));
        assert_eq!(q.push(FormId(0x11), None, 92.0), Ok(3));
        assert_eq!(q.reserved(), 78.0);
        assert_eq!(q.push(t, None, 92.0), Err(Refusal::ActionPoints));
        assert_eq!(q.len(), 3);
        assert_eq!(q.most(92.0), 3);
    }

    #[test]
    fn marks_stop_at_the_rounds_loaded() {
        // A .357 with 6 loaded and AP for more: six marks at most (the owner's example).
        let mut q = MarkQueue::new(26.0, 1, 6);
        for i in 0..6 {
            assert_eq!(q.push(FormId(0x10), None, 1000.0), Ok(i + 1));
        }
        assert_eq!(q.push(FormId(0x10), None, 1000.0), Err(Refusal::Rounds));
        assert_eq!(q.most(1000.0), 6);
        // Two rounds a shot: half as many.
        let q = MarkQueue::new(10.0, 2, 5);
        assert_eq!(q.most(1000.0), 2);
    }

    #[test]
    fn exact_ap_is_enough_and_ap_is_asked_first() {
        let mut q = MarkQueue::new(26.0, 1, 1);
        assert_eq!(q.push(FormId(1), None, 26.0), Ok(1));
        // Out of both: the AP is the reason given.
        assert_eq!(q.check(26.0), Err(Refusal::ActionPoints));
        assert_eq!(q.check(100.0), Err(Refusal::Rounds));
    }

    #[test]
    fn unmarking_frees_the_last_mark_and_its_ap() {
        let mut q = MarkQueue::new(20.0, 1, 6);
        q.push(FormId(1), Some(2), 50.0).unwrap();
        q.push(FormId(2), Some(0), 50.0).unwrap();
        assert_eq!(q.push(FormId(3), None, 50.0), Err(Refusal::ActionPoints));
        assert_eq!(q.pop(), Some(Mark { target: FormId(2), part: Some(0), cost: 20.0 }));
        assert_eq!(q.reserved(), 20.0);
        assert_eq!(q.push(FormId(3), None, 50.0), Ok(2));
        assert_eq!(q.marks().iter().map(|m| m.target).collect::<Vec<_>>(), vec![FormId(1), FormId(3)]);
        q.pop();
        q.pop();
        assert!(q.pop().is_none() && q.is_empty() && q.reserved() == 0.0);
    }

    #[test]
    fn the_volley_fires_in_order_at_the_interval_then_ends() {
        let mut v = Volley::new(3, 0.12, 0.3);
        let mut events = Vec::new();
        let mut t = 0.0;
        while !v.done() && t < 5.0 {
            if let Some(e) = v.step(0.01) {
                events.push((e, (t * 100.0f32).round() / 100.0));
            }
            t += 0.01;
        }
        let shots: Vec<_> = events.iter().filter_map(|(e, t)| matches!(e, VolleyEvent::Shot(_)).then_some(*t)).collect();
        assert_eq!(events.iter().filter_map(|(e, _)| if let VolleyEvent::Shot(i) = e { Some(*i) } else { None }).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(shots[0], 0.0, "the first shot at once");
        assert!((shots[1] - 0.12).abs() < 0.015 && (shots[2] - 0.24).abs() < 0.025, "{shots:?}");
        let done = events.last().unwrap();
        assert_eq!(done.0, VolleyEvent::Done);
        assert!((done.1 - 0.54).abs() < 0.03, "{done:?}");
    }

    #[test]
    fn the_volley_fires_once_a_step_however_long_the_step() {
        let mut v = Volley::new(2, 0.12, 0.3);
        assert_eq!(v.step(1.0), Some(VolleyEvent::Shot(0)));
        assert_eq!(v.step(1.0), Some(VolleyEvent::Shot(1)));
        assert_eq!(v.step(1.0), Some(VolleyEvent::Done));
        assert_eq!(v.step(1.0), None);
    }

    #[test]
    fn spent_ap_comes_back_at_the_rate() {
        // 92 AP at 0.06 a second: 5.52 AP a second.
        assert!((restored_damage(52.0, 92.0, 0.06, 1.0) - 46.48).abs() < 1e-4);
        assert_eq!(restored_damage(3.0, 92.0, 0.06, 1.0), 0.0);
        assert_eq!(restored_damage(0.0, 92.0, 0.06, 1.0), 0.0);
    }
}
