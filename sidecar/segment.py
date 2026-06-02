#!/usr/bin/env python3
"""Operator-installed reference sidecar for the `segment` op (issue #476).

This is a REFERENCE script, not crate-bundled model science. The
`verbreel-ai` crate spawns `python3 <this>` and speaks the one-shot JSON
sidecar protocol defined in `crates/verbreel-ai/src/sidecar.rs`:

    stdin  (one line) : {"op":"segment","params":{...}}
    stdout (one line) : {"op":"segment","result":{...}}
    then exit.

The crate's end-to-end test drives a STUB python3 child with a fixed
payload and does NOT import this file, so the test never needs a real
segmentation model. Operators install the heavy deps (one of mediapipe /
robust-video-matting / modnet / birefnet, plus opencv + numpy) to run this
for real.

Mechanism (CPU-or-GPU, operator's choice — green-screen-free):
  * Per frame, run a foreground-matting model to get a single-channel alpha
    matte (subject = opaque, background = transparent). No keyed color —
    works on arbitrary/cluttered backgrounds, unlike chroma_key.
  * Stack the per-frame mattes into one grayscale PNG strip (frames tiled
    vertically) and import it as a single content-addressed `image`-kind
    asset under assets/<aa>/<sha256>.png. The strip IS the matte asset that
    `Clip.mask` kind `asset` references; the engine computes its sha256 on
    import (this script writes the bytes and reports the hash).
  * Emit the matte reference: CAS hash + matte dimensions + per-frame
    project-time ticks (240,000 Hz), in frame order.

Wire contract (request -> result):
  request : {"op":"segment","params":{"analysis_id":"<uuidv7>",
             "video_path":"assets/<aa>/<sha>.mp4"}}
  result  : {"op":"segment","result":{
             "matte_asset_hash":"<sha256-hex>","width":1920,"height":1080,
             "frames":[{"t_tk":0},{"t_tk":8000}]}}

Determinism note: ML inference is the explicit non-deterministic seam. The
engine runs this ONCE and freezes the matte as the CAS-keyed alpha asset
(cache/ keyed by `analysis_id`); render stays byte-deterministic against the
frozen matte — never per-render inference. See issue #476.
"""

import hashlib
import json
import os
import sys

# Ticks-per-second for project time. The engine fixes the tick rate at
# 240,000 Hz (spec §0.2 / verbreel_types::TICK_RATE_HZ) — the LCM of common
# video framerates, so 24/25/29.97/30/50/59.94/60 all land on integer ticks.
# Every `*_tk` field in the engine is a tick at this rate; emitting any other
# rate would make the frozen #476 wire contract 240x off.
_TICKS_PER_SECOND = 240000


def _fail(detail):
    """Exit non-zero with a stderr detail so the crate maps to ProcessExited."""
    sys.stderr.write(detail + "\n")
    sys.exit(1)


def _matte_frame(model, frame_rgb):
    """Single-channel uint8 alpha matte for one frame (subject = 255).

    The operator wires whichever matting model they installed; the contract
    is a HxW uint8 array. Raises if the model surfaces no alpha (loud, no
    silent all-zero matte).
    """
    alpha = model.matte(frame_rgb)  # operator-supplied: HxW float in [0, 1]
    if alpha is None:
        raise RuntimeError("matting model returned no alpha for a frame")
    import numpy as np  # type: ignore

    return (np.clip(alpha, 0.0, 1.0) * 255.0).round().astype(np.uint8)


def _analyze(video_path):
    """Run the matting model over the clip and freeze the matte strip.

    Returns the `result` dict for the sidecar response. Raises ImportError
    if the operator has not installed the heavy deps.
    """
    import cv2  # type: ignore
    import numpy as np  # type: ignore

    # Operator-installed matting backend. The reference loader picks the
    # first installed of MediaPipe Selfie / RVM / MODNet / BiRefNet; if none
    # is present this raises ImportError, surfaced loudly below.
    from matting_backend import load_matting_model  # type: ignore

    model = load_matting_model()

    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RuntimeError(f"cannot open video: {video_path}")

    src_fps = cap.get(cv2.CAP_PROP_FPS) or 30.0

    mattes = []
    frames = []
    width = height = 0
    frame_index = 0
    try:
        while True:
            ok, frame = cap.read()
            if not ok:
                break
            height, width = frame.shape[:2]
            rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            mattes.append(_matte_frame(model, rgb))
            t_tk = int(round(frame_index / src_fps * _TICKS_PER_SECOND))
            frames.append({"t_tk": t_tk})
            frame_index += 1
    finally:
        cap.release()

    if not mattes:
        raise RuntimeError(f"no frames decoded from {video_path}")

    # Tile per-frame mattes vertically into one grayscale strip — the single
    # `image`-kind asset that Clip.mask kind `asset` references.
    strip = np.concatenate(mattes, axis=0)
    ok, buf = cv2.imencode(".png", strip)
    if not ok:
        raise RuntimeError("matte PNG encode failed")
    png_bytes = buf.tobytes()

    # Content-addressed: the engine keys the asset by sha256 of these bytes.
    sha = hashlib.sha256(png_bytes).hexdigest()
    aa = sha[:2]
    out_dir = os.path.join("assets", aa)
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, f"{sha}.png")
    # Write once; if it already exists the bytes are identical by construction.
    if not os.path.exists(out_path):
        with open(out_path, "wb") as fh:
            fh.write(png_bytes)

    return {
        "matte_asset_hash": sha,
        "width": int(width),
        "height": int(height),
        "frames": frames,
    }


def main():
    line = sys.stdin.readline()
    if not line.strip():
        _fail("segment sidecar: empty request on stdin")

    try:
        request = json.loads(line)
    except json.JSONDecodeError as err:
        _fail(f"segment sidecar: request JSON parse failed: {err}")

    if request.get("op") != "segment":
        _fail(f"segment sidecar: unexpected op {request.get('op')!r}")

    params = request.get("params", {})
    # `analysis_id` is intentionally unused here: caching is the engine's job
    # (it keys the frozen matte by analysis_id), the reference script just
    # computes the matte. Not reading it is not a wiring bug.
    video_path = params.get("video_path")
    if not video_path:
        _fail("segment sidecar: missing params.video_path")

    try:
        result = _analyze(video_path)
    except ImportError as err:
        _fail(
            "segment sidecar: matting model / opencv / numpy not installed "
            f"(operator-install required): {err}"
        )
    except Exception as err:  # noqa: BLE001 — surface any analysis error loudly
        _fail(f"segment sidecar: analysis failed: {err}")

    sys.stdout.write(json.dumps({"op": "segment", "result": result}))
    sys.stdout.write("\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
