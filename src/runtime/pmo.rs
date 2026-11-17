//! Platform Movement object ("Platform" extension): moves one object like a platformer
//! character. The game's own events decide what is solid: while the object tests a
//! position, the events whose condition is "obstacle collision test" run straight away,
//! and their "set obstacle overlap" action reports that the position is blocked.
//!
//! Inferred from how games use it and the object's stored settings (nine numbers, in
//! order: max X speed, X acceleration, X deceleration, gravity, jump strength, max Y
//! speed, jump hold, step up, slope correction). Speeds are hundredths of a pixel per
//! tick; acceleration, deceleration and gravity add a tenth of their value per tick.

use super::Input;
use super::Runtime;

#[derive(Debug, Clone, Default)]
pub(crate) struct Pmo {
    /// The object it moves.
    pub target: Option<u16>,
    pub max_vx: f64,
    pub accel: f64,
    pub decel: f64,
    pub gravity: f64,
    pub jump: f64,
    pub max_vy: f64,
    pub jump_hold: u32,
    pub step_up: u32,
    pub slope: u32,
    pub vx: f64,
    pub vy: f64,
    /// Sub-pixel movement carried to the next tick.
    rem: (f64, f64),
    pub on_ground: bool,
    /// Asked for by the events since the last tick.
    pub right: bool,
    pub left: bool,
    pub jump_pressed: bool,
    hold_left: u32,
    pub paused: bool,
    /// Set by "set obstacle overlap" during a collision test.
    pub overlap: bool,
}

impl Pmo {
    /// Read the stored settings: an 8-byte header, then 16-byte text fields.
    pub fn from_data(data: Option<&[u8]>) -> Self {
        let mut n = [120.0, 800.0, 150.0, 150.0, 23.0, 400.0, 13.0, 4.0, 3.0];
        if let Some(d) = data {
            for (i, f) in d.get(8..).unwrap_or(&[]).chunks(16).take(9).enumerate() {
                let text: String = f.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
                if let Ok(v) = text.trim().parse::<f64>() {
                    n[i] = v;
                }
            }
        }
        Pmo {
            max_vx: n[0],
            accel: n[1],
            decel: n[2],
            gravity: n[3],
            jump: n[4],
            max_vy: n[5],
            jump_hold: n[6].max(0.0) as u32,
            step_up: n[7].max(0.0) as u32,
            slope: n[8].max(0.0) as u32,
            ..Default::default()
        }
    }

    pub fn jumping(&self) -> bool {
        !self.on_ground && self.vy < 0.0
    }

    pub fn falling(&self) -> bool {
        !self.on_ground && self.vy > 0.0
    }

    pub fn moving(&self) -> bool {
        self.vx != 0.0
    }

    /// The new X speed from this tick's input.
    fn steer(&mut self) {
        let dir = self.right as i32 - self.left as i32;
        if dir != 0 {
            self.vx = (self.vx + dir as f64 * self.accel / 10.0).clamp(-self.max_vx, self.max_vx);
        } else {
            let d = self.decel / 10.0;
            self.vx = if self.vx.abs() <= d { 0.0 } else { self.vx - d * self.vx.signum() };
        }
        // Over the maximum (it was lowered): slow down to it.
        if self.vx.abs() > self.max_vx {
            self.vx = self.max_vx * self.vx.signum();
        }
    }

    fn fall(&mut self) {
        if self.jump_pressed && self.on_ground {
            self.vy = -self.jump * 10.0;
            self.on_ground = false;
            self.hold_left = self.jump_hold;
        }
        // The first ticks of a jump go up at full speed ("jump hold"): games have no
        // separate "jump held" action, and their obstacles (40 pixels for a 40-pixel
        // character) need the height.
        if self.vy < 0.0 && self.hold_left > 0 {
            self.hold_left -= 1;
            return;
        }
        self.hold_left = 0;
        self.vy = (self.vy + self.gravity / 10.0).min(self.max_vy);
    }
}

impl Runtime {
    /// The Platform Movement object `oi`'s state, made from its settings the first time.
    pub(crate) fn pmo(&mut self, oi: u16) -> &mut Pmo {
        if !self.pmo.contains_key(&oi) {
            let data = match self.game.object(oi).map(|o| &o.props) {
                Some(crate::ctf_loader::ObjectProperties::Common(c)) => c.extension_data.as_deref(),
                _ => None,
            };
            let p = Pmo::from_data(data);
            self.pmo.insert(oi, p);
        }
        self.pmo.get_mut(&oi).expect("inserted")
    }

    /// Is the moved object blocked where it is now? Runs the game's collision-test events.
    fn pmo_blocked(&mut self, oi: u16, input: &Input) -> bool {
        self.pmo(oi).overlap = false;
        self.pmo_testing = Some(oi);
        self.run_pmo_tests(oi, input);
        self.pmo_testing = None;
        self.pmo(oi).overlap
    }

    /// Move every Platform Movement object's object for this tick.
    pub(crate) fn update_pmo(&mut self, input: &Input) {
        let mut ois: Vec<u16> = self.pmo.keys().copied().collect();
        ois.sort_unstable();
        for oi in ois {
            let p = self.pmo(oi);
            let Some(target) = p.target else { continue };
            if p.paused {
                p.right = false;
                p.left = false;
                p.jump_pressed = false;
                continue;
            }
            let Some(&idx) = self.instances_of(target).first() else { continue };
            {
                let p = self.pmo(oi);
                p.steer();
                p.fall();
            }
            let (vx, vy, rem) = {
                let p = self.pmo(oi);
                (p.vx, p.vy, p.rem)
            };
            let fx = vx / 100.0 + rem.0;
            let fy = vy / 100.0 + rem.1;
            let (sx, sy) = (fx.trunc() as i64, fy.trunc() as i64);
            self.pmo(oi).rem = (fx - sx as f64, fy - sy as f64);
            let (step_up, slope) = {
                let p = self.pmo(oi);
                (p.step_up as i64, p.slope as i64)
            };
            let was_on_ground = self.pmo(oi).on_ground;

            // Across, a pixel at a time, stepping up small ledges.
            let dir = sx.signum() as f64;
            for _ in 0..sx.abs() {
                self.instances[idx].x += dir;
                if !self.pmo_blocked(oi, input) {
                    continue;
                }
                let mut climbed = false;
                for up in 1..=step_up {
                    self.instances[idx].y -= 1.0;
                    if !self.pmo_blocked(oi, input) {
                        climbed = true;
                        break;
                    }
                    if up == step_up {
                        self.instances[idx].y += step_up as f64;
                    }
                }
                if !climbed {
                    self.instances[idx].x -= dir;
                    let p = self.pmo(oi);
                    p.vx = 0.0;
                    p.rem.0 = 0.0;
                    break;
                }
            }
            // Walking down a slope: stay on it.
            if was_on_ground && vy >= 0.0 && sx != 0 {
                let mut down = 0;
                while down < slope {
                    self.instances[idx].y += 1.0;
                    if self.pmo_blocked(oi, input) {
                        self.instances[idx].y -= 1.0;
                        break;
                    }
                    down += 1;
                }
                if down == slope && slope > 0 {
                    // Nothing below within reach: it walked off an edge.
                    self.instances[idx].y -= slope as f64;
                }
            }
            // Up or down.
            let dir = sy.signum() as f64;
            for _ in 0..sy.abs() {
                self.instances[idx].y += dir;
                if self.pmo_blocked(oi, input) {
                    self.instances[idx].y -= dir;
                    let p = self.pmo(oi);
                    p.vy = 0.0;
                    p.rem.1 = 0.0;
                    break;
                }
            }
            // On the ground if one pixel down is blocked.
            self.instances[idx].y += 1.0;
            let ground = self.pmo_blocked(oi, input);
            self.instances[idx].y -= 1.0;
            let p = self.pmo(oi);
            p.on_ground = ground && p.vy >= 0.0;
            if p.on_ground {
                p.vy = 0.0;
                p.rem.1 = 0.0;
            }
            p.right = false;
            p.left = false;
            p.jump_pressed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_read() {
        let mut d = vec![0u8; 8];
        for v in ["120", "800", "150", "150", "23", "400", "13", "4", "3"] {
            let mut f = v.as_bytes().to_vec();
            f.resize(16, 0);
            d.extend(f);
        }
        let p = Pmo::from_data(Some(&d));
        assert_eq!((p.max_vx, p.jump, p.step_up, p.slope), (120.0, 23.0, 4, 3));
    }

    #[test]
    fn walking_speeds_up_and_stops() {
        let mut p = Pmo::from_data(None);
        p.right = true;
        p.steer();
        assert_eq!(p.vx, 80.0);
        p.steer();
        assert_eq!(p.vx, 120.0);
        p.right = false;
        for _ in 0..8 {
            p.steer();
        }
        assert_eq!(p.vx, 0.0);
    }

    #[test]
    fn jumping_needs_the_ground() {
        let mut p = Pmo::from_data(None);
        p.jump_pressed = true;
        p.fall();
        assert!(p.vy > 0.0, "no jump in the air");
        p.vy = 0.0;
        p.on_ground = true;
        p.fall();
        assert_eq!(p.vy, -230.0);
        assert!(p.jumping());
    }
}
