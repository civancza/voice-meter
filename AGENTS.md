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

Always run `make check` before declaring work done. Unit tests live in `#[cfg(test)] mod tests` at the bottom of `src/main.rs`. Logic that can be tested without
AppKit (e.g. `db_frac`, `publish`, calibration math) should get tests when changed.

Debugging: run the binary directly to see panics/ObjC exceptions on stderr:
`./VoiceMeter.app/Contents/MacOS/VoiceMeter`. Crash reports land in `~/Library/Logs/DiagnosticReports/VoiceMeter-*.ips`.
Microphone access only works from the `.app` bundle (it carries `NSMicrophoneUsageDescription`).

## Layout

```
src/main.rs          the whole app (UI, audio, state)
scripts/bundle.sh    builds/signs VoiceMeter.app, Info.plist lives here
Makefile             developer shortcuts
docs/                screenshots, demo.mp4/demo.gif (rendered from videos/)
videos/voice-meter-demo/  HyperFrames demo video (index.html + scripts/mix.sh soundtrack); see its AGENTS.md
.github/             CI, release workflow, templates
```

If you change the HUD look (colors, sizes, labels), mirror it in `videos/voice-meter-demo/index.html` and re-render.

## Architecture (src/main.rs)

- **Audio thread** (`start_audio`, `publish`): cpal callback computes RMS per buffer and stores the max since last read
  in an `AtomicU32` (`fetch_max` on f32 bits, valid because values are non-negative). No locks, no allocation.
- **Main thread** (`Controller`, an `NSObject` subclass via `define_class!`): an `NSTimer` at `FPS` (30) calls `on_tick`:
  1. swap level → dB, attack/release smoothing, peak hold;
  2. alarm logic (`loud_ticks` ≥ 5 ⇒ `alarm_ticks` = 1.6 s);
  3. calibration (collect 5 s, p90 + 6 dB, clamped);
  4. pick `Mood`, update only CALayer properties that changed, inside a `CATransaction` with actions disabled;
  5. `update_zoom`: eases `State.zoom` 0→1, moves/resizes the `NSPanel` toward screen center and scales the `stage`
     layer by up to `ZOOM_SCALE`.
- **View tree**: `NSPanel` (borderless, non-activating, level 25, all Spaces) → `NSVisualEffectView` (blur, shaped by
  `pill_mask` image) → `canvas` NSView → `stage` CALayer (fixed W×H, scaled for zoom) → segment/orb/text layers.
- **Persistence**: `NSUserDefaults` keys `thresholdDb`, `jumpWhenLoud`, `hudX`, `hudY` (domain `com.civancza.voicemeter`).

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
- Prefer adding new visual constants next to `W`, `H`, `CORNER`, `ZOOM_SCALE` rather than inline magic numbers.
- Update `README.md` (features/usage) and `CHANGELOG.md` (`Unreleased`) for user-visible changes.

## Definition of done

1. `make check` passes.
2. `make run` launches; HUD renders, meter reacts to speech, menu items work.
3. For UI changes, include a screenshot in the PR.
4. CPU/memory not noticeably higher than before.
