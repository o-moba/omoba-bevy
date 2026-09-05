#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 ROW_DIRECTORY LOCOMOTION_PNG ACTIONS_PNG" >&2
  exit 2
fi

row_directory=$1
locomotion_png=$2
actions_png=$3

command -v magick >/dev/null || {
  echo "missing required command: magick" >&2
  exit 2
}

for state in idle run attack cast hit death; do
  row="$row_directory/$state-row.png"
  [[ -f "$row" ]] || {
    echo "missing row: $row" >&2
    exit 2
  }
  dimensions=$(magick identify -format '%wx%h' "$row")
  [[ "$dimensions" == "2048x256" ]] || {
    echo "$row must be 2048x256, got $dimensions" >&2
    exit 2
  }
done

mkdir -p "$(dirname "$locomotion_png")" "$(dirname "$actions_png")"
magick "$row_directory/idle-row.png" "$row_directory/run-row.png" \
  -append -define png:color-type=6 "$locomotion_png"
magick "$row_directory/attack-row.png" "$row_directory/cast-row.png" \
  "$row_directory/hit-row.png" "$row_directory/death-row.png" \
  -append -define png:color-type=6 "$actions_png"

locomotion_dimensions=$(magick identify -format '%wx%h' "$locomotion_png")
action_dimensions=$(magick identify -format '%wx%h' "$actions_png")
[[ "$locomotion_dimensions" == "2048x512" ]] || exit 1
[[ "$action_dimensions" == "2048x1024" ]] || exit 1
echo "wrote $locomotion_png ($locomotion_dimensions) and $actions_png ($action_dimensions)"
