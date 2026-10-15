use super::*;

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

pub(super) fn steady(db: f32, threshold: f32) -> Meter {
    let mut meter = Meter::new(threshold, true);
    meter.smooth = db;
    meter.peak = db;
    meter
}

#[test]
fn smoothing_attack_release_and_floor() {
    let mut meter = Meter::new(DEFAULT_THRESHOLD, true);
    close(meter.step(-20.0).smooth, -38.0);
    close(meter.step(-50.0).smooth, -39.2);
    close(meter.step(-90.0).smooth, -41.78);
    for _ in 0..200 {
        meter.step(-90.0);
    }
    assert!((meter.smooth - SMOOTH_FLOOR_DB).abs() < 1e-4);
}

#[test]
fn peak_holds_for_27_ticks_then_decays_and_resets_on_attack() {
    let mut meter = Meter::new(DEFAULT_THRESHOLD, true);
    let peak = meter.step(-20.0).peak;
    assert_eq!(PEAK_HOLD_TICKS, 27);
    for _ in 0..PEAK_HOLD_TICKS {
        close(meter.step(-90.0).peak, peak);
    }
    close(meter.step(-90.0).peak, peak - PEAK_DECAY_DB);
    close(meter.step(-90.0).peak, peak - 2.0 * PEAK_DECAY_DB);
    let frame = meter.step(0.0);
    close(frame.peak, frame.smooth);
    assert_eq!(meter.peak_hold, PEAK_HOLD_TICKS);
}

#[test]
fn alarm_requires_five_consecutive_ticks_and_expires_after_48_quiet_ticks() {
    let mut meter = steady(-20.0, -20.0);
    for _ in 1..LOUD_DEBOUNCE_TICKS {
        let frame = meter.step(-20.0);
        assert!(!frame.alarm_active);
        assert_eq!(frame.mood, Mood::Careful);
    }
    let frame = meter.step(-20.0);
    assert!(frame.too_loud && frame.alarm_active);
    assert_eq!(frame.mood, Mood::TooLoud);
    assert_eq!(ALARM_TICKS, 48);
    for _ in 0..10 {
        meter.step(-20.0);
        assert_eq!(meter.alarm_ticks, ALARM_TICKS);
    }
    for _ in 1..ALARM_TICKS {
        assert!(meter.step(-90.0).too_loud);
    }
    let frame = meter.step(-90.0);
    assert!(!frame.alarm_active && !frame.too_loud);
    assert_eq!(frame.alarm_opacity, 0.0);
    assert_eq!(frame.border_width, 1.0);
}

#[test]
fn one_quiet_tick_resets_debounce() {
    let mut meter = steady(-20.0, -20.0);
    for _ in 1..LOUD_DEBOUNCE_TICKS {
        meter.step(-20.0);
    }
    meter.step(-90.0);
    assert_eq!(meter.loud_ticks, 0);
    meter.smooth = -20.0;
    assert!(!meter.step(-20.0).alarm_active);
    assert_eq!(meter.loud_ticks, 1);
}

#[test]
fn calibration_runs_150_ticks_and_applies_result_after_the_mood_decision() {
    let mut meter = steady(-30.0, -12.0);
    meter.start_calibration();
    let first = meter.step(-30.0);
    assert_eq!(first.label, "SPEAK… 5s");
    assert_eq!(first.mood, Mood::Calibrating);
    for tick in 2..CALIBRATION_TICKS {
        let frame = meter.step(-30.0);
        assert_eq!(frame.calibration_result, None);
        assert_eq!(frame.threshold, -12.0);
        if tick == 30 {
            assert_eq!(frame.label, "SPEAK… 4s");
        }
        if tick == 120 {
            assert_eq!(frame.label, "SPEAK… 1s");
        }
    }
    let frame = meter.step(-30.0);
    assert_eq!(meter.mode(), &Mode::Live);
    assert_eq!(frame.calibration_result, Some(-24.0));
    assert_eq!(frame.threshold, -24.0);
    assert_eq!(frame.mood, Mood::Good);
    assert_eq!(frame.label, "GOOD");
    assert_eq!(threshold_label(frame.threshold), "Alert above -24 dB");
    let next = meter.step(-30.0);
    assert_eq!(next.mood, Mood::Careful);
    assert_eq!(next.calibration_result, None);
}

#[test]
fn calibration_uses_filtered_smoothed_samples_and_can_restart() {
    let mut meter = steady(-55.0, -20.0);
    meter.start_calibration();
    meter.step(-55.0);
    assert!(matches!(meter.mode(), Mode::Calibrating { samples, .. } if samples.is_empty()));
    meter.smooth = -40.0;
    meter.step(-20.0);
    assert!(matches!(meter.mode(), Mode::Calibrating { samples, .. } if samples == &vec![-29.0]));
    meter.start_calibration();
    assert!(matches!(meter.mode(), Mode::Calibrating { ticks_left: CALIBRATION_TICKS, samples } if samples.is_empty()));
}

#[test]
fn calibration_needs_more_than_twenty_samples() {
    for (count, expected) in [(0, None), (20, None), (21, Some(-24.0))] {
        let mut meter = steady(-60.0, -20.0);
        meter.mode = Mode::Calibrating {
            ticks_left: 1,
            samples: vec![-30.0; count],
        };
        let frame = meter.step(-60.0);
        assert_eq!(frame.calibration_result, expected);
        assert_eq!(meter.mode(), &Mode::Live);
        assert_eq!(meter.threshold(), expected.unwrap_or(-20.0));
    }
}

#[test]
fn calibration_includes_the_final_tick_sample() {
    let mut meter = steady(-30.0, -20.0);
    meter.mode = Mode::Calibrating {
        ticks_left: 1,
        samples: vec![-30.0; CALIBRATION_MIN_SAMPLES - 1],
    };
    assert_eq!(meter.step(-30.0).calibration_result, Some(-24.0));
}

#[test]
fn setting_threshold_preserves_running_state_and_updates_menu_data() {
    let mut meter = steady(-30.0, -20.0);
    meter.step(-30.0);
    meter.start_calibration();
    meter.set_threshold(-18.4);
    let frame = meter.step(-30.0);
    assert_eq!(frame.tick, 2);
    assert_eq!(frame.smooth, -30.0);
    assert_eq!(frame.threshold, -18.4);
    assert_eq!(threshold_label(frame.threshold), "Alert above -18 dB");
    assert_eq!(frame.selected_preset_db, -18);
    assert!(matches!(meter.mode(), Mode::Calibrating { ticks_left: 149, .. }));
}

#[test]
fn rms_conversion_matches_audio_level_units_and_silence_floor() {
    assert_eq!(rms_db(0.0), -90.0);
    assert_eq!(rms_db(1e-7), -90.0);
    assert_eq!(rms_db(1.0), 0.0);
    close(rms_db(0.1), -20.0);
}
