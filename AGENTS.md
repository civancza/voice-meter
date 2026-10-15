# AGENTS.md

Guidance for AI coding agents (Copilot, Claude Code, Codex, Cursor, …) and humans working on this repo.
`CLAUDE.md` and `.github/copilot-instructions.md` point here. Keep this file as the single source of truth.

## Project in one paragraph

Voice Meter is a single-binary native macOS menu-bar app written in Rust. It reads the default microphone level with
`cpal` (Core Audio) and renders a floating glass HUD with AppKit + Core Animation through the `objc2` bindings. Hard
constraints: **minimal CPU and memory**, **native look**, **no webviews / Electron / GPU frameworks / async runtimes**.

## Commands

| Task | Command |
|---|---|
| Build + launch the app | `make run` |
| Build `VoiceMeter.app` only | `make app` (= `scripts/bundle.sh`) |
| Everything CI checks | `make check` (fmt check, clippy `-D warnings`, tests, release build) |
| Unit tests | `make test` |
| Format | `make fmt` |
| Release zip | `make zip` → `dist/VoiceMeter-<ver>-macos-<arch>.zip` |
| Regenerate app icon | `make icon` (AppKit/Swift + iconutil; no third-party tools) |

Always run `make check` before declaring work done. Pure model tests live in `src/meter/tests.rs` and
`src/meter/presentation_tests.rs`; audio tests are in `src/audio.rs`, and render-cache regressions in `src/hud/tests.rs`.
Logic that can be tested without AppKit (e.g. `db_frac`, `Exchange::record`, calibration math) should get tests when changed.

The ignored `audio::tests::live_capture_recovers_from_stopped_callbacks_errors_and_input_changes` test needs a real
microphone and bundle permission. Copy the test executable reported by `cargo test --release --no-run` into a temporary
copy of `VoiceMeter.app` as `Contents/MacOS/VoiceMeter`, ad-hoc sign that bundle, and run its executable with
`--ignored --exact audio::tests::live_capture_recovers_from_stopped_callbacks_errors_and_input_changes --nocapture`.
It pauses only its own capture stream and simulates error/input-change signals; it does not change the system input.

Debugging: run the binary directly to see panics/ObjC exceptions on stderr:
`./VoiceMeter.app/Contents/MacOS/VoiceMeter`. Crash reports land in `~/Library/Logs/DiagnosticReports/VoiceMeter-*.ips`.
Microphone access only works from the `.app` bundle (it carries `NSMicrophoneUsageDescription`).

## Layout

```
src/main.rs          Controller actions, NSWindowDelegate, wiring, and app entry point
src/meter.rs         pure Meter model and complete Frame presentation snapshots
src/meter/           model and presentation tests
src/audio.rs         recoverable cpal capture, callback health, atomic peak-RMS exchange (plus tests)
src/hearing.rs       pure Ear model for the 👂 row: smoothing, mood, label (plus tests)
src/hearing/monitor.rs  tap lifecycle state machine + default output mute/volume dB
src/hearing/tap.rs   Core Audio process tap + private aggregate device, real-time IOProc
src/prefs.rs         typed NSUserDefaults settings access
src/hud.rs           HUD/layer construction, snapshot rendering, one render cache
src/hud/placement.rs window placement/zoom animation, pill mask, screen helpers
src/hud/tests.rs     full-snapshot repaint/invalidation regression tests
src/menu.rs          status item, shared menus, refresh, and icon tint
scripts/bundle.sh    builds/signs VoiceMeter.app, Info.plist lives here
scripts/generate-icon.swift  native AppKit artwork generator (build-time only)
assets/              checked-in app icon (ICNS + 1024px PNG)
Makefile             developer shortcuts
docs/                screenshots, demo.mp4/demo.gif (rendered from videos/)
videos/voice-meter-demo/  HyperFrames demo video (index.html + scripts/mix.sh soundtrack); see its AGENTS.md
.github/             CI, release workflow, templates
```

If you change the HUD look (colors, sizes, labels), mirror it in `videos/voice-meter-demo/index.html` and re-render.

## Architecture

- **Audio capture** (`audio::Capture`, `audio::Exchange`): cpal callback computes RMS per buffer and `Exchange::record`s
  the max since last read in an `AtomicU32` (`fetch_max` on f32 bits, valid because values are non-negative). No locks,
  no allocation. The same `Exchange` counts non-empty callbacks, including silence; `audio::Health` turns that into
  receiving/stalled state. Both are shared with the hearing tap. `Capture` is a `Disconnected | Live` state machine:
  `poll` returns `Option<f32>` (`None` = no mic) and once per second compares the default input device ID (cheap; the
  stream format is read only when connecting) and rebuilds capture on a device change, stream error, or three seconds
  without callbacks. Each `Live` connection owns its own atomic exchange, so late callbacks cannot contaminate a
  replacement. Failed starts retry once per second with deduplicated diagnostics.
- **Main-thread controller** (`Controller`, an `NSObject` subclass via `define_class!`): an `NSTimer` at `meter::FPS`
  (30 Hz) maps the capture reading through `meter::rms_db` and calls `Meter::step(Option<f32>)`; a `None`/`Some` transition
  shows `NO MIC`, clears stale peaks/alarms and (on loss) cancels incomplete calibration. It passes the complete
  frame to the HUD/menu and advances placement separately. Calibration starts through `Meter::start_calibration`;
  a finishing frame's `calibration_result` is persisted through `Prefs`, recolors the segments/marker, and refreshes menus.
- **Model** (`meter::Meter`): owns smoothing, peak hold, alarm debounce/hold, calibration, mood, labels and refresh
  cadence. `Frame` includes every visual value, so rendering needs no signal/mood/calibration decisions. All threshold
  sources clamp to -45..=-3 dB; amber starts at threshold minus 6 dB, matching Careful; alarm borders use TooLoud RGB.
- **Rendering** (`hud::Hud`): owns layers and one `Rendered` last-drawn snapshot. Only changed properties are applied,
  inside a `CATransaction` with actions disabled. Text buffers in the cache are reused. Changing thresholds or calling
  `Hud::show` resets the entire cache. All show paths (initial display, toggle, calibration) use `Hud::show`; on the next
  visible tick every property is repainted from the complete frame, even if moods/alarms changed while hidden and the
  model's incremental flags are now false. The model and menu-bar tint continue advancing while the HUD is hidden.
- **Placement** (`hud::Placement`): owns jump preference, home origin, and zoom progress independently of the model/cache.
  Zoom eases 0→1, moves/resizes the panel toward screen center, and scales the stage by up to `ZOOM_SCALE` using
  `CATransform3DMakeScale`. The controller's `windowDidMove:` delegate persists changed home origins only at rest;
  programmatic animation moves and moves while zoomed are ignored. No preference writes occur inside `update_zoom`.
  Connected-display checks use `CGRectContainsPoint`.
- **View tree**: `NSPanel` (borderless, non-activating, level 25, all Spaces) → `NSVisualEffectView` (blur, shaped by
  `pill_mask` image) → stored `canvas` NSView → `stage` CALayer (fixed W×H, scaled for zoom) → segment/orb/text layers.
- **Hearing level** (`hearing::Monitor`, `hearing::tap::Tap`, `hearing::Ear`): a private global stereo process tap in a
  private aggregate device; its IOProc records peak RMS into an `audio::Exchange`, exactly like the mic callback.
  Tap functions are resolved with `dlsym` so the app still launches on macOS < 14.2 (`Unsupported`, shows `—`).
  `Monitor` runs an `Idle | Starting | Running { tap, health } | Unsupported` state machine (starting happens on a helper
  thread because it blocks while the permission prompt is open) and adds the default output's volume in dB (mute →
  `MUTED`; read every 500 ms; main element, else mean of channels 1–2, else 0 dB). A tap that has delivered and then
  stalls for 3 s is restarted. `Ear` is the pure model (`meter::ease` attack/release, label every `DB_TEXT_EVERY_TICKS`,
  amber ≥ `CAREFUL_DB`, red ≥ `LOUD_DB`). The HUD's 20 pt bottom 👂 row (`EAR_H`; the main row is `ROW_H`) has its own
  `HearingSnapshot` cache (bar width in whole points, mood, label) and repaints only on change.
  Requires `NSAudioCaptureUsageDescription` (in `scripts/bundle.sh`).
- **Menus** (`menu::Menu`): owns the status item and shared status/HUD context menu, updates checks/titles, and applies
  the model's status tint independently of HUD visibility.
- **Persistence** (`prefs::Prefs`): wraps `NSUserDefaults` keys `thresholdDb`, `jumpWhenLoud`, `hudX`, `hudY`
  (domain `com.civancza.voicemeter`); no other module accesses defaults directly.

## Rules and gotchas

- **Performance is a feature.** Do not add per-frame allocations, string formatting every frame, or redraws of unchanged
  layers. Keep the timer at ≤ 30 fps. Check CPU with `ps -o rss=,%cpu= -p $(pgrep -f VoiceMeter.app/Contents/MacOS)`;
  it should stay around 1 % or below.
- All AppKit/CoreAnimation calls must happen on the main thread (`MainThreadMarker`). Never touch UI from the audio
  callback.
- `NSVisualEffectView` ignores `cornerRadius` for the blur. Rounded corners come from `setMaskImage(pill_mask(r))`.
- `objc2-app-kit` 0.3 has `NSImageResizingMode::Stretch`/`Tile` swapped on Apple Silicon. `pill_mask` hard-codes
  AppKit's real values; AppKit throws if a mask image is not in stretch mode.
- `kill` by PID only (no `pkill`/`killall`) when scripting app restarts.
- Prefer adding new visual constants next to `W`, `H`, `CORNER`, `ZOOM_SCALE` in `src/hud.rs` rather than inline magic numbers.
- Update `README.md` (features/usage) and `CHANGELOG.md` (`Unreleased`) for user-visible changes.

## Definition of done

1. `make check` passes.
2. `make run` launches; HUD renders, meter reacts to speech, menu items work.
3. For UI changes, include a screenshot in the PR.
4. CPU/memory not noticeably higher than before.
