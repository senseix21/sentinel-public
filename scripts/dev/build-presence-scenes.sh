#!/usr/bin/env bash
# Public fixtures only: bus.jpg and COCO 000000000077.jpg, supplied by the caller.
set -euo pipefail
[[ $# == 3 ]] || { echo 'usage: build-presence-scenes.sh FRONT_IMAGE COCO_77_IMAGE OUTPUT_MP4' >&2; exit 1; }
ffmpeg -v error -loop 1 -i "$1" -loop 1 -i "$2" \
  -f lavfi -i 'color=black:size=1280x720:rate=20' \
  -filter_complex '[0:v]scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:0:0,setsar=1,fps=20,trim=duration=8,setpts=PTS-STARTPTS,drawbox=x=iw-32:y=ih-32:w=32:h=32:color=red:t=fill[front];[1:v]crop=160:375:0:0,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:0:0,setsar=1,fps=20,trim=duration=8,setpts=PTS-STARTPTS,drawbox=x=iw-32:y=ih-32:w=32:h=32:color=lime:t=fill[back];[2:v]trim=duration=6,setpts=PTS-STARTPTS,drawbox=x=iw-32:y=ih-32:w=32:h=32:color=blue:t=fill[empty];[front][back][empty]concat=n=3:v=1:a=0[out]' \
  -map '[out]' -c:v libx264 -preset fast -profile:v baseline -g 20 -bf 0 \
  -pix_fmt yuv420p -an -y "$3"
