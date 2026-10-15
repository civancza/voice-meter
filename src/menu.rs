//! Menu-bar item and shared status/HUD context menu.
use crate::{
    Controller,
    hud::Hud,
    meter::{self, Frame},
};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSColor, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::NSString;

const PRESETS: [(i32, &str); 5] = [
    (-30, "Very strict"),
    (-24, "Strict"),
    (-18, "Balanced"),
    (-12, "Relaxed"),
    (-6, "Very relaxed"),
];

pub struct Menu {
    status_item: Retained<NSStatusItem>,
    toggle_item: Retained<NSMenuItem>,
    jump_item: Retained<NSMenuItem>,
    info_item: Retained<NSMenuItem>,
    preset_items: Vec<Retained<NSMenuItem>>,
}

impl Menu {
    pub fn build(mtm: MainThreadMarker, controller: &Controller, hud: &Hud) -> Self {
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let item = |title: &str, action: Option<objc2::runtime::Sel>| {
            let it = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    action,
                    &NSString::from_str(""),
                )
            };
            if action.is_some() {
                unsafe {
                    it.setTarget(Some(controller));
                }
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

        unsafe {
            hud.canvas.setMenu(Some(&menu));
        }

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
        Self {
            status_item,
            toggle_item,
            jump_item,
            info_item,
            preset_items,
        }
    }

    pub fn refresh(&self, threshold: f32, jump: bool, visible: bool) {
        self.jump_item.setState(if jump { 1 } else { 0 });
        self.info_item
            .setTitle(&NSString::from_str(&meter::threshold_label(threshold)));
        self.toggle_item
            .setTitle(&NSString::from_str(if visible { "Hide Meter" } else { "Show Meter" }));
        for item in &self.preset_items {
            item.setState(if item.tag() == threshold.round() as isize { 1 } else { 0 });
        }
    }

    pub fn render(&self, mtm: MainThreadMarker, frame: &Frame) {
        if frame.mood_changed
            && let Some(button) = self.status_item.button(mtm)
        {
            let tint = frame
                .status_tint
                .map(|(r, g, b)| NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0));
            button.setContentTintColor(tint.as_deref());
        }
    }
}
