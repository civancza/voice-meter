# Contributing to Voice Meter

Thanks for helping people talk at a reasonable volume. 🙂

## Setup

1. macOS 11+, Xcode Command Line Tools (`xcode-select --install`), Rust via [rustup](https://rustup.rs).
   The pinned toolchain in `rust-toolchain.toml` installs `rustfmt` and `clippy` automatically.
2. `make run` builds `VoiceMeter.app` and launches it. Grant microphone access on first run.

## Workflow

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach.
2. Create a branch: `feat/<short-name>`, `fix/<short-name>` or `docs/<short-name>`.
3. Keep changes focused. One PR = one concern.
4. Run `make check` (fmt, clippy with `-D warnings`, tests, release build). CI runs the same.
5. Manually verify with `make run`. For visual changes attach a screenshot or short screen recording to the PR.
6. Add a line to `CHANGELOG.md` under **Unreleased** for user-visible changes.

### Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/): `feat: jump to center when too loud`,
`fix: transparent HUD corners`, `docs: …`, `refactor: …`, `chore: …`.

## Design principles

- **Tiny footprint.** No per-frame allocations; update only changed layer properties; no new heavy dependencies
  (webviews, async runtimes, GPU frameworks). Mention CPU/RSS impact in the PR for anything in the render loop.
- **Native.** AppKit, Core Animation and Core Audio through `objc2`/`cpal`.
- **Private.** Audio is measured, never recorded, stored or transmitted. PRs that change this will not be accepted.
- **Calm UX.** Clear and glanceable, alarming only when it should be.

## Using AI coding agents

Agents are welcome. [AGENTS.md](AGENTS.md) contains the build commands, architecture, gotchas and definition of done.
Please review agent-generated code like your own, and keep `AGENTS.md` up to date when you learn something an agent
(or a new contributor) would need.

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml`, move **Unreleased** notes in `CHANGELOG.md` under the new version.
2. Commit, then tag: `git tag v0.2.0 && git push --tags`.
3. The `release` workflow builds arm64 and x86_64 zips and attaches them to a GitHub Release.

## Code of conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
