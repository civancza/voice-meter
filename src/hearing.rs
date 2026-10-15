//! What you hear: the level of everything apps play, adjusted by the default output volume.
//!
//! [`Monitor`] does the I/O (process tap + volume/mute properties); [`Ear`] is the pure model
//! that smooths the level and decides colours and labels, like `meter::Meter` does for the mic.

mod monitor;
mod tap;

use crate::meter::{self, DB_TEXT_EVERY_TICKS, Mood, SILENCE_LABEL_AT_DB, SMOOTH_FLOOR_DB};
pub use monitor::Monitor;

/// Effective output level (signal dBFS + volume dB). Absolute loudness depends on the speakers or
/// headphones, so these are conservative rules of thumb: full-scale music at max volume is red.
pub const CAREFUL_DB: f32 = -30.0;
pub const LOUD_DB: f32 = -20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    /// No default output, or system audio capture is not possible.
    Unavailable,
    Muted,
    Level(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Unavailable,
    Muted,
    Db(i32),
    Silent,
}

impl Label {
    fn level(db: f32) -> Self {
        if db <= SILENCE_LABEL_AT_DB {
            Self::Silent
        } else {
            Self::Db(db.round() as i32)
        }
    }

    pub fn text(self) -> String {
        match self {
            Self::Unavailable => "—".into(),
            Self::Muted => "MUTED".into(),
            Self::Db(db) => meter::db_label(db as f32),
            Self::Silent => meter::db_label(meter::MIN_DB),
        }
    }
}

/// Everything the HUD's 👂 row draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub fill: f64,
    pub mood: Mood,
    pub label: Label,
}

/// Smooths the hearing level with the mic's attack/release and throttles the dB label the same way.
pub struct Ear {
    smooth: f32,
    tick: u64,
    label: Label,
}

impl Ear {
    pub fn new() -> Self {
        Self {
            smooth: SMOOTH_FLOOR_DB,
            tick: 0,
            label: Label::Unavailable,
        }
    }

    pub fn step(&mut self, reading: Reading) -> Frame {
        self.tick += 1;
        let db = match reading {
            Reading::Level(db) => db,
            Reading::Muted => return self.reset(Label::Muted),
            Reading::Unavailable => return self.reset(Label::Unavailable),
        };
        self.smooth = meter::ease(self.smooth, db);
        // Show the level immediately when sound returns, then at the mic's label cadence.
        let fresh = matches!(self.label, Label::Unavailable | Label::Muted);
        if fresh || self.tick.is_multiple_of(DB_TEXT_EVERY_TICKS) {
            self.label = Label::level(self.smooth);
        }
        Frame {
            fill: meter::db_frac(self.smooth),
            mood: mood(self.smooth),
            label: self.label,
        }
    }

    fn reset(&mut self, label: Label) -> Frame {
        self.smooth = SMOOTH_FLOOR_DB;
        self.label = label;
        Frame {
            fill: 0.0,
            mood: Mood::NoMic,
            label,
        }
    }
}

fn mood(db: f32) -> Mood {
    if db >= LOUD_DB {
        Mood::TooLoud
    } else if db >= CAREFUL_DB {
        Mood::Careful
    } else if db > SILENCE_LABEL_AT_DB {
        Mood::Good
    } else {
        Mood::Quiet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::MIN_DB;

    fn settle(ear: &mut Ear, db: f32) -> Frame {
        (0..120).map(|_| ear.step(Reading::Level(db))).last().unwrap()
    }

    #[test]
    fn moods_follow_hearing_thresholds() {
        assert_eq!(mood(MIN_DB), Mood::Quiet);
        assert_eq!(mood(CAREFUL_DB - 1.0), Mood::Good);
        assert_eq!(mood(CAREFUL_DB), Mood::Careful);
        assert_eq!(mood(LOUD_DB), Mood::TooLoud);
    }

    #[test]
    fn ear_smooths_and_labels_like_the_mic() {
        let mut ear = Ear::new();
        let frame = settle(&mut ear, -14.2);
        assert_eq!(frame.mood, Mood::TooLoud);
        assert_eq!(frame.label, Label::Db(-14));
        assert_eq!(frame.label.text(), "−14 dB");
        assert!((frame.fill - meter::db_frac(-14.2)).abs() < 1e-3);
        let frame = settle(&mut ear, -90.0);
        assert_eq!((frame.mood, frame.label, frame.fill), (Mood::Quiet, Label::Silent, 0.0));
    }

    #[test]
    fn muted_and_unavailable_reset_the_level() {
        let mut ear = Ear::new();
        settle(&mut ear, -10.0);
        let muted = ear.step(Reading::Muted);
        assert_eq!((muted.fill, muted.mood, muted.label), (0.0, Mood::NoMic, Label::Muted));
        assert_eq!(Label::Muted.text(), "MUTED");
        assert_eq!(ear.step(Reading::Unavailable).label.text(), "—");
        // The label refreshes immediately when sound becomes available again.
        assert_eq!(ear.step(Reading::Level(-40.0)).label, Label::Db(-51));
    }
}
