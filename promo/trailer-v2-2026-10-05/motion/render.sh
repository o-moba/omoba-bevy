#!/bin/zsh
# Renders out/omoba-trailer-v2.mp4 (or the path given): 1920x1080, 60 fps, H.264 + AAC.
set -e
cd "$(dirname "$0")"
node render.mjs "$@"
