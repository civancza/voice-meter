//! Microphone capture and peak RMS exchange with the UI.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use cpal::platform::{CoreAudioDevice, CoreAudioHost, CoreAudioStream};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

const CHECK_INTERVAL: Duration = Duration::from_secs(1);
const STALL_TIMEOUT: Duration = Duration::from_secs(3);

/// Shared with one stream's callbacks; every connection gets a fresh one, so late
/// callbacks from a replaced stream can never leak into the new one.
#[derive(Default)]
struct Signal {
    level: AtomicU32,
    buffers: AtomicU32,
    failed: AtomicBool,
}

struct Health {
    last_buffer: Instant,
    receiving: bool,
}

impl Health {
    fn new(now: Instant) -> Self {
        Self {
            last_buffer: now,
            receiving: false,
        }
    }

    fn observe(&mut self, now: Instant, buffers: u32) {
        if buffers != 0 {
            self.last_buffer = now;
            self.receiving = true;
        }
    }

    fn stalled(&self, now: Instant) -> bool {
        now.duration_since(self.last_buffer) >= STALL_TIMEOUT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reconnect {
    InputChanged,
    StreamError,
    Stalled,
}

impl fmt::Display for Reconnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputChanged => "default input changed",
            Self::StreamError => "stream error",
            Self::Stalled => "callbacks stopped",
        })
    }
}

struct Live {
    _stream: CoreAudioStream,
    device: CoreAudioDevice,
    signal: Arc<Signal>,
    health: Health,
}

impl Live {
    fn connect(device: Option<CoreAudioDevice>, now: Instant) -> Result<Self, String> {
        let device = device.ok_or("no default microphone")?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("cannot read microphone configuration: {e}"))?;
        let signal = Arc::<Signal>::default();
        let stream = start(&device, &config, signal.clone())?;
        eprintln!(
            "voice-meter: microphone connected ({:?}, {} Hz, {} channels)",
            config.sample_format(),
            config.sample_rate().0,
            config.channels()
        );
        Ok(Self {
            _stream: stream,
            device,
            signal,
            health: Health::new(now),
        })
    }

    /// A silent buffer is healthy; a stream that stops delivering buffers is not.
    fn reconnect_reason(&self, now: Instant, default: Option<&CoreAudioDevice>) -> Option<Reconnect> {
        if default != Some(&self.device) {
            Some(Reconnect::InputChanged)
        } else if self.signal.failed.load(Ordering::Relaxed) {
            Some(Reconnect::StreamError)
        } else if self.health.stalled(now) {
            Some(Reconnect::Stalled)
        } else {
            None
        }
    }

    fn reading(&self) -> Option<f32> {
        let level = take_level(&self.signal.level);
        (self.health.receiving && !self.signal.failed.load(Ordering::Relaxed)).then_some(level)
    }
}

enum State {
    Disconnected { last_error: Option<String> },
    Live(Live),
}

/// Owns capture and checks the default input once per second, not per frame.
pub struct Capture {
    state: State,
    next_check: Instant,
}

impl Capture {
    pub fn new() -> Self {
        Self {
            state: State::Disconnected { last_error: None },
            next_check: Instant::now(),
        }
    }

    /// The loudest RMS since the last poll, or `None` while no microphone is delivering audio.
    pub fn poll(&mut self) -> Option<f32> {
        let now = Instant::now();
        if let State::Live(live) = &mut self.state {
            live.health.observe(now, live.signal.buffers.swap(0, Ordering::Relaxed));
        }
        if now >= self.next_check {
            self.next_check = now + CHECK_INTERVAL;
            self.refresh(now);
        }
        match &self.state {
            State::Live(live) => live.reading(),
            State::Disconnected { .. } => None,
        }
    }

    fn refresh(&mut self, now: Instant) {
        let default = CoreAudioHost.default_input_device();
        if let State::Live(live) = &self.state {
            let Some(reason) = live.reconnect_reason(now, default.as_ref()) else {
                return;
            };
            eprintln!("voice-meter: {reason}; reconnecting microphone");
        }
        // Drop the old stream before opening the new one.
        let last_error = match std::mem::replace(&mut self.state, State::Disconnected { last_error: None }) {
            State::Disconnected { last_error } => last_error,
            State::Live(_) => None,
        };
        self.state = match Live::connect(default, now) {
            Ok(live) => State::Live(live),
            Err(error) => {
                if last_error.as_ref() != Some(&error) {
                    eprintln!("voice-meter: {error}; retrying automatically");
                }
                State::Disconnected {
                    last_error: Some(error),
                }
            }
        };
    }
}

/// Takes the loudest RMS published since the previous read, resetting it to zero.
fn take_level(level: &AtomicU32) -> f32 {
    f32::from_bits(level.swap(0, Ordering::Relaxed))
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

fn build_stream<T: SizedSample>(
    device: &CoreAudioDevice,
    config: &cpal::StreamConfig,
    signal: Arc<Signal>,
) -> Result<CoreAudioStream, cpal::BuildStreamError>
where
    f32: FromSample<T>,
{
    let errors = signal.clone();
    device.build_input_stream(
        config,
        move |samples: &[T], _: &_| {
            let sum_sq = samples
                .iter()
                .map(|&sample| {
                    let value = f32::from_sample(sample);
                    value * value
                })
                .sum();
            publish(&signal.level, sum_sq, samples.len());
            if !samples.is_empty() {
                signal.buffers.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |e| {
            if !errors.failed.swap(true, Ordering::Relaxed) {
                eprintln!("voice-meter: audio error: {e}");
            }
        },
        None,
    )
}

fn start(
    device: &CoreAudioDevice,
    config: &cpal::SupportedStreamConfig,
    signal: Arc<Signal>,
) -> Result<CoreAudioStream, String> {
    let format = config.sample_format();
    let config = config.config();
    let stream = match format {
        cpal::SampleFormat::F32 => build_stream::<f32>(device, &config, signal),
        cpal::SampleFormat::I16 => build_stream::<i16>(device, &config, signal),
        cpal::SampleFormat::I32 => build_stream::<i32>(device, &config, signal),
        _ => return Err(format!("unsupported microphone sample format: {format}")),
    }
    .map_err(|e| format!("cannot open microphone: {e}"))?;
    stream.play().map_err(|e| format!("cannot start microphone: {e}"))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires microphone permission; run the test binary from a signed .app bundle"]
    fn live_capture_recovers_from_stopped_callbacks_errors_and_input_changes() {
        fn live(capture: &mut Capture) -> &mut Live {
            match &mut capture.state {
                State::Live(live) => live,
                State::Disconnected { last_error } => panic!("disconnected: {last_error:?}"),
            }
        }
        fn wait_for_capture(capture: &mut Capture) -> Arc<Signal> {
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                if let Some(level) = capture.poll() {
                    assert!(level.is_finite() && level >= 0.0);
                    return live(capture).signal.clone();
                }
                std::thread::sleep(Duration::from_millis(33));
            }
            panic!("no microphone callbacks within eight seconds");
        }
        fn replaced(capture: &Capture, previous: &Arc<Signal>) -> bool {
            matches!(&capture.state, State::Live(live) if !Arc::ptr_eq(previous, &live.signal))
        }

        let mut capture = Capture::new();
        let previous = wait_for_capture(&mut capture);
        live(&mut capture)._stream.pause().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !replaced(&capture, &previous) && Instant::now() < deadline {
            capture.poll();
            std::thread::sleep(Duration::from_millis(33));
        }
        assert!(replaced(&capture, &previous), "stalled stream was not replaced");

        let previous = wait_for_capture(&mut capture);
        previous.failed.store(true, Ordering::Relaxed);
        capture.next_check = Instant::now();
        assert_eq!(capture.poll(), None);
        assert!(replaced(&capture, &previous), "failed stream was not replaced");

        let previous = wait_for_capture(&mut capture);
        let other = CoreAudioHost
            .devices()
            .unwrap()
            .find(|device| device != &live(&mut capture).device)
            .expect("a second audio device");
        live(&mut capture).device = other;
        capture.next_check = Instant::now();
        assert_eq!(capture.poll(), None);
        assert!(replaced(&capture, &previous), "changed input was not reopened");
        wait_for_capture(&mut capture);
    }

    #[test]
    fn silent_buffers_keep_capture_healthy() {
        let start = Instant::now();
        let mut health = Health::new(start);
        let level = AtomicU32::new(0);
        for second in 1..=60 {
            let now = start + Duration::from_secs(second);
            publish(&level, 0.0, 128);
            health.observe(now, 1);
            assert_eq!(take_level(&level), 0.0);
            assert!(health.receiving);
            assert!(!health.stalled(now));
        }
    }

    #[test]
    fn missing_callbacks_stall_after_three_seconds() {
        let start = Instant::now();
        let mut health = Health::new(start);
        assert!(!health.receiving);
        assert!(!health.stalled(start + STALL_TIMEOUT / 2));
        assert!(health.stalled(start + STALL_TIMEOUT));
        health.observe(start + STALL_TIMEOUT, 1);
        assert!(!health.stalled(start + STALL_TIMEOUT));
        health.observe(start + STALL_TIMEOUT * 2, 0);
        assert!(health.stalled(start + STALL_TIMEOUT * 2));
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
    fn take_level_resets_peak() {
        let level = AtomicU32::new(0);
        publish(&level, 1.0, 4);
        assert_eq!(take_level(&level), 0.5);
        assert_eq!(take_level(&level), 0.0);
        publish(&level, 0.01, 1);
        assert!((take_level(&level) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn sample_conversion_preserves_normalization() {
        for sample in [i16::MIN, -1, 0, 1, i16::MAX] {
            assert_eq!(f32::from_sample(sample), sample as f32 / 32768.0);
        }
        for sample in [i32::MIN, -1, 0, 1, i32::MAX] {
            assert_eq!(f32::from_sample(sample), sample as f32 / 2147483648.0);
        }
        for sample in [-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
            assert_eq!(f32::from_sample(sample), sample);
        }
    }
}
