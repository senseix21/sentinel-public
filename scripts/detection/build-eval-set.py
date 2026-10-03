#!/usr/bin/env python3
"""Build a detection evaluation set from COCO val2017 person annotations.

DI.R8 proves the adapter on one frame. One frame is an anecdote: it cannot
separate a detector that works from one that happened to work there, and it
says nothing about the rate at which people are missed. This builds a set with
real ground truth so that rate can be measured.

The output is not the Phase 6 accuracy gate. That gate waits on a dataset and
criteria agreed as representative of this product's cameras, and COCO is a
general-purpose benchmark of photographs, not surveillance footage. What this
produces is a measurement on a stated public subset, which is worth having
precisely because it is reproducible by someone who disagrees with it.

Images containing a crowd-flagged person region are excluded: COCO marks those
as a single unbounded blob, and a detector that correctly finds six people
inside one would be charged five false positives for it.
"""

import argparse
import json
import pathlib
import random
import subprocess
import sys
import urllib.request

PERSON_CATEGORY = 1
PAD_COLOUR = "0x727272"


def letterbox_filter(width, height, alignment):
    placement = "0:0" if alignment == "topleft" else "(ow-iw)/2:(oh-ih)/2"
    return (
        f"scale={width}:{height}:force_original_aspect_ratio=decrease,"
        f"pad={width}:{height}:{placement}:color={PAD_COLOUR}"
    )


def select_images(annotations, count, seed):
    people = {}
    crowded = set()
    for annotation in annotations["annotations"]:
        if annotation["category_id"] != PERSON_CATEGORY:
            continue
        if annotation.get("iscrowd", 0):
            crowded.add(annotation["image_id"])
            continue
        people.setdefault(annotation["image_id"], []).append(annotation["bbox"])
    eligible = sorted(set(people) - crowded)
    random.Random(seed).shuffle(eligible)
    chosen = eligible[:count]
    images = {image["id"]: image for image in annotations["images"]}
    return [(images[image_id], people[image_id]) for image_id in chosen]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--annotations", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--count", type=int, default=50)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--input-width", type=int, default=416)
    parser.add_argument("--input-height", type=int, default=416)
    parser.add_argument("--alignment", choices=("centre", "topleft"), default="centre")
    arguments = parser.parse_args()

    with arguments.annotations.open(encoding="utf-8") as handle:
        annotations = json.load(handle)

    selected = select_images(annotations, arguments.count, arguments.seed)
    frames_directory = arguments.output / "frames"
    frames_directory.mkdir(parents=True, exist_ok=True)
    jpegs = arguments.output.parent / "jpeg"
    jpegs.mkdir(parents=True, exist_ok=True)

    frames = []
    for index, (image, boxes) in enumerate(selected, start=1):
        jpeg = jpegs / image["file_name"]
        if not jpeg.exists():
            urllib.request.urlretrieve(image["coco_url"], jpeg)
        raw = frames_directory / f"{image['id']}.rgb"
        subprocess.run(
            [
                "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
                "-i", str(jpeg),
                "-vf", letterbox_filter(
                    arguments.input_width, arguments.input_height, arguments.alignment
                ),
                "-pix_fmt", "rgb24", "-f", "rawvideo", str(raw),
            ],
            check=True,
        )
        frames.append(
            {
                "path": f"frames/{raw.name}",
                "source_width": image["width"],
                "source_height": image["height"],
                "persons": [[float(value) for value in box] for box in boxes],
            }
        )
        print(f"  {index}/{len(selected)} {image['file_name']} {len(boxes)} person(s)",
              file=sys.stderr)

    manifest = {
        "source": "COCO val2017, person category, crowd-flagged images excluded",
        "selection": f"seed {arguments.seed}, first {arguments.count} after shuffle",
        "alignment": arguments.alignment,
        "input_width": arguments.input_width,
        "input_height": arguments.input_height,
        "frames": frames,
    }
    destination = arguments.output / "manifest.json"
    with destination.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
    people = sum(len(frame["persons"]) for frame in frames)
    print(f"{len(frames)} frames, {people} people -> {destination}", file=sys.stderr)


if __name__ == "__main__":
    main()
