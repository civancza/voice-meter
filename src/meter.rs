//! Pure 30 Hz meter state and presentation data, independent of audio capture and AppKit.
//!
//! Feed already-converted dB readings to [`Meter::step`]. The model owns smoothing,
//! peak hold, alarm debounce, calibration, and text refresh cadence. Render the
//! returned snapshot; persist `calibration_result` when present. Calibration is
//! applied after the finishing tick's mood decision, just as in the controller.
//!
//! Deliberate differences from the original controller: all threshold sources
//! clamp to -45..=-3 dB; segment amber starts 6 dB below the threshold, matching
//! the careful mood; alarm borders use [`Mood::TooLoud`]'s RGB. Presentation data
//! advances even while the HUD is hidden. On revealing the HUD, render the whole
//! snapshot rather than relying on its incremental `*_updated` flags.

pub const N_SEG: usize = 28;
pub const MIN_DB: f32 = -60.0;
pub const MAX_DB: f32 = 0.0;
pub const FPS: f64 = 30.0;
pub const DEFAULT_THRESHOLD: f32 = -20.0;
pub const MIN_THRESHOLD: f32 = -45.0;
pub const MAX_THRESHOLD: f32 = -3.0;
pub const CAREFUL_BELOW_DB: f32 = 6.0;
pub const GOOD_BELOW_DB: f32 = 26.0;
pub const ATTACK: f32 = 0.55;
pub const RELEASE: f32 = 0.10;
pub const SMOOTH_FLOOR_DB: f32 = MIN_DB - 5.0;
pub const PEAK_HOLD_TICKS: u32 = (FPS * 0.9) as u32;
pub const PEAK_DECAY_DB: f32 = 0.7;
pub const PEAK_VISIBLE_ABOVE_DB: f32 = MIN_DB + 2.0;
pub const LOUD_DEBOUNCE_TICKS: u32 = 5;
pub const ALARM_TICKS: u32 = (FPS * 1.6) as u32;
pub const CALIBRATION_TICKS: u32 = (FPS * 5.0) as u32;
pub const CALIBRATION_SAMPLE_ABOVE_DB: f32 = -55.0;
pub const CALIBRATION_MIN_SAMPLES: usize = 21;
pub const CALIBRATION_HEADROOM_DB: f32 = 6.0;
pub const CALIBRATION_PERCENTILE_NUMERATOR: usize = 9;
pub const CALIBRATION_PERCENTILE_DENOMINATOR: usize = 10;
pub const DB_TEXT_EVERY_TICKS: u64 = 4;
pub const CALIBRATION_LABEL_EVERY_TICKS: u64 = 6;
pub const SILENCE_LABEL_AT_DB: f32 = MIN_DB + 1.0;
pub const SEGMENT_OFF_ALPHA: f64 = 0.14;

pub type Rgb = (f64, f64, f64);
pub type Rgba = (f64, f64, f64, f64);

const SAFE_START_RGB: Rgb = (0.20, 0.75, 1.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Quiet,
    Good,
    Careful,
    TooLoud,
    Calibrating,
    NoMic,
}

impl Mood {
    pub fn rgb(self) -> Rgb {
        match self {
            Self::Quiet => (0.55, 0.62, 0.74),
            Self::Good => (0.25, 0.95, 0.62),
            Self::Careful => (1.0, 0.74, 0.22),
            Self::TooLoud => (1.0, 0.27, 0.40),
            Self::Calibrating => (0.62, 0.52, 1.0),
            Self::NoMic => (0.45, 0.45, 0.5),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Quiet => "QUIET",
            Self::Good => "GOOD",
            Self::Careful => "EASY…",
            Self::TooLoud => "TOO LOUD",
            Self::Calibrating => "SPEAK…",
            Self::NoMic => "NO MIC",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Live,
    Calibrating { ticks_left: u32, samples: Vec<f32> },
}

/// A complete snapshot. Text retains the original 4/6-tick refresh cadence.
/// Geometry is in HUD points; the renderer only needs to position the orb.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub tick: u64,
    pub smooth: f32,
    pub peak: f32,
    pub frac: f64,
    pub mood: Mood,
    pub mood_changed: bool,
    pub rgb: Rgb,
    pub status_tint: Option<Rgb>,
    pub label: String,
    pub label_updated: bool,
    pub db_value: i32,
    pub db_text: String,
    pub db_text_updated: bool,
    pub lit_n: usize,
    pub peak_i: usize,
    pub segment_on: [bool; N_SEG],
    /// The displayed alarm (also the input to jump-to-centre), after mood priority.
    pub too_loud: bool,
    /// The alarm latch, even when calibration or a missing mic overrides its mood.
    pub alarm_active: bool,
    pub pulse: f64,
    pub orb_diameter: f64,
    pub orb_shadow_radius: f64,
    pub alarm_opacity: f32,
    pub ring_opacity: f32,
    pub border_width: f64,
    pub border_rgba: Rgba,
    /// Present for exactly the tick that successfully completes calibration.
    pub calibration_result: Option<f32>,
    /// Includes a newly calibrated threshold, applied after this tick's mood decision.
    pub threshold: f32,
    pub threshold_frac: f64,
    pub selected_preset_db: i32,
}

#[derive(Debug)]
pub struct Meter {
    threshold: f32,
    has_mic: bool,
    smooth: f32,
    peak: f32,
    peak_hold: u32,
    loud_ticks: u32,
    alarm_ticks: u32,
    tick: u64,
    mood: Mood,
    mode: Mode,
    label: String,
    db_value: Option<i32>,
    db_text: String,
}

impl Meter {
    /// Starts without a microphone; the first `Some` reading connects it.
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold: clamp_threshold(threshold),
            has_mic: false,
            smooth: MIN_DB,
            peak: MIN_DB,
            peak_hold: 0,
            loud_ticks: 0,
            alarm_ticks: 0,
            tick: 0,
            mood: Mood::Quiet,
            mode: Mode::Live,
            label: String::new(),
            db_value: None,
            db_text: String::new(),
        }
    }

    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    pub fn set_threshold(&mut self, db: f32) {
        self.threshold = clamp_threshold(db);
    }

    /// Gaining or losing the microphone drops stale levels and alarms; losing it cancels calibration.
    fn set_mic(&mut self, has_mic: bool) {
        if self.has_mic == has_mic {
            return;
        }
        self.has_mic = has_mic;
        self.smooth = MIN_DB;
        self.peak = MIN_DB;
        self.peak_hold = 0;
        self.loud_ticks = 0;
        self.alarm_ticks = 0;
        self.db_value = None;
        if !has_mic {
            self.mode = Mode::Live;
        }
    }

    #[cfg(test)]
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Restarts the five-second sample window without resetting smoothing or alarms.
    pub fn start_calibration(&mut self) {
        self.mode = Mode::Calibrating {
            ticks_left: CALIBRATION_TICKS,
            samples: Vec::new(),
        };
    }

    /// Consume one dB reading at 30 Hz, including when the HUD is hidden; `None` means no microphone.
    pub fn step(&mut self, db: Option<f32>) -> Frame {
        self.set_mic(db.is_some());
        let db = db.unwrap_or(f32::NEG_INFINITY);
        self.tick += 1;
        self.smooth = ease(self.smooth, db);
        let smooth = self.smooth;

        if smooth > self.peak {
            self.peak = smooth;
            self.peak_hold = PEAK_HOLD_TICKS;
        } else if self.peak_hold > 0 {
            self.peak_hold -= 1;
        } else {
            self.peak -= PEAK_DECAY_DB;
        }

        if smooth >= self.threshold {
            self.loud_ticks += 1;
        } else {
            self.loud_ticks = 0;
        }
        if self.loud_ticks >= LOUD_DEBOUNCE_TICKS {
            self.alarm_ticks = ALARM_TICKS;
        } else if self.alarm_ticks > 0 {
            self.alarm_ticks -= 1;
        }

        let mut calibration_result = None;
        let mut calibration_secs = 0;
        if let Mode::Calibrating { ticks_left, samples } = &mut self.mode {
            if smooth > CALIBRATION_SAMPLE_ABOVE_DB {
                samples.push(smooth);
            }
            *ticks_left -= 1;
            calibration_secs = (*ticks_left as f64 / FPS).ceil() as u32;
            if *ticks_left == 0 {
                if samples.len() >= CALIBRATION_MIN_SAMPLES {
                    samples.sort_by(f32::total_cmp);
                    let p90 =
                        samples[samples.len() * CALIBRATION_PERCENTILE_NUMERATOR / CALIBRATION_PERCENTILE_DENOMINATOR];
                    calibration_result = Some(clamp_threshold((p90 + CALIBRATION_HEADROOM_DB).round()));
                }
                self.mode = Mode::Live;
            }
        }

        let alarm_active = self.alarm_ticks > 0;
        let mood = if !self.has_mic {
            Mood::NoMic
        } else if matches!(self.mode, Mode::Calibrating { .. }) {
            Mood::Calibrating
        } else if alarm_active {
            Mood::TooLoud
        } else {
            level_mood(smooth, self.threshold)
        };
        let mood_changed = self.tick == 1 || self.mood != mood;
        self.mood = mood;
        let rgb = mood.rgb();
        let status_tint = match mood {
            Mood::Quiet | Mood::NoMic => None,
            _ => Some(rgb),
        };
        let label_updated =
            mood_changed || (mood == Mood::Calibrating && self.tick.is_multiple_of(CALIBRATION_LABEL_EVERY_TICKS));
        if label_updated {
            self.label = if mood == Mood::Calibrating {
                format!("SPEAK… {calibration_secs}s")
            } else {
                mood.label().to_string()
            };
        }

        let mut db_text_updated = false;
        if self.tick.is_multiple_of(DB_TEXT_EVERY_TICKS) || self.db_value.is_none() {
            let value = smooth.round() as i32;
            if self.db_value != Some(value) {
                self.db_value = Some(value);
                self.db_text = db_label(smooth);
                db_text_updated = true;
            }
        }

        let frac = db_frac(smooth);
        let lit_n = (frac * N_SEG as f64).round() as usize;
        let peak_i = ((db_frac(self.peak) * N_SEG as f64).round() as usize).saturating_sub(1);
        let segment_on = std::array::from_fn(|i| i < lit_n || (i == peak_i && self.peak > PEAK_VISIBLE_ABOVE_DB));
        let pulse = 0.5 + 0.5 * (self.tick as f64 * 0.45).sin();
        let too_loud = mood == Mood::TooLoud;
        let (r, g, b) = Mood::TooLoud.rgb();
        let border_rgba = if too_loud {
            (r, g, b, 0.5 + 0.5 * pulse)
        } else {
            (1.0, 1.0, 1.0, 0.10)
        };

        // on_tick applied calibration only after rendering and choosing the jump target.
        if let Some(threshold) = calibration_result {
            self.set_threshold(threshold);
        }

        Frame {
            tick: self.tick,
            smooth,
            peak: self.peak,
            frac,
            mood,
            mood_changed,
            rgb,
            status_tint,
            label: self.label.clone(),
            label_updated,
            db_value: self.db_value.expect("first step initializes the dB text"),
            db_text: self.db_text.clone(),
            db_text_updated,
            lit_n,
            peak_i,
            segment_on,
            too_loud,
            alarm_active,
            pulse,
            orb_diameter: 14.0 + 22.0 * frac,
            orb_shadow_radius: 4.0 + 14.0 * frac,
            alarm_opacity: if too_loud { (0.12 + 0.22 * pulse) as f32 } else { 0.0 },
            ring_opacity: if too_loud {
                (0.35 + 0.65 * pulse) as f32
            } else if mood == Mood::Calibrating {
                0.8
            } else {
                0.18
            },
            border_width: if too_loud { 2.0 } else { 1.0 },
            border_rgba,
            calibration_result,
            threshold: self.threshold,
            threshold_frac: db_frac(self.threshold),
            selected_preset_db: self.threshold.round() as i32,
        }
    }
}

/// Shared saved-value, preset and calibration range: inclusive -45..=-3 dB.
/// NaN falls back to the default; infinities clamp to the corresponding endpoint.
/// Callers must distinguish a missing saved preference from a stored zero first.
pub fn clamp_threshold(db: f32) -> f32 {
    if db.is_nan() {
        DEFAULT_THRESHOLD
    } else {
        db.clamp(MIN_THRESHOLD, MAX_THRESHOLD)
    }
}

pub fn db_frac(db: f32) -> f64 {
    ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0) as f64
}

/// One attack/release smoothing step from `smooth` toward `db`, floored at `SMOOTH_FLOOR_DB`.
pub fn ease(smooth: f32, db: f32) -> f32 {
    let k = if db > smooth { ATTACK } else { RELEASE };
    smooth + (db.max(SMOOTH_FLOOR_DB) - smooth) * k
}

pub fn db_label(db: f32) -> String {
    let number = if db <= SILENCE_LABEL_AT_DB {
        "—".to_string()
    } else {
        format!("{}", db.round() as i32).replace('-', "−")
    };
    format!("{number} dB")
}

pub fn threshold_label(threshold: f32) -> String {
    format!("Alert above {threshold:.0} dB")
}

pub fn careful_from(threshold: f32) -> f32 {
    threshold - CAREFUL_BELOW_DB
}

/// Instantaneous non-alarm mood: crossing the threshold still needs debounce.
pub fn level_mood(db: f32, threshold: f32) -> Mood {
    if db >= careful_from(threshold) {
        Mood::Careful
    } else if db >= threshold - GOOD_BELOW_DB {
        Mood::Good
    } else {
        Mood::Quiet
    }
}

/// Segment midpoints use the same careful boundary as the live mood.
/// The safe-zone cyan-to-mint gradient is preserved.
pub fn segment_rgb(i: usize, threshold: f32) -> Rgb {
    let seg_db = MIN_DB + (i as f32 + 0.5) / N_SEG as f32 * (MAX_DB - MIN_DB);
    if seg_db >= threshold {
        Mood::TooLoud.rgb()
    } else if seg_db >= careful_from(threshold) {
        Mood::Careful.rgb()
    } else {
        let amber_idx = (db_frac(careful_from(threshold)) * N_SEG as f64).max(1.0);
        let t = (i as f64 / amber_idx).clamp(0.0, 1.0);
        let (r, g, b) = SAFE_START_RGB;
        let (end_r, end_g, end_b) = Mood::Good.rgb();
        (r + (end_r - r) * t, g + (end_g - g) * t, b + (end_b - b) * t)
    }
}

/// Convert the peak RMS supplied by audio capture to the model's dB input.
pub fn rms_db(rms: f32) -> f32 {
    if rms > 1e-7 { 20.0 * rms.log10() } else { -90.0 }
}

#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod tests;
