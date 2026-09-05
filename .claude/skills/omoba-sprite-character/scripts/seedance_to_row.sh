#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 SOURCE_VIDEO OUTPUT_ROW_PNG T0,T1,T2,T3,T4,T5,T6,T7" >&2
  exit 2
fi

source_video=$1
output_row=$2
timecodes_csv=$3
cell_size=256
margin=16
usable_size=$((cell_size - margin * 2))

for command_name in ffmpeg ffprobe magick; do
  command -v "$command_name" >/dev/null || {
    echo "missing required command: $command_name" >&2
    exit 2
  }
done

[[ -f "$source_video" ]] || {
  echo "source video does not exist: $source_video" >&2
  exit 2
}

IFS=',' read -r -a timecodes <<< "$timecodes_csv"
if [[ ${#timecodes[@]} -ne 8 ]]; then
  echo "exactly eight comma-separated timecodes are required" >&2
  exit 2
fi

for timecode in "${timecodes[@]}"; do
  [[ "$timecode" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
    echo "invalid timecode: $timecode" >&2
    exit 2
  }
done

output_dir=$(dirname "$output_row")
output_name=$(basename "$output_row" .png)
mkdir -p "$output_dir"
scratch_dir=$(mktemp -d "${TMPDIR:-/tmp}/omoba-seedance.XXXXXX")
trap 'rm -rf "$scratch_dir"' EXIT

duration=$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$source_video")
for index in "${!timecodes[@]}"; do
  timecode=${timecodes[$index]}
  awk -v selected="$timecode" -v total="$duration" 'BEGIN { exit !(selected >= 0 && selected <= total) }' || {
    echo "timecode $timecode exceeds clip duration $duration" >&2
    exit 2
  }
  raw_frame=$(printf '%s/raw-%02d.png' "$scratch_dir" "$index")
  clean_frame=$(printf '%s/clean-%02d.png' "$scratch_dir" "$index")
  ffmpeg -v error -ss "$timecode" -i "$source_video" -frames:v 1 -y "$raw_frame"
  magick "$raw_frame" -alpha on -fuzz 7% -transparent white "$clean_frame"
done

magick "$scratch_dir"/clean-*.png -evaluate-sequence Max "$scratch_dir/union.png"
# `%@` already reports the alpha/content bounding box relative to the original
# canvas. Running `-trim` first resets its offset to +0+0 and would make the
# subsequent common crop cut characters that are not anchored at the top-left.
trim_geometry=$(magick "$scratch_dir/union.png" -format '%@' info:)
if [[ -z "$trim_geometry" || "$trim_geometry" == "0x0+0+0" ]]; then
  echo "white-matte removal produced an empty sequence" >&2
  exit 1
fi

frame_paths=()
mapping_file="$output_dir/$output_name.frames.tsv"
printf 'runtime_frame\ttimecode_seconds\tsource_video\n' > "$mapping_file"
for index in "${!timecodes[@]}"; do
  normalized_frame=$(printf '%s/frame-%02d.png' "$scratch_dir" "$index")
  magick "$scratch_dir/clean-$(printf '%02d' "$index").png" \
    -crop "$trim_geometry" +repage \
    -filter point -resize "${usable_size}x${usable_size}" \
    -gravity south -background none -extent "${cell_size}x$((cell_size - margin))" \
    -gravity north -extent "${cell_size}x${cell_size}" \
    "$normalized_frame"
  alpha_mean=$(magick "$normalized_frame" -alpha extract -format '%[fx:mean]' info:)
  awk -v alpha="$alpha_mean" 'BEGIN { exit !(alpha > 0.0001) }' || {
    echo "normalized frame $index is empty after crop/matte processing" >&2
    exit 1
  }
  for edge_geometry in \
    "${cell_size}x2+0+0" \
    "${cell_size}x2+0+$((cell_size - 2))" \
    "2x${cell_size}+0+0" \
    "2x${cell_size}+$((cell_size - 2))+0"; do
    edge_alpha=$(magick "$normalized_frame" -alpha extract \
      -crop "$edge_geometry" +repage -format '%[fx:mean]' info:)
    awk -v alpha="$edge_alpha" 'BEGIN { exit !(alpha == 0) }' || {
      echo "normalized frame $index touches the two-pixel boundary ($edge_geometry)" >&2
      exit 1
    }
  done
  frame_paths+=("$normalized_frame")
  printf '%d\t%s\t%s\n' "$index" "${timecodes[$index]}" "$source_video" >> "$mapping_file"
done

magick "${frame_paths[@]}" +append -define png:color-type=6 "$output_row"
dimensions=$(magick identify -format '%wx%h' "$output_row")
if [[ "$dimensions" != "2048x256" ]]; then
  echo "unexpected output dimensions: $dimensions" >&2
  exit 1
fi

echo "wrote $output_row ($dimensions), mapping $mapping_file"
