use super::tests::steady;
use super::*;

#[test]
fn silent_calibration_completes_without_changing_threshold() {
    let mut meter = Meter::new(-18.0, true);
    meter.start_calibration();
    for _ in 0..CALIBRATION_TICKS {
        assert_eq!(meter.step(-90.0).calibration_result, None);
    }
    assert_eq!(meter.mode(), &Mode::Live);
    assert_eq!(meter.threshold(), -18.0);
}

#[test]
fn calibration_p90_sorting_rounding_and_clamping() {
    let cases = [
        ((0..100).rev().map(|i| -50.0 + i as f32 * 0.1).collect(), -35.0),
        (vec![-30.4; 21], -24.0),
        (vec![-54.0; 21], MIN_THRESHOLD),
        (vec![-2.0; 21], MAX_THRESHOLD),
    ];
    for (samples, expected) in cases {
        let mut meter = steady(-60.0, -20.0);
        meter.mode = Mode::Calibrating { ticks_left: 1, samples };
        assert_eq!(meter.step(-60.0).calibration_result, Some(expected));
    }
}

#[test]
fn missing_mic_and_calibration_override_alarm_without_resetting_it() {
    let mut meter = steady(-10.0, -20.0);
    for _ in 0..LOUD_DEBOUNCE_TICKS {
        meter.step(-10.0);
    }
    meter.start_calibration();
    let frame = meter.step(-10.0);
    assert!(frame.alarm_active && !frame.too_loud);
    assert_eq!(frame.mood, Mood::Calibrating);
    assert_eq!(frame.ring_opacity, 0.8);
    meter.has_mic = false;
    let frame = meter.step(-10.0);
    assert_eq!(frame.mood, Mood::NoMic);
    assert_eq!(frame.label, "NO MIC");
    assert_eq!(frame.status_tint, None);
    assert!(frame.alarm_active && !frame.too_loud);
}

#[test]
fn mood_boundaries_and_segment_colours_share_the_careful_boundary() {
    let threshold = -20.0;
    assert_eq!(careful_from(threshold), -26.0);
    assert_eq!(level_mood(-46.001, threshold), Mood::Quiet);
    assert_eq!(level_mood(-46.0, threshold), Mood::Good);
    assert_eq!(level_mood(-26.001, threshold), Mood::Good);
    assert_eq!(level_mood(-26.0, threshold), Mood::Careful);
    assert_eq!(level_mood(threshold, threshold), Mood::Careful);
    for threshold in [MIN_THRESHOLD, -30.0, DEFAULT_THRESHOLD, MAX_THRESHOLD] {
        for i in 0..N_SEG {
            let db = MIN_DB + (i as f32 + 0.5) / N_SEG as f32 * (MAX_DB - MIN_DB);
            let rgb = segment_rgb(i, threshold);
            if db >= threshold {
                assert_eq!(rgb, Mood::TooLoud.rgb());
            } else if level_mood(db, threshold) == Mood::Careful {
                assert_eq!(rgb, Mood::Careful.rgb());
            } else {
                assert_ne!(rgb, Mood::Careful.rgb());
                assert_ne!(rgb, Mood::TooLoud.rgb());
            }
        }
    }
    assert_eq!(segment_rgb(0, threshold), SAFE_START_RGB);
}

#[test]
fn segments_round_saturate_and_show_a_peak_only_above_minus_58() {
    let mut meter = steady(-30.0, -20.0);
    let frame = meter.step(-30.0);
    assert_eq!(frame.lit_n, 14);
    assert_eq!(frame.peak_i, 13);
    assert_eq!(frame.segment_on.iter().filter(|&&on| on).count(), 14);
    meter.smooth = -60.0;
    meter.peak = -20.0;
    meter.peak_hold = 1;
    let frame = meter.step(-60.0);
    assert_eq!(frame.lit_n, 0);
    assert_eq!(frame.segment_on.iter().filter(|&&on| on).count(), 1);
    assert!(frame.segment_on[frame.peak_i]);
    for (peak, visible) in [(-58.0, false), (-57.99, true)] {
        meter.peak = peak;
        meter.peak_hold = 1;
        assert_eq!(meter.step(-60.0).segment_on.iter().any(|&on| on), visible);
    }
    let mut low = steady(-65.0, -20.0);
    let frame = low.step(-65.0);
    assert_eq!(frame.peak_i, 0);
    assert_eq!(frame.lit_n, 0);
    let mut high = steady(10.0, -20.0);
    let frame = high.step(10.0);
    assert_eq!(frame.peak_i, N_SEG - 1);
    assert_eq!(frame.lit_n, N_SEG);
    assert!(frame.segment_on.iter().all(|&on| on));
}

#[test]
fn clamp_threshold_is_shared_by_construction_and_setter() {
    for (input, expected) in [
        (-90.0, -45.0),
        (-45.0, -45.0),
        (-20.5, -20.5),
        (-3.0, -3.0),
        (0.0, -3.0),
        (f32::NEG_INFINITY, -45.0),
        (f32::INFINITY, -3.0),
        (f32::NAN, DEFAULT_THRESHOLD),
    ] {
        assert_eq!(clamp_threshold(input), expected);
        let mut meter = Meter::new(input, true);
        assert_eq!(meter.threshold(), expected);
        meter.set_threshold(input);
        assert_eq!(meter.threshold(), expected);
    }
}

#[test]
fn db_frac_maps_range_and_clamps() {
    assert_eq!(db_frac(MIN_DB), 0.0);
    assert_eq!(db_frac(MAX_DB), 1.0);
    assert!((db_frac(-30.0) - 0.5).abs() < 1e-9);
    assert_eq!(db_frac(-100.0), 0.0);
    assert_eq!(db_frac(10.0), 1.0);
}

#[test]
fn mood_labels_are_distinct() {
    let all = [
        Mood::Quiet,
        Mood::Good,
        Mood::Careful,
        Mood::TooLoud,
        Mood::Calibrating,
        Mood::NoMic,
    ];
    let mut labels: Vec<_> = all.iter().map(|m| m.label()).collect();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), all.len());
    assert_eq!(
        all.map(Mood::label),
        ["QUIET", "GOOD", "EASY…", "TOO LOUD", "SPEAK…", "NO MIC"]
    );
}

#[test]
fn labels_use_an_em_dash_for_silence_and_unicode_minus_for_levels() {
    assert_eq!(db_label(-90.0), "— dB");
    assert_eq!(db_label(-59.0), "— dB");
    assert_eq!(db_label(-58.9), "−59 dB");
    assert_eq!(db_label(-20.5), "−21 dB");
    assert_eq!(db_label(0.0), "0 dB");
    assert_eq!(threshold_label(-20.0), "Alert above -20 dB");
}

#[test]
fn text_refresh_cadence_and_unchanged_integer_match_the_original() {
    let mut meter = Meter::new(-20.0, true);
    let first = meter.step(-20.0);
    assert_eq!(first.db_value, -38);
    assert_eq!(first.db_text, "−38 dB");
    assert!(first.db_text_updated && first.label_updated && first.mood_changed);
    for _ in 0..2 {
        let frame = meter.step(-20.0);
        assert_eq!(frame.db_text, first.db_text);
        assert!(!frame.db_text_updated);
    }
    assert!(meter.step(-20.0).db_text_updated);

    let mut meter = steady(-59.1, -20.0);
    assert_eq!(meter.step(-59.1).db_text, "— dB");
    meter.smooth = -58.9;
    for _ in 0..3 {
        assert_eq!(meter.step(-58.9).db_text, "— dB");
    }
    // The old code only changed the text if the rounded integer changed.
    assert_eq!(meter.db_value, Some(-59));

    meter.start_calibration();
    let frame = meter.step(-58.9);
    assert!(frame.label_updated);
    let frame = meter.step(-58.9);
    assert!(frame.label_updated && !frame.mood_changed);
    assert!(!meter.step(-58.9).label_updated);
}

#[test]
fn alarm_presentation_reuses_mood_colour_and_original_pulse() {
    let mut meter = steady(-10.0, -20.0);
    for _ in 1..LOUD_DEBOUNCE_TICKS {
        meter.step(-10.0);
    }
    let frame = meter.step(-10.0);
    assert_eq!(frame.rgb, Mood::TooLoud.rgb());
    assert_eq!(frame.status_tint, Some(frame.rgb));
    assert_eq!(
        (frame.border_rgba.0, frame.border_rgba.1, frame.border_rgba.2),
        frame.rgb
    );
    assert_eq!(frame.pulse, 0.5 + 0.5 * (5.0_f64 * 0.45).sin());
    assert_eq!(frame.border_width, 2.0);
    assert_eq!(frame.alarm_opacity, (0.12 + 0.22 * frame.pulse) as f32);
    assert_eq!(frame.ring_opacity, (0.35 + 0.65 * frame.pulse) as f32);
    assert_eq!(frame.orb_diameter, 14.0 + 22.0 * frame.frac);
    assert_eq!(frame.orb_shadow_radius, 4.0 + 14.0 * frame.frac);
    assert_eq!(frame.threshold_frac, db_frac(-20.0));
    assert_eq!(frame.selected_preset_db, -20);
}
