---
workflow: general-video
flow: automation
storyboard: no
message: "Voice Meter tells you, gently then loudly, when you're talking too loud in meetings"
destination: github-readme
aspect: 1920x1080
language: en
length: 30s
angle: story
---

## Intent

A ~30s 16:9 demo for the Voice Meter GitHub repo. Story: a mock video call starts,
your voice rises, the meter goes GOOD → EASY… → TOO LOUD and jumps to the screen
center with an alarm, you calm down and it glides home. Then quick feature hits
(calibrate, presets, menu bar, <1% CPU, never records) and an end card with the
install command. "Super nice and super cool", "cool sounds and music".

## Assets

- ../../docs/screenshot.png — real HUD screenshot; reference for recreating the HUD 1:1 in HTML.
- ../../src/main.rs — exact colors, sizes, states and labels of the HUD.

## Customizations

- Music bed plus UI sound effects (whoosh on jump, alarm ping on TOO LOUD, soft tick on calm). No voiceover.

## Notes

- Origin story: the user speaks too loudly in meetings and his wife was freaking out; light humor welcome.
- Recreate the macOS HUD faithfully (dark glass, 20pt corners, cyan→mint→amber→red 28-segment meter, orb, white limit tick, dB readout, mood label).
