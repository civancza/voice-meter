//! Microphone capture and peak RMS exchange with the UI.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

/// Takes the loudest RMS published since the previous read, resetting it to zero.
pub fn take_level(level: &AtomicU32) -> f32 {
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
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    level: Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    f32: FromSample<T>,
{
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
            publish(&level, sum_sq, samples.len());
        },
        |e| eprintln!("voice-meter: audio error: {e}"),
        None,
    )
}

/// Starts the default microphone, or returns `None` if unavailable or unsupported.
/// Keep the returned stream alive for as long as capture is needed.
pub fn start(level: Arc<AtomicU32>) -> Option<cpal::Stream> {
    let device = cpal::default_host().default_input_device()?;
    let cfg = device.default_input_config().ok()?;
    let format = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let stream = match format {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, level),
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, level),
        cpal::SampleFormat::I32 => build_stream::<i32>(&device, &config, level),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

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
