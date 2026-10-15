//! Voice Meter — a tiny floating HUD that shows how loudly you speak.
#![allow(unused_unsafe)]

use std::cell::{OnceCell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameVibrantDark, NSApplication, NSApplicationActivationPolicy,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath, NSColor, NSFont, NSFontWeightHeavy,
    NSFontWeightSemibold, NSImage, NSImageResizingMode, NSMenu, NSMenuItem, NSPanel, NSScreen, NSStatusBar,
    NSStatusItem, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::{
    NSArray, NSEdgeInsets, NSObject, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, NSUserDefaults,
};
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, CATransform3D, kCAAlignmentRight};

const N_SEG: usize = 28;
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 0.0;
const W: f64 = 344.0;
const H: f64 = 76.0;
const FPS: f64 = 30.0;
const CORNER: f64 = 20.0;
const METER_X: f64 = 78.0;
const METER_W: f64 = 174.0;
const DEFAULT_THR: f32 = -20.0;
const PREF_KEY: &str = "thresholdDb";
const JUMP_KEY: &str = "jumpWhenLoud";
const HOME_X_KEY: &str = "hudX";
const HOME_Y_KEY: &str = "hudY";
/// How big the HUD gets when it jumps to the screen centre.
const ZOOM_SCALE: f64 = 2.2;
const PRESETS: [(i32, &str); 5] = [
    (-30, "Very strict"),
    (-24, "Strict"),
    (-18, "Balanced"),
    (-12, "Relaxed"),
    (-6, "Very relaxed"),
];

#[derive(Clone, Copy, PartialEq)]
enum Mood {
    Quiet,
    Good,
    Careful,
    TooLoud,
    Calibrating,
    NoMic,
}

impl Mood {
    fn rgb(self) -> (f64, f64, f64) {
        match self {
            Mood::Quiet => (0.55, 0.62, 0.74),
            Mood::Good => (0.25, 0.95, 0.62),
            Mood::Careful => (1.0, 0.74, 0.22),
            Mood::TooLoud => (1.0, 0.27, 0.40),
            Mood::Calibrating => (0.62, 0.52, 1.0),
            Mood::NoMic => (0.45, 0.45, 0.5),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Mood::Quiet => "QUIET",
            Mood::Good => "GOOD",
            Mood::Careful => "EASY…",
            Mood::TooLoud => "TOO LOUD",
            Mood::Calibrating => "SPEAK…",
            Mood::NoMic => "NO MIC",
        }
    }
}

fn cg(r: f64, g: f64, b: f64, a: f64) -> CFRetained<CGColor> {
    CGColor::new_srgb(r, g, b, a)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

fn db_frac(db: f32) -> f64 {
    (((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0)) as f64
}

struct Ui {
    panel: Retained<NSPanel>,
    status_item: Retained<NSStatusItem>,
    toggle_item: Retained<NSMenuItem>,
    jump_item: Retained<NSMenuItem>,
    glass: Retained<NSVisualEffectView>,
    stage: Retained<CALayer>,
    info_item: Retained<NSMenuItem>,
    preset_items: Vec<Retained<NSMenuItem>>,
    alarm_bg: Retained<CALayer>,
    border: Retained<CALayer>,
    orb: Retained<CALayer>,
    ring: Retained<CALayer>,
    segs: Vec<Retained<CALayer>>,
    marker: Retained<CALayer>,
    db_text: Retained<CATextLayer>,
    mood_text: Retained<CATextLayer>,
}

struct State {
    thr: f32,
    smooth: f32,
    peak: f32,
    peak_hold: u32,
    loud_ticks: u32,
    alarm_ticks: u32,
    tick: u64,
    mood: Option<Mood>,
    lit: Vec<bool>,
    on_colors: Vec<CFRetained<CGColor>>,
    off_colors: Vec<CFRetained<CGColor>>,
    calib: Option<(u32, Vec<f32>)>,
    last_db_text: i32,
    jump: bool,
    /// 0 = home (small), 1 = centred and scaled up
    zoom: f64,
    home: CGPoint,
}

struct Ivars {
    level: Arc<AtomicU32>,
    has_mic: bool,
    ui: OnceCell<Ui>,
    st: RefCell<State>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VMController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _sender: &AnyObject) {
            self.on_tick();
        }

        #[unsafe(method(toggleHud:))]
        fn toggle_hud(&self, _sender: &AnyObject) {
            let ui = self.ui();
            if ui.panel.isVisible() {
                ui.panel.orderOut(None);
            } else {
                ui.panel.orderFrontRegardless();
            }
            self.refresh_menu();
        }

        #[unsafe(method(toggleJump:))]
        fn toggle_jump(&self, _sender: &AnyObject) {
            let jump = {
                let mut st = self.ivars().st.borrow_mut();
                st.jump = !st.jump;
                st.jump
            };
            unsafe {
                NSUserDefaults::standardUserDefaults().setBool_forKey(jump, &NSString::from_str(JUMP_KEY));
            }
            self.refresh_menu();
        }

        #[unsafe(method(setPreset:))]
        fn set_preset(&self, sender: &NSMenuItem) {
            self.set_threshold(sender.tag() as f32);
        }

        #[unsafe(method(calibrate:))]
        fn calibrate(&self, _sender: &AnyObject) {
            let ui = self.ui();
            if !ui.panel.isVisible() {
                ui.panel.orderFrontRegardless();
            }
            self.ivars().st.borrow_mut().calib = Some(((5.0 * FPS) as u32, Vec::new()));
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker, level: Arc<AtomicU32>, has_mic: bool) -> Retained<Self> {
        let saved = unsafe { NSUserDefaults::standardUserDefaults().floatForKey(&NSString::from_str(PREF_KEY)) };
        let thr = if saved < -3.0 && saved > -60.0 {
            saved
        } else {
            DEFAULT_THR
        };
        let mut st = State {
            thr,
            smooth: MIN_DB,
            peak: MIN_DB,
            peak_hold: 0,
            loud_ticks: 0,
            alarm_ticks: 0,
            tick: 0,
            mood: None,
            lit: vec![true; N_SEG],
            on_colors: Vec::new(),
            off_colors: Vec::new(),
            calib: None,
            last_db_text: i32::MIN,
            jump: unsafe {
                let d = NSUserDefaults::standardUserDefaults();
                let k = NSString::from_str(JUMP_KEY);
                d.objectForKey(&k).is_none() || d.boolForKey(&k)
            },
            zoom: 0.0,
            home: CGPoint::new(0.0, 0.0),
        };
        build_colors(&mut st);
        let this = Self::alloc(mtm).set_ivars(Ivars {
            level,
            has_mic,
            ui: OnceCell::new(),
            st: RefCell::new(st),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn ui(&self) -> &Ui {
        self.ivars().ui.get().expect("ui built")
    }

    fn build(&self, mtm: MainThreadMarker) {
        unsafe {
            // ---- Floating glass panel ----
            let frame = rect(0.0, 0.0, W, H);
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setReleasedWhenClosed(false);
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
            if let Some(app) = NSAppearance::appearanceNamed(NSAppearanceNameVibrantDark) {
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
            ring.setFrame(rect(40.0 - 22.0, H / 2.0 - 22.0, 44.0, 44.0));
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
            let base_y = 20.0;
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
            db_text.setFrame(rect(258.0, 34.0, 66.0, 26.0));
            db_text.setContentsScale(scale);
            db_text.setAlignmentMode(kCAAlignmentRight);
            db_text.setForegroundColor(Some(&cg(1.0, 1.0, 1.0, 0.95)));
            let f = NSFont::monospacedDigitSystemFontOfSize_weight(20.0, NSFontWeightSemibold);
            let _: () = msg_send![&*db_text, setFont: &*f];
            db_text.setFontSize(20.0);
            stage.addSublayer(&db_text);

            let mood_text = CATextLayer::new();
            mood_text.setFrame(rect(240.0, 18.0, 84.0, 14.0));
            mood_text.setContentsScale(scale);
            mood_text.setAlignmentMode(kCAAlignmentRight);
            let f = NSFont::systemFontOfSize_weight(10.0, NSFontWeightHeavy);
            let _: () = msg_send![&*mood_text, setFont: &*f];
            mood_text.setFontSize(10.0);
            stage.addSublayer(&mood_text);

            // ---- Menu (status bar + right-click on HUD) ----
            let menu = NSMenu::new(mtm);
            menu.setAutoenablesItems(false);
            let item = |title: &str, action: Option<objc2::runtime::Sel>| {
                let it = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    action,
                    &NSString::from_str(""),
                );
                if action.is_some() {
                    it.setTarget(Some(self));
                } else {
                    it.setEnabled(false);
                }
                menu.addItem(&it);
                it
            };
            let info_item = item("", None);
            let toggle_item = item("Hide Meter", Some(sel!(toggleHud:)));
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            let jump_item = item("Jump to Center When Too Loud", Some(sel!(toggleJump:)));
            item("Calibrate to My Voice (speak 5 s)", Some(sel!(calibrate:)));
            item("Too-loud level:", None);
            let mut preset_items = Vec::new();
            for (db, name) in PRESETS {
                let it = item(&format!("   {name}  ({db} dB)"), Some(sel!(setPreset:)));
                it.setTag(db as isize);
                preset_items.push(it);
            }
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            let q = item("Quit Voice Meter", Some(sel!(quit:)));
            q.setKeyEquivalent(&NSString::from_str("q"));

            canvas.setMenu(Some(&menu));

            let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
            if let Some(btn) = status_item.button(mtm)
                && let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("waveform"),
                    Some(&NSString::from_str("Voice Meter")),
                )
            {
                img.setTemplate(true);
                btn.setImage(Some(&img));
            }
            status_item.setMenu(Some(&menu));

            // ---- Position: top-center, or the last place the user dragged it ----
            let home = saved_home(mtm).unwrap_or_else(|| {
                NSScreen::mainScreen(mtm)
                    .map(|screen| {
                        let vf = screen.visibleFrame();
                        CGPoint::new(
                            vf.origin.x + (vf.size.width - W) / 2.0,
                            vf.origin.y + vf.size.height - H - 12.0,
                        )
                    })
                    .unwrap_or(CGPoint::new(100.0, 100.0))
            });
            panel.setFrameOrigin(home);
            self.ivars().st.borrow_mut().home = home;
            panel.orderFrontRegardless();
            panel.invalidateShadow();

            let _ = self.ivars().ui.set(Ui {
                panel,
                status_item,
                toggle_item,
                jump_item,
                glass,
                stage,
                info_item,
                preset_items,
                alarm_bg,
                border,
                orb,
                ring,
                segs,
                marker,
                db_text,
                mood_text,
            });

            self.place_marker();
            self.refresh_menu();

            let timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / FPS,
                self,
                sel!(tick:),
                None,
                true,
            );
            NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
    }

    fn set_threshold(&self, thr: f32) {
        {
            let mut st = self.ivars().st.borrow_mut();
            st.thr = thr;
            build_colors(&mut st);
            st.lit.iter_mut().for_each(|l| *l = !*l); // force repaint
        }
        unsafe {
            NSUserDefaults::standardUserDefaults().setFloat_forKey(thr, &NSString::from_str(PREF_KEY));
        }
        self.place_marker();
        self.refresh_menu();
    }

    fn place_marker(&self) {
        let ui = self.ui();
        let thr = self.ivars().st.borrow().thr;
        let x = METER_X + db_frac(thr) * METER_W;
        unsafe { ui.marker.setFrame(rect(x - 1.0, 14.0, 2.0, 48.0)) };
    }

    fn refresh_menu(&self) {
        let ui = self.ui();
        let (thr, jump) = {
            let st = self.ivars().st.borrow();
            (st.thr, st.jump)
        };
        unsafe {
            ui.jump_item.setState(if jump { 1 } else { 0 });
            ui.info_item
                .setTitle(&NSString::from_str(&format!("Alert above {:.0} dB", thr)));
            ui.toggle_item.setTitle(&NSString::from_str(if ui.panel.isVisible() {
                "Hide Meter"
            } else {
                "Show Meter"
            }));
            for it in &ui.preset_items {
                it.setState(if it.tag() == thr.round() as isize { 1 } else { 0 });
            }
        }
    }

    fn on_tick(&self) {
        let ivars = self.ivars();
        let ui = self.ui();
        let raw = f32::from_bits(ivars.level.swap(0, Ordering::Relaxed));
        let db = if raw > 1e-7 { 20.0 * raw.log10() } else { -90.0 };

        let mut finished_calib: Option<f32> = None;
        let mut st = ivars.st.borrow_mut();
        st.tick += 1;
        // fast attack, slower release: reacts instantly, reads calmly
        let k = if db > st.smooth { 0.55 } else { 0.10 };
        st.smooth += (db.max(MIN_DB - 5.0) - st.smooth) * k;
        let smooth = st.smooth;

        if smooth > st.peak {
            st.peak = smooth;
            st.peak_hold = (FPS * 0.9) as u32;
        } else if st.peak_hold > 0 {
            st.peak_hold -= 1;
        } else {
            st.peak -= 0.7;
        }

        if smooth >= st.thr {
            st.loud_ticks += 1
        } else {
            st.loud_ticks = 0
        }
        if st.loud_ticks >= 5 {
            st.alarm_ticks = (FPS * 1.6) as u32;
        } else if st.alarm_ticks > 0 {
            st.alarm_ticks -= 1;
        }

        let thr = st.thr;
        let mut calib_secs = 0;
        if let Some((left, samples)) = st.calib.as_mut() {
            if smooth > -55.0 {
                samples.push(smooth);
            }
            *left -= 1;
            calib_secs = (*left as f64 / FPS).ceil() as i32;
            if *left == 0 {
                if samples.len() > 20 {
                    samples.sort_by(|a, b| a.total_cmp(b));
                    let p90 = samples[samples.len() * 9 / 10];
                    finished_calib = Some((p90 + 6.0).round().clamp(-45.0, -3.0));
                }
                st.calib = None;
            }
        }

        let mood = if !ivars.has_mic {
            Mood::NoMic
        } else if st.calib.is_some() {
            Mood::Calibrating
        } else if st.alarm_ticks > 0 {
            Mood::TooLoud
        } else if smooth >= thr - 6.0 {
            Mood::Careful
        } else if smooth >= thr - 26.0 {
            Mood::Good
        } else {
            Mood::Quiet
        };
        let mood_changed = st.mood != Some(mood);
        st.mood = Some(mood);
        let (r, g, b) = mood.rgb();

        unsafe {
            if mood_changed && let Some(btn) = ui.status_item.button(self.mtm()) {
                let tint = match mood {
                    Mood::Quiet | Mood::NoMic => None,
                    _ => Some(NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)),
                };
                btn.setContentTintColor(tint.as_deref());
            }

            if ui.panel.isVisible() {
                CATransaction::begin();
                CATransaction::setDisableActions(true);

                let frac = db_frac(smooth);
                let lit_n = (frac * N_SEG as f64).round() as usize;
                let peak_i = ((db_frac(st.peak) * N_SEG as f64).round() as usize).saturating_sub(1);
                for i in 0..N_SEG {
                    let on = i < lit_n || (i == peak_i && st.peak > MIN_DB + 2.0);
                    if st.lit[i] != on {
                        st.lit[i] = on;
                        let c = if on { &st.on_colors[i] } else { &st.off_colors[i] };
                        ui.segs[i].setBackgroundColor(Some(c));
                    }
                }

                let pulse = 0.5 + 0.5 * ((st.tick as f64) * 0.45).sin();
                let d = 14.0 + 22.0 * frac;
                ui.orb.setFrame(rect(40.0 - d / 2.0, H / 2.0 - d / 2.0, d, d));
                ui.orb.setCornerRadius(d / 2.0);
                ui.orb.setShadowRadius(4.0 + 14.0 * frac);
                if mood_changed {
                    ui.orb.setBackgroundColor(Some(&cg(r, g, b, 1.0)));
                    ui.orb.setShadowColor(Some(&cg(r, g, b, 1.0)));
                    ui.ring.setBorderColor(Some(&cg(r, g, b, 1.0)));
                    ui.mood_text.setForegroundColor(Some(&cg(r, g, b, 1.0)));
                }

                if mood == Mood::TooLoud {
                    ui.alarm_bg.setOpacity((0.12 + 0.22 * pulse) as f32);
                    ui.ring.setOpacity((0.35 + 0.65 * pulse) as f32);
                    ui.border.setBorderWidth(2.0);
                    ui.border.setBorderColor(Some(&cg(1.0, 0.3, 0.42, 0.5 + 0.5 * pulse)));
                } else if mood_changed {
                    ui.alarm_bg.setOpacity(0.0);
                    ui.ring.setOpacity(if mood == Mood::Calibrating { 0.8 } else { 0.18 });
                    ui.border.setBorderWidth(1.0);
                    ui.border.setBorderColor(Some(&cg(1.0, 1.0, 1.0, 0.10)));
                }

                if mood == Mood::Calibrating {
                    if mood_changed || st.tick.is_multiple_of(6) {
                        let s = format!("SPEAK… {calib_secs}s");
                        let _: () = msg_send![&*ui.mood_text, setString: &*NSString::from_str(&s)];
                    }
                } else if mood_changed {
                    let _: () = msg_send![&*ui.mood_text, setString: &*NSString::from_str(mood.label())];
                }

                // number updates ~7×/s so it's readable
                if st.tick.is_multiple_of(4) || st.last_db_text == i32::MIN {
                    let v = smooth.round() as i32;
                    if v != st.last_db_text {
                        st.last_db_text = v;
                        let s = if smooth <= MIN_DB + 1.0 {
                            "—".to_string()
                        } else {
                            format!("{}", v).replace('-', "−")
                        };
                        let _: () = msg_send![&*ui.db_text, setString: &*NSString::from_str(&format!("{s} dB"))];
                    }
                }

                CATransaction::commit();
            }
        }
        self.update_zoom(&mut st, mood == Mood::TooLoud);
        drop(st);
        if let Some(t) = finished_calib {
            self.set_threshold(t);
        }
    }
}

impl Controller {
    /// Glides the HUD between its home spot and a scaled-up, centred position.
    fn update_zoom(&self, st: &mut State, too_loud: bool) {
        let ui = self.ui();
        let panel = &ui.panel;
        let target = if too_loud && st.jump && panel.isVisible() {
            1.0
        } else {
            0.0
        };

        if st.zoom == 0.0 {
            // At rest: track where the user dragged the HUD and remember it.
            let o = panel.frame().origin;
            if o.x != st.home.x || o.y != st.home.y {
                st.home = o;
                unsafe {
                    let d = NSUserDefaults::standardUserDefaults();
                    d.setDouble_forKey(o.x, &NSString::from_str(HOME_X_KEY));
                    d.setDouble_forKey(o.y, &NSString::from_str(HOME_Y_KEY));
                }
            }
            if target == 0.0 {
                return;
            }
        }
        if st.zoom == target {
            return;
        }

        // springy approach: snappy in, gentle out
        let k = if target > st.zoom { 0.28 } else { 0.14 };
        st.zoom += (target - st.zoom) * k;
        if (target - st.zoom).abs() < 0.004 {
            st.zoom = target;
        }
        let e = st.zoom;

        let mtm = self.mtm();
        let home_c = CGPoint::new(st.home.x + W / 2.0, st.home.y + H / 2.0);
        let Some(screen) = screen_containing(mtm, home_c).or_else(|| NSScreen::mainScreen(mtm)) else {
            return;
        };
        let vf = screen.visibleFrame();
        let center = CGPoint::new(vf.origin.x + vf.size.width / 2.0, vf.origin.y + vf.size.height / 2.0);

        let s = 1.0 + (ZOOM_SCALE - 1.0) * e;
        let (w, h) = (W * s, H * s);
        let cx = home_c.x + (center.x - home_c.x) * e;
        let cy = home_c.y + (center.y - home_c.y) * e;
        let f = rect(cx - w / 2.0, cy - h / 2.0, w, h);

        unsafe {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            ui.stage.setTransform(scale3d(s));
            if let Some(l) = ui.glass.subviews().firstObject().and_then(|c| c.layer()) {
                l.setCornerRadius(CORNER * s);
            }
            CATransaction::commit();
            ui.glass.setMaskImage(Some(&pill_mask(CORNER * s)));
            panel.setFrame_display(f, true);
            panel.invalidateShadow();
        }
    }
}

/// Stretchable stadium-shaped mask: opaque inside the pill, transparent corners.
fn pill_mask(radius: f64) -> Retained<NSImage> {
    let r = radius.round().max(1.0);
    let size = CGSize::new(r * 2.0 + 1.0, r * 2.0 + 1.0);
    unsafe {
        let img = NSImage::initWithSize(objc2::AllocAnyThread::alloc(), size);
        #[allow(deprecated)]
        img.lockFocus();
        NSColor::blackColor().set();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(0.0, 0.0, size.width, size.height), r, r).fill();
        #[allow(deprecated)]
        img.unlockFocus();
        img.setCapInsets(NSEdgeInsets {
            top: r,
            left: r,
            bottom: r,
            right: r,
        });
        // objc2-app-kit 0.3 has Stretch/Tile swapped on Apple Silicon; use AppKit's real values.
        let stretch = if cfg!(target_arch = "x86_64") { 0 } else { 1 };
        img.setResizingMode(NSImageResizingMode(stretch));
        img
    }
}

fn scale3d(s: f64) -> CATransform3D {
    CATransform3D {
        m11: s,
        m12: 0.0,
        m13: 0.0,
        m14: 0.0,
        m21: 0.0,
        m22: s,
        m23: 0.0,
        m24: 0.0,
        m31: 0.0,
        m32: 0.0,
        m33: 1.0,
        m34: 0.0,
        m41: 0.0,
        m42: 0.0,
        m43: 0.0,
        m44: 1.0,
    }
}

fn screen_containing(mtm: MainThreadMarker, p: CGPoint) -> Option<Retained<NSScreen>> {
    let screens: Retained<NSArray<NSScreen>> = NSScreen::screens(mtm);
    screens.iter().find(|sc| {
        let f = sc.frame();
        p.x >= f.origin.x && p.x < f.origin.x + f.size.width && p.y >= f.origin.y && p.y < f.origin.y + f.size.height
    })
}

fn saved_home(mtm: MainThreadMarker) -> Option<CGPoint> {
    unsafe {
        let d = NSUserDefaults::standardUserDefaults();
        d.objectForKey(&NSString::from_str(HOME_X_KEY))?;
        let p = CGPoint::new(
            d.doubleForKey(&NSString::from_str(HOME_X_KEY)),
            d.doubleForKey(&NSString::from_str(HOME_Y_KEY)),
        );
        // only reuse it if it is still on a connected display
        screen_containing(mtm, CGPoint::new(p.x + W / 2.0, p.y + H / 2.0)).map(|_| p)
    }
}

fn build_colors(st: &mut State) {
    let amber_from = st.thr - 8.0;
    let amber_idx = (db_frac(amber_from) * N_SEG as f64).max(1.0);
    st.on_colors.clear();
    st.off_colors.clear();
    for i in 0..N_SEG {
        let seg_db = MIN_DB + (i as f32 + 0.5) / N_SEG as f32 * (MAX_DB - MIN_DB);
        let (r, g, b) = if seg_db >= st.thr {
            (1.0, 0.27, 0.40)
        } else if seg_db >= amber_from {
            (1.0, 0.74, 0.22)
        } else {
            // cyan → mint gradient across the safe zone
            let t = (i as f64 / amber_idx).clamp(0.0, 1.0);
            (0.20 + 0.05 * t, 0.75 + 0.20 * t, 1.0 - 0.38 * t)
        };
        st.on_colors.push(cg(r, g, b, 1.0));
        st.off_colors.push(cg(r, g, b, 0.14));
    }
}

/// Publishes the loudest RMS seen since the UI last read it. For non-negative
/// f32 values, bit patterns order like the numbers, so `fetch_max` on the bits works.
fn publish(level: &AtomicU32, sum_sq: f32, n: usize) {
    if n == 0 {
        return;
    }
    let rms = (sum_sq / n as f32).sqrt();
    level.fetch_max(rms.to_bits(), Ordering::Relaxed);
}

fn start_audio(level: Arc<AtomicU32>) -> Option<cpal::Stream> {
    let dev = cpal::default_host().default_input_device()?;
    let cfg = dev.default_input_config().ok()?;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let err = |e| eprintln!("voice-meter: audio error: {e}");
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            &config,
            move |d: &[f32], _: &_| publish(&level, d.iter().map(|s| s * s).sum(), d.len()),
            err,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_input_stream(
            &config,
            move |d: &[i16], _: &_| {
                publish(
                    &level,
                    d.iter()
                        .map(|&s| {
                            let f = s as f32 / 32768.0;
                            f * f
                        })
                        .sum(),
                    d.len(),
                )
            },
            err,
            None,
        ),
        cpal::SampleFormat::I32 => dev.build_input_stream(
            &config,
            move |d: &[i32], _: &_| {
                publish(
                    &level,
                    d.iter()
                        .map(|&s| {
                            let f = s as f32 / 2147483648.0;
                            f * f
                        })
                        .sum(),
                    d.len(),
                )
            },
            err,
            None,
        ),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(stream)
}

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let level = Arc::new(AtomicU32::new(0));
    let stream = start_audio(level.clone());
    let ctrl = Controller::new(mtm, level, stream.is_some());
    ctrl.build(mtm);

    app.run();
    drop(stream);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_frac_maps_range_and_clamps() {
        assert_eq!(db_frac(MIN_DB), 0.0);
        assert_eq!(db_frac(MAX_DB), 1.0);
        assert!((db_frac(-30.0) - 0.5).abs() < 1e-9);
        assert_eq!(db_frac(-100.0), 0.0);
        assert_eq!(db_frac(10.0), 1.0);
    }

    #[test]
    fn publish_keeps_loudest_rms() {
        let level = AtomicU32::new(0);
        publish(&level, 0.25 * 4.0, 4); // rms 0.5
        publish(&level, 0.01 * 4.0, 4); // rms 0.1, quieter: ignored
        assert!((f32::from_bits(level.load(Ordering::Relaxed)) - 0.5).abs() < 1e-6);
        publish(&level, 0.0, 0); // empty buffer: no-op
        assert!((f32::from_bits(level.load(Ordering::Relaxed)) - 0.5).abs() < 1e-6);
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
    }
}
