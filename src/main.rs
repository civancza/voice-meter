//! Voice Meter — a tiny floating HUD that shows how loudly you speak.
mod audio;
mod hearing;
mod hud;
mod menu;
mod meter;
mod prefs;

use hud::{Hud, Placement};
use menu::Menu;
use meter::Meter;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSMenuItem, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObject, NSRunLoop, NSRunLoopCommonModes, NSTimer};
use prefs::Prefs;
use std::cell::{OnceCell, RefCell};

struct Ui {
    hud: Hud,
    menu: Menu,
    placement: Placement,
}

struct Ivars {
    capture: RefCell<audio::Capture>,
    hearing: RefCell<hearing::Monitor>,
    ear: RefCell<hearing::Ear>,
    ui: OnceCell<Ui>,
    meter: RefCell<Meter>,
    prefs: Prefs,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VMController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowDidMove:))]
        fn window_did_move(&self, _notification: &NSNotification) {
            if let Some(ui) = self.ivars().ui.get() {
                ui.placement.did_move(&ui.hud, &self.ivars().prefs);
            }
        }
    }

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _sender: &AnyObject) { self.on_tick(); }

        #[unsafe(method(toggleHud:))]
        fn toggle_hud(&self, _sender: &AnyObject) {
            let hud = &self.ui().hud;
            if hud.panel.isVisible() { hud.panel.orderOut(None); } else { hud.show(); }
            self.refresh_menu();
        }

        #[unsafe(method(toggleJump:))]
        fn toggle_jump(&self, _sender: &AnyObject) {
            let jump = !self.ui().placement.jump.get();
            self.ui().placement.jump.set(jump);
            self.ivars().prefs.set_jump_when_loud(jump);
            self.refresh_menu();
        }

        #[unsafe(method(setPreset:))]
        fn set_preset(&self, sender: &NSMenuItem) { self.set_threshold(sender.tag() as f32); }

        #[unsafe(method(calibrate:))]
        fn calibrate(&self, _sender: &AnyObject) {
            self.ivars().meter.borrow_mut().start_calibration();
            if !self.ui().hud.panel.isVisible() { self.ui().hud.show(); }
            self.refresh_menu();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let prefs = Prefs::load();
        let threshold = prefs.threshold().unwrap_or(meter::DEFAULT_THRESHOLD);
        let this = Self::alloc(mtm).set_ivars(Ivars {
            capture: RefCell::new(audio::Capture::new()),
            hearing: RefCell::new(hearing::Monitor::new()),
            ear: RefCell::new(hearing::Ear::new()),
            ui: OnceCell::new(),
            meter: RefCell::new(Meter::new(threshold)),
            prefs,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn ui(&self) -> &Ui {
        self.ivars().ui.get().expect("ui built")
    }

    fn build(&self, mtm: MainThreadMarker) {
        let hud = Hud::build(mtm);
        hud.set_threshold(self.ivars().meter.borrow().threshold());
        let menu = Menu::build(mtm, self, &hud);
        let placement = Placement::new(mtm, &self.ivars().prefs);
        placement.position(&hud);
        assert!(self.ivars().ui.set(Ui { hud, menu, placement }).is_ok());
        self.ui().hud.panel.setDelegate(Some(ProtocolObject::from_ref(self)));
        self.ui().hud.show();
        self.refresh_menu();
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / meter::FPS,
                self,
                sel!(tick:),
                None,
                true,
            )
        };
        unsafe {
            NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
    }

    fn set_threshold(&self, threshold: f32) {
        let threshold = meter::clamp_threshold(threshold);
        self.ivars().meter.borrow_mut().set_threshold(threshold);
        self.ivars().prefs.set_threshold(threshold);
        self.ui().hud.set_threshold(threshold);
        self.refresh_menu();
    }

    fn refresh_menu(&self) {
        let ui = self.ui();
        ui.menu.refresh(
            self.ivars().meter.borrow().threshold(),
            ui.placement.jump.get(),
            ui.hud.panel.isVisible(),
        );
    }

    fn on_tick(&self) {
        let db = self.ivars().capture.borrow_mut().poll().map(meter::rms_db);
        let frame = self.ivars().meter.borrow_mut().step(db);
        let ui = self.ui();
        ui.menu.render(self.mtm(), &frame);
        ui.hud.render(&frame);
        let reading = self.ivars().hearing.borrow_mut().poll();
        ui.hud.render_hearing(&self.ivars().ear.borrow_mut().step(reading));
        ui.placement.update_zoom(&ui.hud, frame.too_loud);
        if let Some(threshold) = frame.calibration_result {
            self.set_threshold(threshold);
        }
    }
}

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let controller = Controller::new(mtm);
    controller.build(mtm);
    app.run();
}
