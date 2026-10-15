//! AppKit layer construction and snapshot-only rendering.

mod placement;
#[cfg(test)]
mod tests;

use crate::hearing::{self, Label};
use crate::meter::{self, Frame, N_SEG};
use crate::prefs::Prefs;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameVibrantDark, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSColor, NSFont, NSFontWeightHeavy, NSFontWeightSemibold, NSPanel, NSScreen, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSString;
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, kCAAlignmentCenter, kCAAlignmentRight};
pub use placement::Placement;
use placement::pill_mask;
use std::cell::RefCell;

const W: f64 = 344.0;
/// Main microphone row; the smaller 👂 hearing row sits below it.
const ROW_H: f64 = 76.0;
const EAR_H: f64 = 20.0;
const H: f64 = ROW_H + EAR_H;
const MID_Y: f64 = EAR_H + ROW_H / 2.0;
const EAR_Y: f64 = 15.0;
const EAR_TRACK_H: f64 = 3.0;
const CORNER: f64 = 20.0;
const METER_X: f64 = 78.0;
const METER_W: f64 = 174.0;
const ZOOM_SCALE: f64 = 2.2;

pub struct Hud {
    pub panel: Retained<NSPanel>,
    pub canvas: Retained<NSView>,
    glass: Retained<NSVisualEffectView>,
    stage: Retained<CALayer>,
    alarm_bg: Retained<CALayer>,
    border: Retained<CALayer>,
    orb: Retained<CALayer>,
    ring: Retained<CALayer>,
    segs: Vec<Retained<CALayer>>,
    marker: Retained<CALayer>,
    db_text: Retained<CATextLayer>,
    mood_text: Retained<CATextLayer>,
    ear_fill: Retained<CALayer>,
    ear_text: Retained<CATextLayer>,
    rendered: RefCell<Rendered>,
    rendered_hearing: RefCell<Option<HearingSnapshot>>,
    colors: RefCell<Vec<(CFRetained<CGColor>, CFRetained<CGColor>)>>,
}

#[derive(Default)]
struct Rendered {
    snapshot: Option<Snapshot>,
}

struct Snapshot {
    segment_on: [bool; N_SEG],
    orb_diameter: f64,
    orb_shadow_radius: f64,
    rgb: meter::Rgb,
    alarm_opacity: f32,
    ring_opacity: f32,
    border_width: f64,
    border_rgba: meter::Rgba,
    label: String,
    db_text: String,
}

impl Rendered {
    fn changed<T: PartialEq + ?Sized>(&self, value: &T, field: impl Fn(&Snapshot) -> &T) -> bool {
        self.snapshot.as_ref().is_none_or(|previous| field(previous) != value)
    }

    fn remember(&mut self, frame: &Frame) {
        let (mut label, mut db_text) = self
            .snapshot
            .take()
            .map(|old| (old.label, old.db_text))
            .unwrap_or_default();
        label.clone_from(&frame.label);
        db_text.clone_from(&frame.db_text);
        self.snapshot = Some(Snapshot {
            segment_on: frame.segment_on,
            orb_diameter: frame.orb_diameter,
            orb_shadow_radius: frame.orb_shadow_radius,
            rgb: frame.rgb,
            alarm_opacity: frame.alarm_opacity,
            ring_opacity: frame.ring_opacity,
            border_width: frame.border_width,
            border_rgba: frame.border_rgba,
            label,
            db_text,
        });
    }
}

impl Hud {
    pub fn build(mtm: MainThreadMarker) -> Self {
        // ---- Floating glass panel ----
        let frame = rect(0.0, 0.0, W, H);
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        panel.setLevel(25); // NSStatusWindowLevel: float above full-screen meeting apps
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setMovableByWindowBackground(true);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary,
        );
        if let Some(app) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameVibrantDark }) {
            panel.setAppearance(Some(&app));
        }

        let glass = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        glass.setMaterial(NSVisualEffectMaterial::HUDWindow);
        glass.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        glass.setState(NSVisualEffectState::Active);
        glass.setWantsLayer(true);
        // NSVisualEffectView ignores layer corner radius for the blur; a mask image is the proper way.
        glass.setMaskImage(Some(&pill_mask(CORNER)));
        panel.setContentView(Some(&glass));

        let canvas = NSView::initWithFrame(NSView::alloc(mtm), frame);
        canvas.setWantsLayer(true);
        if let Some(l) = canvas.layer() {
            l.setCornerRadius(CORNER);
            l.setMasksToBounds(true);
        }
        glass.addSubview(&canvas);
        canvas.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let root = canvas.layer().expect("layer");
        // All content lives in a fixed-size "stage" that gets scaled when zooming.
        let stage = CALayer::new();
        stage.setAnchorPoint(CGPoint::new(0.0, 0.0));
        stage.setBounds(frame);
        stage.setPosition(CGPoint::new(0.0, 0.0));
        root.addSublayer(&stage);
        // render text sharp enough for the zoomed-in size
        let scale = NSScreen::mainScreen(mtm).map(|s| s.backingScaleFactor()).unwrap_or(2.0) * ZOOM_SCALE;

        let alarm_bg = CALayer::new();
        alarm_bg.setFrame(frame);
        alarm_bg.setBackgroundColor(Some(&cg(1.0, 0.15, 0.3, 1.0)));
        alarm_bg.setOpacity(0.0);
        stage.addSublayer(&alarm_bg);

        let border = CALayer::new();
        border.setFrame(frame);
        border.setCornerRadius(CORNER);
        border.setBorderWidth(1.0);
        border.setBorderColor(Some(&cg(1.0, 1.0, 1.0, 0.10)));
        stage.addSublayer(&border);

        // ---- Orb ----
        let ring = CALayer::new();
        ring.setFrame(rect(40.0 - 22.0, MID_Y - 22.0, 44.0, 44.0));
        ring.setCornerRadius(22.0);
        ring.setBorderWidth(1.5);
        stage.addSublayer(&ring);

        let orb = CALayer::new();
        orb.setShadowOffset(CGSize::new(0.0, 0.0));
        orb.setShadowOpacity(0.95);
        stage.addSublayer(&orb);

        // ---- Segmented wedge meter ----
        let gap = 2.0;
        let seg_w = (METER_W - gap * (N_SEG as f64 - 1.0)) / N_SEG as f64;
        let base_y = EAR_H + 20.0;
        let mut segs = Vec::with_capacity(N_SEG);
        for i in 0..N_SEG {
            let h = 10.0 + 26.0 * (i as f64 / (N_SEG as f64 - 1.0));
            let s = CALayer::new();
            s.setFrame(rect(METER_X + i as f64 * (seg_w + gap), base_y, seg_w, h));
            s.setCornerRadius(seg_w / 2.0);
            stage.addSublayer(&s);
            segs.push(s);
        }

        let marker = CALayer::new();
        marker.setBackgroundColor(Some(&cg(1.0, 1.0, 1.0, 0.9)));
        marker.setCornerRadius(1.0);
        marker.setShadowColor(Some(&cg(1.0, 1.0, 1.0, 1.0)));
        marker.setShadowOpacity(0.6);
        marker.setShadowRadius(3.0);
        marker.setShadowOffset(CGSize::new(0.0, 0.0));
        stage.addSublayer(&marker);

        // ---- Text ----
        let db_text = CATextLayer::new();
        db_text.setFrame(rect(258.0, EAR_H + 34.0, 66.0, 26.0));
        db_text.setContentsScale(scale);
        db_text.setAlignmentMode(unsafe { kCAAlignmentRight });
        db_text.setForegroundColor(Some(&cg(1.0, 1.0, 1.0, 0.95)));
        let f = NSFont::monospacedDigitSystemFontOfSize_weight(20.0, unsafe { NSFontWeightSemibold });
        unsafe {
            let _: () = msg_send![&*db_text, setFont: &*f];
        }
        db_text.setFontSize(20.0);
        stage.addSublayer(&db_text);

        let mood_text = CATextLayer::new();
        mood_text.setFrame(rect(240.0, EAR_H + 18.0, 84.0, 14.0));
        mood_text.setContentsScale(scale);
        mood_text.setAlignmentMode(unsafe { kCAAlignmentRight });
        let f = NSFont::systemFontOfSize_weight(10.0, unsafe { NSFontWeightHeavy });
        unsafe {
            let _: () = msg_send![&*mood_text, setFont: &*f];
        }
        mood_text.setFontSize(10.0);
        stage.addSublayer(&mood_text);

        // ---- Hearing row: 👂, thin level bar with a too-loud tick, dB readout ----
        let ear = CATextLayer::new();
        ear.setFrame(rect(30.0, EAR_Y - 8.0, 20.0, 16.0));
        ear.setContentsScale(scale);
        ear.setAlignmentMode(unsafe { kCAAlignmentCenter });
        ear.setFontSize(11.0);
        unsafe {
            let _: () = msg_send![&*ear, setString: &*NSString::from_str("👂")];
        }
        stage.addSublayer(&ear);

        let track_y = EAR_Y - EAR_TRACK_H / 2.0;
        let track = CALayer::new();
        track.setFrame(rect(METER_X, track_y, METER_W, EAR_TRACK_H));
        track.setCornerRadius(EAR_TRACK_H / 2.0);
        track.setBackgroundColor(Some(&cg(1.0, 1.0, 1.0, 0.10)));
        stage.addSublayer(&track);

        let ear_fill = CALayer::new();
        ear_fill.setCornerRadius(EAR_TRACK_H / 2.0);
        ear_fill.setShadowOffset(CGSize::new(0.0, 0.0));
        ear_fill.setShadowOpacity(0.7);
        ear_fill.setShadowRadius(3.0);
        stage.addSublayer(&ear_fill);

        let loud_tick = CALayer::new();
        let loud_x = METER_X + METER_W * meter::db_frac(hearing::LOUD_DB);
        loud_tick.setFrame(rect(loud_x - 0.5, EAR_Y - 4.0, 1.0, 8.0));
        loud_tick.setBackgroundColor(Some(&cg(1.0, 1.0, 1.0, 0.45)));
        stage.addSublayer(&loud_tick);

        let ear_text = CATextLayer::new();
        ear_text.setFrame(rect(258.0, EAR_Y - 7.0, 66.0, 14.0));
        ear_text.setContentsScale(scale);
        ear_text.setAlignmentMode(unsafe { kCAAlignmentRight });
        let f = NSFont::monospacedDigitSystemFontOfSize_weight(10.0, unsafe { NSFontWeightSemibold });
        unsafe {
            let _: () = msg_send![&*ear_text, setFont: &*f];
        }
        ear_text.setFontSize(10.0);
        stage.addSublayer(&ear_text);
        Self {
            panel,
            canvas,
            glass,
            stage,
            alarm_bg,
            border,
            orb,
            ring,
            segs,
            marker,
            db_text,
            mood_text,
            ear_fill,
            ear_text,
            rendered: RefCell::new(Rendered::default()),
            rendered_hearing: RefCell::new(None),
            colors: RefCell::new(Vec::new()),
        }
    }

    pub fn invalidate(&self) {
        *self.rendered.borrow_mut() = Rendered::default();
        *self.rendered_hearing.borrow_mut() = None;
    }

    pub fn show(&self) {
        self.invalidate();
        self.panel.orderFrontRegardless();
    }

    pub fn set_threshold(&self, threshold: f32) {
        self.build_colors(threshold);
        self.marker.setFrame(rect(
            METER_X + meter::db_frac(threshold) * METER_W - 1.0,
            EAR_H + 14.0,
            2.0,
            48.0,
        ));
        self.invalidate();
    }

    fn build_colors(&self, threshold: f32) {
        *self.colors.borrow_mut() = (0..N_SEG)
            .map(|i| {
                let (r, g, b) = meter::segment_rgb(i, threshold);
                (cg(r, g, b, 1.0), cg(r, g, b, meter::SEGMENT_OFF_ALPHA))
            })
            .collect();
    }

    pub fn render(&self, frame: &Frame) {
        if !self.panel.isVisible() {
            return;
        }
        let mut rendered = self.rendered.borrow_mut();
        let old = rendered.snapshot.as_ref();
        macro_rules! changed {
            ($field:ident) => {
                rendered.changed(&frame.$field, |f| &f.$field)
            };
        }
        unsafe {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            let colors = self.colors.borrow();
            for (i, segment) in self.segs.iter().enumerate() {
                if old.is_none_or(|previous| previous.segment_on[i] != frame.segment_on[i]) {
                    let (on, off) = &colors[i];
                    segment.setBackgroundColor(Some(if frame.segment_on[i] { on } else { off }));
                }
            }
            if changed!(orb_diameter) {
                let d = frame.orb_diameter;
                self.orb.setFrame(rect(40.0 - d / 2.0, MID_Y - d / 2.0, d, d));
                self.orb.setCornerRadius(d / 2.0);
            }
            if changed!(orb_shadow_radius) {
                self.orb.setShadowRadius(frame.orb_shadow_radius);
            }
            if changed!(rgb) {
                let (r, g, b) = frame.rgb;
                let color = cg(r, g, b, 1.0);
                self.orb.setBackgroundColor(Some(&color));
                self.orb.setShadowColor(Some(&color));
                self.ring.setBorderColor(Some(&color));
                self.mood_text.setForegroundColor(Some(&color));
            }
            if changed!(alarm_opacity) {
                self.alarm_bg.setOpacity(frame.alarm_opacity);
            }
            if changed!(ring_opacity) {
                self.ring.setOpacity(frame.ring_opacity);
            }
            if changed!(border_width) {
                self.border.setBorderWidth(frame.border_width);
            }
            if changed!(border_rgba) {
                let (r, g, b, a) = frame.border_rgba;
                self.border.setBorderColor(Some(&cg(r, g, b, a)));
            }
            if changed!(label) {
                let _: () = msg_send![&*self.mood_text, setString: &*NSString::from_str(&frame.label)];
            }
            if changed!(db_text) {
                let _: () = msg_send![&*self.db_text, setString: &*NSString::from_str(&frame.db_text)];
            }
            CATransaction::commit();
        }
        rendered.remember(frame);
    }

    /// Repaints only what changed in the hearing row; the label text changes at most ~7 Hz.
    pub fn render_hearing(&self, frame: &hearing::Frame) {
        if !self.panel.isVisible() {
            return;
        }
        let next = HearingSnapshot {
            width: (METER_W * frame.fill).round(),
            mood: frame.mood,
            label: frame.label,
        };
        let previous = self.rendered_hearing.borrow_mut().replace(next);
        if previous == Some(next) {
            return;
        }
        unsafe {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            if previous.is_none_or(|p| p.width != next.width) {
                self.ear_fill
                    .setFrame(rect(METER_X, EAR_Y - EAR_TRACK_H / 2.0, next.width, EAR_TRACK_H));
            }
            if previous.is_none_or(|p| p.mood != next.mood) {
                let (r, g, b) = next.mood.rgb();
                let color = cg(r, g, b, 1.0);
                self.ear_fill.setBackgroundColor(Some(&color));
                self.ear_fill.setShadowColor(Some(&color));
                self.ear_text.setForegroundColor(Some(&color));
            }
            if previous.is_none_or(|p| p.label != next.label) {
                let _: () = msg_send![&*self.ear_text, setString: &*NSString::from_str(&next.label.text())];
            }
            CATransaction::commit();
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct HearingSnapshot {
    width: f64,
    mood: meter::Mood,
    label: Label,
}

fn cg(r: f64, g: f64, b: f64, a: f64) -> CFRetained<CGColor> {
    CGColor::new_srgb(r, g, b, a)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}
