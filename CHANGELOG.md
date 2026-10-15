# Changelog

All notable changes to this project are documented here. Format: [Keep a Changelog](https://keepachangelog.com/),
versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Fixed
- Recover microphone capture automatically after default-input changes, audio errors, or stopped callbacks when
  switching meeting apps. Retry unavailable microphones and update `NO MIC` dynamically instead of staying frozen.
- Clear stale peaks, alarms, and incomplete calibration when microphone capture is lost.

## [0.2.0] - 2026-10-15

### Added
- 30-second demo video (HyperFrames) with synthesized music and SFX, README GIF and screenshots.

### Changed
- Separate the meter model, audio capture, preferences, HUD rendering/placement, and menu/controller wiring into modules.
- Unify amber segments with the Careful boundary at 6 dB below the threshold; clamp all thresholds to -45..=-3 dB.
- Use the TooLoud mood color for the alarm border, and persist HUD home position from window-move notifications
  instead of the animation tick.

### Fixed
- Repaint the complete HUD snapshot when shown, preventing stale orb, ring, label, and alarm colors after hidden
  mood transitions.

## [0.1.0] - 2026-10-15

### Added
- Floating glass HUD with orb, 28-segment meter, peak hold and dB readout.
- States QUIET / GOOD / EASY… / TOO LOUD with red pulse and tinted menu-bar icon.
- Strictness presets and 5-second voice calibration.
- Jump to center at 2.2× size when too loud (toggleable).
- Remembered, multi-monitor-aware HUD position.
