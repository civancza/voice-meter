#!/usr/bin/env bash
# Mix synthesized music/voice bed + bundled SFX cues into assets/audio/soundtrack.mp3
set -euo pipefail
cd "$(dirname "$0")/.."
node scripts/make-audio.mjs

# cue: "<file> <start-seconds> <volume>"
CUES=(
  "pop 0.40 0.30" "pop 0.70 0.25"
  "notification 6.30 0.45"
  "whoosh-short 7.70 0.55" "impact-bass-1 8.00 0.75" "error 8.15 0.50" "ping 9.40 0.35"
  "whoosh-short 11.65 0.45" "chime 12.00 0.45"
  "whoosh 15.85 0.55" "impact-bass-1 16.15 0.70"
  "pop 18.00 0.30" "pop 18.55 0.30" "pop 19.10 0.30" "pop 19.65 0.30" "pop 20.20 0.30"
  "whoosh-short 23.90 0.45" "typing 24.60 0.35" "sparkle 26.60 0.40"
)

inputs=(-i assets/audio/music.wav)
filters=""
labels="[0:a]"
n=1
for cue in "${CUES[@]}"; do
  read -r name start vol <<<"$cue"
  inputs+=(-i "assets/sfx/$name.mp3")
  ms=$(awk "BEGIN{printf \"%d\", $start*1000}")
  filters+="[$n:a]aresample=44100,aformat=channel_layouts=stereo,volume=$vol,adelay=$ms|$ms[s$n];"
  labels+="[s$n]"
  n=$((n + 1))
done
filters+="${labels}amix=inputs=$n:normalize=0:duration=first,alimiter=limit=0.95,atrim=0:30[out]"

ffmpeg -y -loglevel error "${inputs[@]}" -filter_complex "$filters" -map "[out]" \
  -c:a libmp3lame -b:a 192k assets/audio/soundtrack.mp3
rm -f assets/audio/music.wav
echo "wrote assets/audio/soundtrack.mp3"
