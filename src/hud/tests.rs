use super::Rendered;
use crate::meter::{ALARM_TICKS, Frame, Meter, Mood, N_SEG};

fn assert_full_repaint(cache: &Rendered, frame: &Frame) {
    assert!(cache.changed(&frame.rgb, |f| &f.rgb));
    assert!(cache.changed(&frame.label, |f| &f.label));
    assert!(cache.changed(&frame.db_text, |f| &f.db_text));
    assert!(cache.changed(&frame.orb_diameter, |f| &f.orb_diameter));
    assert!(cache.changed(&frame.orb_shadow_radius, |f| &f.orb_shadow_radius));
    assert!(cache.changed(&frame.ring_opacity, |f| &f.ring_opacity));
    assert!(cache.changed(&frame.alarm_opacity, |f| &f.alarm_opacity));
    assert!(cache.changed(&frame.border_width, |f| &f.border_width));
    assert!(cache.changed(&frame.border_rgba, |f| &f.border_rgba));
    for i in 0..N_SEG {
        assert!(cache.changed(&frame.segment_on[i], |f| &f.segment_on[i]));
    }
}

#[test]
fn showing_after_alarm_expired_hidden_repaints_without_model_change_flags() {
    let mut meter = Meter::new(-20.0);
    let mut frame = meter.step(Some(0.0));
    for _ in 0..10 {
        frame = meter.step(Some(0.0));
    }
    assert_eq!(frame.mood, Mood::TooLoud);
    let mut cache = Rendered::default();
    cache.remember(&frame);
    // Hidden ticks advance the model but do not touch the drawn snapshot.
    for _ in 0..ALARM_TICKS + 100 {
        frame = meter.step(Some(-90.0));
    }
    assert_eq!(frame.mood, Mood::Quiet);
    assert!(!frame.mood_changed && !frame.label_updated && !frame.db_text_updated);
    assert!(cache.changed(&frame.rgb, |f| &f.rgb));
    assert!(cache.changed(&frame.alarm_opacity, |f| &f.alarm_opacity));
    cache = Rendered::default(); // Same invalidation used by Hud::show.
    assert_full_repaint(&cache, &frame);
    cache.remember(&frame);
    assert!(!cache.changed(&frame.rgb, |f| &f.rgb));
    assert!(!cache.changed(&frame.label, |f| &f.label));
}

#[test]
fn showing_after_alarm_started_hidden_repaints_the_complete_snapshot() {
    let mut meter = Meter::new(-20.0);
    let mut frame = meter.step(Some(-90.0));
    let mut cache = Rendered::default();
    cache.remember(&frame);
    for _ in 0..15 {
        frame = meter.step(Some(0.0));
    }
    assert_eq!(frame.mood, Mood::TooLoud);
    assert!(!frame.mood_changed && !frame.label_updated);
    assert!(cache.changed(&frame.rgb, |f| &f.rgb));
    cache = Rendered::default();
    assert_full_repaint(&cache, &frame);
}

#[test]
fn threshold_invalidation_repaints_even_when_segment_on_bits_are_unchanged() {
    let mut meter = Meter::new(-20.0);
    let frame = meter.step(Some(-90.0));
    let mut cache = Rendered::default();
    cache.remember(&frame);
    assert!(!cache.changed(&frame.segment_on, |f| &f.segment_on));
    meter.set_threshold(-30.0);
    cache = Rendered::default(); // Same invalidation used by Hud::set_threshold.
    assert_full_repaint(&cache, &meter.step(Some(-90.0)));
}
