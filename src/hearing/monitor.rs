//! Hearing I/O: a system-audio process tap plus the default output's mute/volume properties.

use super::Reading;
use super::tap::{self, Tap};
use crate::audio::Health;
use crate::meter;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectHasProperty, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectPropertyElement, AudioObjectPropertyScope, AudioObjectPropertySelector, kAudioDevicePropertyMute,
    kAudioDevicePropertyVolumeDecibels, kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};
use std::ffi::c_void;
use std::ptr::{NonNull, null};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const VOLUME_INTERVAL: Duration = Duration::from_millis(500);
const CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Volume {
    NoDevice,
    Muted,
    /// Device gain in dB; 0 for outputs without a software volume (e.g. HDMI).
    Gain(f32),
}

enum Capture {
    /// Retries once per second; a repeated identical error is logged once.
    Idle {
        last_error: Option<String>,
    },
    /// Starting can block for as long as the system audio permission prompt is open, so it runs
    /// on a helper thread and the HUD keeps animating.
    Starting {
        started: Receiver<Result<Tap, String>>,
        last_error: Option<String>,
    },
    Running {
        tap: Tap,
        health: Health,
    },
    /// Process taps do not exist on this macOS version.
    Unsupported,
}

impl Capture {
    fn new() -> Self {
        if tap::supported() {
            Self::Idle { last_error: None }
        } else {
            eprintln!("voice-meter: hearing level needs macOS 14.2 or later");
            Self::Unsupported
        }
    }

    /// Starts the tap, and restarts it if Core Audio stops delivering buffers (e.g. coreaudiod
    /// restart). Buffers only flow once system audio permission is granted, so a tap that never
    /// delivered is waiting for the user, not stalled.
    fn refresh(self, now: Instant) -> Self {
        match self {
            Self::Running { tap, health } if health.receiving && health.stalled(now) => {
                eprintln!("voice-meter: hearing capture stopped; restarting");
                drop(tap);
                start(None)
            }
            Self::Idle { last_error } => start(last_error),
            Self::Starting { started, last_error } => match started.try_recv() {
                Err(TryRecvError::Empty) => Self::Starting { started, last_error },
                Err(TryRecvError::Disconnected) => Self::Idle { last_error },
                Ok(Ok(tap)) => {
                    eprintln!("voice-meter: hearing capture started");
                    Self::Running {
                        tap,
                        health: Health::new(now),
                    }
                }
                Ok(Err(error)) => {
                    if last_error.as_ref() != Some(&error) {
                        eprintln!("voice-meter: {error}; retrying automatically");
                    }
                    Self::Idle {
                        last_error: Some(error),
                    }
                }
            },
            running_or_unsupported => running_or_unsupported,
        }
    }
}

fn start(last_error: Option<String>) -> Capture {
    let (sender, started) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("hearing-tap".into()).spawn(move || {
        // The receiver may be gone if the app is quitting; nothing to clean up beyond `Tap`'s Drop.
        let _ = sender.send(Tap::start());
    });
    match spawned {
        Ok(_) => Capture::Starting { started, last_error },
        Err(error) => {
            eprintln!("voice-meter: cannot start hearing capture: {error}");
            Capture::Idle { last_error }
        }
    }
}

pub struct Monitor {
    capture: Capture,
    volume: Volume,
    next_volume: Instant,
    next_check: Instant,
}

impl Monitor {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            capture: Capture::new(),
            volume: Volume::NoDevice,
            next_volume: now,
            next_check: now,
        }
    }

    pub fn poll(&mut self) -> Reading {
        let now = Instant::now();
        if let Capture::Running { tap, health } = &mut self.capture {
            health.observe(now, tap.exchange());
        }
        if now >= self.next_volume {
            self.next_volume = now + VOLUME_INTERVAL;
            self.volume = read_volume();
        }
        if now >= self.next_check {
            self.next_check = now + CHECK_INTERVAL;
            self.capture = std::mem::replace(&mut self.capture, Capture::Unsupported).refresh(now);
        }
        let level = match &self.capture {
            Capture::Running { tap, .. } => Some(tap.exchange().take_level()),
            _ => None,
        };
        match (self.volume, level) {
            (Volume::NoDevice, _) | (_, None) => Reading::Unavailable,
            (Volume::Muted, _) => Reading::Muted,
            (Volume::Gain(gain), Some(rms)) => Reading::Level(meter::rms_db(rms) + gain),
        }
    }
}

fn read_volume() -> Volume {
    let Some(device) = property::<AudioObjectID>(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    )
    .filter(|&id| id != 0) else {
        return Volume::NoDevice;
    };
    let muted = property::<u32>(
        device,
        kAudioDevicePropertyMute,
        kAudioObjectPropertyScopeOutput,
        kAudioObjectPropertyElementMain,
    );
    if muted.is_some_and(|m| m != 0) {
        return Volume::Muted;
    }
    let gain = |element| {
        property::<f32>(
            device,
            kAudioDevicePropertyVolumeDecibels,
            kAudioObjectPropertyScopeOutput,
            element,
        )
    };
    // Built-in speakers often expose only per-channel (left/right) volume, not a main control.
    let gain = gain(kAudioObjectPropertyElementMain).or_else(|| {
        let (sum, count) = (1..=2)
            .filter_map(gain)
            .fold((0.0_f32, 0_u8), |(sum, count), v| (sum + v, count + 1));
        (count > 0).then(|| sum / f32::from(count))
    });
    Volume::Gain(gain.unwrap_or(0.0).min(0.0))
}

fn property<T: Copy + Default>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    element: AudioObjectPropertyElement,
) -> Option<T> {
    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: element,
    };
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    // SAFETY: the address/value pointers outlive the calls and `size` matches `T`, the fixed-size
    // type Core Audio documents for these selectors.
    let status = unsafe {
        if !AudioObjectHasProperty(object, NonNull::from(&address)) {
            return None;
        }
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast::<c_void>(),
        )
    };
    (status == 0 && size as usize == size_of::<T>()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_output_volume_never_panics() {
        match read_volume() {
            Volume::Gain(gain) => assert!(gain <= 0.0),
            Volume::NoDevice | Volume::Muted => {}
        }
    }
}
