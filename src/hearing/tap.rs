//! System-wide output capture with a Core Audio process tap (macOS 14.2+).
//!
//! A private, unmuted, global stereo tap is wrapped in a private aggregate device whose IOProc
//! publishes the loudest RMS per UI frame, exactly like the microphone callback.

use crate::audio::Exchange;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice, AudioObjectID, CATapDescription,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioSubTapUIDKey,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::Arc;

type OSStatus = i32;
type CreateProcessTap = unsafe extern "C-unwind" fn(*const CATapDescription, *mut AudioObjectID) -> OSStatus;
type DestroyProcessTap = unsafe extern "C-unwind" fn(AudioObjectID) -> OSStatus;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_DEFAULT: *mut c_void = -2_isize as *mut c_void;

/// Field order matters: the device stops (in `Drop`) before `exchange` is released.
pub struct Tap {
    tap: AudioObjectID,
    device: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    destroy_tap: DestroyProcessTap,
    exchange: Arc<Exchange>,
}

impl Tap {
    pub fn start() -> Result<Self, String> {
        // Resolved at runtime so the app still launches on macOS versions without process taps.
        let (create_tap, destroy_tap) = symbols().ok_or("hearing level needs macOS 14.2 or later")?;
        let mut this = Self {
            tap: 0,
            device: 0,
            io_proc: None,
            destroy_tap,
            exchange: Arc::default(),
        };
        // SAFETY: plain Objective-C object creation and configuration on a fresh description.
        let description = unsafe {
            let d =
                CATapDescription::initStereoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &NSArray::new());
            d.setPrivate(true);
            d.setName(&NSString::from_str("Voice Meter"));
            d
        };
        // SAFETY: the description is a live object and `tap` a valid out-pointer.
        check("create system audio tap", unsafe {
            create_tap(Retained::as_ptr(&description), &mut this.tap)
        })?;

        // SAFETY: `UUID` is always set; its string is the tap's UID used by aggregate devices.
        let tap_uid = unsafe { description.UUID().UUIDString() };
        let device_uid = NSString::from_str(&format!("com.civancza.voicemeter.tap.{tap_uid}"));
        let sub_tap = dictionary(&[(kAudioSubTapUIDKey, tap_uid.as_ref())]);
        let taps = NSArray::from_slice(&[&*sub_tap]);
        let yes = NSNumber::new_bool(true);
        let config = dictionary(&[
            (kAudioAggregateDeviceUIDKey, device_uid.as_ref()),
            (
                kAudioAggregateDeviceNameKey,
                NSString::from_str("Voice Meter Hearing").as_ref(),
            ),
            (kAudioAggregateDeviceIsPrivateKey, yes.as_ref()),
            (kAudioAggregateDeviceTapAutoStartKey, yes.as_ref()),
            (kAudioAggregateDeviceTapListKey, taps.as_ref()),
        ]);
        // SAFETY: NSDictionary is toll-free bridged to CFDictionary; out-pointers are valid; the
        // client data points into `exchange`, which outlives the IOProc (see `Drop`).
        unsafe {
            let config = &*(Retained::as_ptr(&config) as *const CFDictionary);
            check(
                "create aggregate device",
                AudioHardwareCreateAggregateDevice(config, NonNull::from(&mut this.device)),
            )?;
            check(
                "register audio callback",
                AudioDeviceCreateIOProcID(
                    this.device,
                    Some(io_proc),
                    Arc::as_ptr(&this.exchange).cast_mut().cast(),
                    NonNull::from(&mut this.io_proc),
                ),
            )?;
            check("start capture", AudioDeviceStart(this.device, this.io_proc))?;
        }
        Ok(this)
    }

    /// A running tap delivers (silent) buffers continuously once permission is granted.
    pub fn exchange(&self) -> &Exchange {
        &self.exchange
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // SAFETY: each id was created by `start` (zero/None when that step never happened);
        // stopping and destroying the IOProc waits for in-flight callbacks before `exchange` drops.
        unsafe {
            if self.io_proc.is_some() {
                AudioDeviceStop(self.device, self.io_proc);
                AudioDeviceDestroyIOProcID(self.device, self.io_proc);
            }
            if self.device != 0 {
                AudioHardwareDestroyAggregateDevice(self.device);
            }
            if self.tap != 0 {
                (self.destroy_tap)(self.tap);
            }
        }
    }
}

/// Runs on Core Audio's real-time thread: no locks, no allocation. Tap streams are Float32.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> OSStatus {
    // SAFETY: `client` is the `Exchange` registered in `start`, alive until the IOProc is destroyed;
    // Core Audio passes a valid buffer list with `mNumberBuffers` trailing buffers.
    unsafe {
        let exchange = &*client.cast::<Exchange>();
        let list = input.as_ptr();
        let buffers: &[AudioBuffer] =
            std::slice::from_raw_parts((&raw const (*list).mBuffers).cast(), (*list).mNumberBuffers as usize);
        let (mut sum_sq, mut n) = (0.0_f32, 0_usize);
        for buffer in buffers {
            if buffer.mData.is_null() {
                continue;
            }
            let samples = std::slice::from_raw_parts(
                buffer.mData.cast::<f32>(),
                buffer.mDataByteSize as usize / size_of::<f32>(),
            );
            sum_sq += samples.iter().map(|v| v * v).sum::<f32>();
            n += samples.len();
        }
        exchange.record(sum_sq, n);
    }
    0
}

pub fn supported() -> bool {
    symbols().is_some()
}

fn symbols() -> Option<(CreateProcessTap, DestroyProcessTap)> {
    // SAFETY: CoreAudio is already loaded (linked via cpal); the symbols, when present, have
    // exactly these C signatures.
    unsafe {
        let create = dlsym(RTLD_DEFAULT, c"AudioHardwareCreateProcessTap".as_ptr());
        let destroy = dlsym(RTLD_DEFAULT, c"AudioHardwareDestroyProcessTap".as_ptr());
        (!create.is_null() && !destroy.is_null()).then(|| {
            (
                std::mem::transmute::<*mut c_void, CreateProcessTap>(create),
                std::mem::transmute::<*mut c_void, DestroyProcessTap>(destroy),
            )
        })
    }
}

fn dictionary(entries: &[(&CStr, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<_> = entries
        .iter()
        .map(|(key, _)| NSString::from_str(key.to_str().unwrap_or_default()))
        .collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&AnyObject> = entries.iter().map(|&(_, value)| value).collect();
    NSDictionary::from_slices(&key_refs, &values)
}

fn check(what: &str, status: OSStatus) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("cannot {what} (OSStatus {status})"))
    }
}
