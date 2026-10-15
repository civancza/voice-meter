//! Typed access to the HUD's persisted settings.

use objc2::rc::Retained;
use objc2_foundation::{NSString, NSUserDefaults};

const THRESHOLD_KEY: &str = "thresholdDb";
const JUMP_KEY: &str = "jumpWhenLoud";
const HOME_X_KEY: &str = "hudX";
const HOME_Y_KEY: &str = "hudY";

pub struct Prefs {
    defaults: Retained<NSUserDefaults>,
}

impl Prefs {
    pub fn load() -> Self {
        Self {
            defaults: NSUserDefaults::standardUserDefaults(),
        }
    }

    /// Returns the raw saved threshold, leaving validation to the caller.
    pub fn threshold(&self) -> Option<f32> {
        let key = NSString::from_str(THRESHOLD_KEY);
        self.defaults.objectForKey(&key)?;
        Some(self.defaults.floatForKey(&key))
    }

    pub fn set_threshold(&self, threshold: f32) {
        self.defaults
            .setFloat_forKey(threshold, &NSString::from_str(THRESHOLD_KEY));
    }

    /// Jumping is enabled by default when the preference has not been saved.
    pub fn jump_when_loud(&self) -> bool {
        let key = NSString::from_str(JUMP_KEY);
        self.defaults.objectForKey(&key).is_none() || self.defaults.boolForKey(&key)
    }

    pub fn set_jump_when_loud(&self, jump: bool) {
        self.defaults.setBool_forKey(jump, &NSString::from_str(JUMP_KEY));
    }

    /// Returns the saved origin; the caller checks whether its display still exists.
    /// As before, a saved X with no Y uses NSUserDefaults' zero default for Y.
    pub fn home(&self) -> Option<(f64, f64)> {
        let x_key = NSString::from_str(HOME_X_KEY);
        self.defaults.objectForKey(&x_key)?;
        Some((
            self.defaults.doubleForKey(&x_key),
            self.defaults.doubleForKey(&NSString::from_str(HOME_Y_KEY)),
        ))
    }

    pub fn set_home(&self, x: f64, y: f64) {
        self.defaults.setDouble_forKey(x, &NSString::from_str(HOME_X_KEY));
        self.defaults.setDouble_forKey(y, &NSString::from_str(HOME_Y_KEY));
    }
}
