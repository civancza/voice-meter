#!/usr/bin/env bash
# Builds VoiceMeter.app (release, ad-hoc signed).
#
# Usage: scripts/bundle.sh [--install] [--zip] [--target <triple>]
#   --install  copy the app to /Applications
#   --zip      also produce dist/VoiceMeter-<version>-macos-<arch>.zip
#   --target   cross-compile, e.g. x86_64-apple-darwin (Intel) on Apple Silicon
set -euo pipefail
cd "$(dirname "$0")/.."

INSTALL=0 ZIP=0 TARGET=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --install) INSTALL=1 ;;
    --zip) ZIP=1 ;;
    --target) TARGET="${2:?--target needs a value}"; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [[ -n $TARGET ]]; then
  ARCH="${TARGET%%-*}"; [[ $ARCH == aarch64 ]] && ARCH=arm64
  cargo build --release --target "$TARGET"
  BIN="target/$TARGET/release/voice-meter"
else
  ARCH=$(uname -m)
  cargo build --release
  BIN="target/release/voice-meter"
fi

APP=VoiceMeter.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/VoiceMeter"
cp assets/VoiceMeter.icns "$APP/Contents/Resources/VoiceMeter.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Voice Meter</string>
  <key>CFBundleDisplayName</key><string>Voice Meter</string>
  <key>CFBundleIdentifier</key><string>com.civancza.voicemeter</string>
  <key>CFBundleExecutable</key><string>VoiceMeter</string>
  <key>CFBundleIconFile</key><string>VoiceMeter.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAudioCaptureUsageDescription</key><string>Voice Meter measures how loud the sound you hear is (nothing is recorded or stored) to help protect your ears.</string>
  <key>NSMicrophoneUsageDescription</key><string>Voice Meter listens to your microphone level (nothing is recorded or stored) to show how loudly you speak.</string>
</dict></plist>
PLIST
codesign --force --sign - "$APP" >/dev/null
echo "Built $APP ($VERSION, $ARCH)"

if [[ $ZIP == 1 ]]; then
  mkdir -p dist
  OUT="dist/VoiceMeter-${VERSION}-macos-${ARCH}.zip"
  rm -f "$OUT"
  ditto -c -k --keepParent "$APP" "$OUT"
  echo "Packaged $OUT"
fi

if [[ $INSTALL == 1 ]]; then
  rm -rf "/Applications/$APP"
  cp -R "$APP" /Applications/
  echo "Installed to /Applications/$APP"
fi
