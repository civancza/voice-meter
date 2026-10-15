//! Window placement is independent of meter state and the last rendered snapshot.
use super::{CORNER, H, Hud, Prefs, W, ZOOM_SCALE, rect};
use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBezierPath, NSColor, NSImage, NSImageResizingMode, NSScreen};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_core_graphics::CGRectContainsPoint;
use objc2_foundation::NSEdgeInsets;
use objc2_quartz_core::CATransaction;
#[allow(deprecated)]
use objc2_quartz_core::CATransform3DMakeScale;
use std::cell::Cell;

pub struct Placement {
    pub jump: Cell<bool>,
    zoom: Cell<f64>,
    home: Cell<CGPoint>,
    animating: Cell<bool>,
}

impl Placement {
    pub fn new(mtm: MainThreadMarker, prefs: &Prefs) -> Self {
        let saved = prefs
            .home()
            .map(|(x, y)| CGPoint::new(x, y))
            .filter(|p| screen_containing(mtm, CGPoint::new(p.x + W / 2.0, p.y + H / 2.0)).is_some());
        let home = saved.unwrap_or_else(|| {
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
        Self {
            jump: Cell::new(prefs.jump_when_loud()),
            zoom: Cell::new(0.0),
            home: Cell::new(home),
            animating: Cell::new(false),
        }
    }

    pub fn position(&self, hud: &Hud) {
        hud.panel.setFrameOrigin(self.home.get());
        hud.panel.invalidateShadow();
    }

    /// Delegate notifications also occur synchronously during our own animation.
    /// Ignore those, as well as moves while zoomed, to retain the unscaled home.
    pub fn did_move(&self, hud: &Hud, prefs: &Prefs) {
        if self.animating.get() || self.zoom.get() != 0.0 {
            return;
        }
        let origin = hud.panel.frame().origin;
        if origin != self.home.get() {
            self.home.set(origin);
            prefs.set_home(origin.x, origin.y);
        }
    }

    /// Glides the HUD between its home spot and a scaled-up, centred position.
    pub fn update_zoom(&self, hud: &Hud, too_loud: bool) {
        let target = if too_loud && self.jump.get() && hud.panel.isVisible() {
            1.0
        } else {
            0.0
        };
        let zoom = self.zoom.get();
        if zoom == target {
            return;
        }
        let k = if target > zoom { 0.28 } else { 0.14 };
        let mut e = zoom + (target - zoom) * k;
        if (target - e).abs() < 0.004 {
            e = target;
        }
        self.zoom.set(e);
        let home = self.home.get();
        let home_c = CGPoint::new(home.x + W / 2.0, home.y + H / 2.0);
        let mtm = hud.panel.mtm();
        let Some(screen) = screen_containing(mtm, home_c).or_else(|| NSScreen::mainScreen(mtm)) else {
            return;
        };
        let vf = screen.visibleFrame();
        let center = CGPoint::new(vf.origin.x + vf.size.width / 2.0, vf.origin.y + vf.size.height / 2.0);
        let s = 1.0 + (ZOOM_SCALE - 1.0) * e;
        let (w, h) = (W * s, H * s);
        let cx = home_c.x + (center.x - home_c.x) * e;
        let cy = home_c.y + (center.y - home_c.y) * e;
        self.animating.set(true);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        #[allow(deprecated)] // Requested C API, renamed by objc2 0.3.2.
        hud.stage.setTransform(CATransform3DMakeScale(s, s, 1.0));
        if let Some(layer) = hud.canvas.layer() {
            layer.setCornerRadius(CORNER * s);
        }
        CATransaction::commit();
        hud.glass.setMaskImage(Some(&pill_mask(CORNER * s)));
        hud.panel.setFrame_display(rect(cx - w / 2.0, cy - h / 2.0, w, h), true);
        hud.panel.invalidateShadow();
        self.animating.set(false);
    }
}

/// Stretchable stadium mask. Raw resizing values work around the arm64 binding quirk.
pub(super) fn pill_mask(radius: f64) -> Retained<NSImage> {
    let r = radius.round().max(1.0);
    let size = CGSize::new(r * 2.0 + 1.0, r * 2.0 + 1.0);
    let img = NSImage::initWithSize(NSImage::alloc(), size);
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
    let stretch = if cfg!(target_arch = "x86_64") { 0 } else { 1 };
    img.setResizingMode(NSImageResizingMode(stretch));
    img
}

fn screen_containing(mtm: MainThreadMarker, point: CGPoint) -> Option<Retained<NSScreen>> {
    NSScreen::screens(mtm)
        .iter()
        .find(|screen| CGRectContainsPoint(screen.frame(), point))
}
