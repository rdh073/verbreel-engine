#!/usr/bin/env python3
"""Operator-installed reference sidecar for the `face_mouth` op (issue #474).

This is a REFERENCE script, not crate-bundled model science. The
`verbreel-ai` crate spawns `python3 <this>` and speaks the one-shot JSON
sidecar protocol defined in `crates/verbreel-ai/src/sidecar.rs`:

    stdin  (one line) : {"op":"face_mouth","params":{...}}
    stdout (one line) : {"op":"face_mouth","result":{...}}
    then exit.

The crate's end-to-end test drives a STUB python3 child with a fixed
payload and does NOT import this file, so the test never needs mediapipe.
Operators install the heavy deps (mediapipe, opencv) to run this for real.

Mechanism (CPU-only, budget-aware):
  * Face detect + landmarks via MediaPipe Face Mesh (refined lips).
  * MAR = vertical_lip_gap / horizontal_lip_width per face per sampled
    frame, using inner-lip landmarks (vertical idx 13/14, corners 78/308).
  * Stable identity across frames via greedy IoU association -> `speaker_id`
    ("f0", "f1", ...) so each speaker's MAR series is continuous.

Wire contract (request -> result):
  request : {"op":"face_mouth","params":{"analysis_id":"<uuidv7>",
             "video_path":"assets/<aa>/<sha>.mp4","max_faces":4,
             "sample_fps":15}}
  result  : {"op":"face_mouth","result":{"faces":[
             {"speaker_id":"f0",
              "bbox_trace":[{"t_tk":0,"x":120,"y":80,"w":300,"h":300}],
              "mouth_open_trace":[{"t_tk":0,"mar":0.04}]}]}}

Determinism note: ML inference is the explicit non-deterministic seam. The
engine runs this ONCE and freezes the traces in the cache keyed by
`analysis_id`; render stays byte-deterministic. See issue #474.
"""

import json
import math
import sys

# Inner-lip landmark indices for MAR (MediaPipe Face Mesh, refined lips).
_LIP_TOP = 13
_LIP_BOTTOM = 14
_LIP_LEFT_CORNER = 78
_LIP_RIGHT_CORNER = 308

# Ticks-per-second for project time. The engine fixes the tick rate at
# 240,000 Hz (spec §0.2 / verbreel_types::TICK_RATE_HZ) — the LCM of common
# video framerates, so 24/25/29.97/30/50/59.94/60 all land on integer ticks.
# Every `*_tk` field in the engine is a tick at this rate; emitting any other
# rate would make the frozen #474 wire contract 240x off.
_TICKS_PER_SECOND = 240000


def _fail(detail):
    """Exit non-zero with a stderr detail so the crate maps to ProcessExited."""
    sys.stderr.write(detail + "\n")
    sys.exit(1)


def _iou(a, b):
    """Intersection-over-union of two (x, y, w, h) integer boxes."""
    ax0, ay0, aw, ah = a
    bx0, by0, bw, bh = b
    ax1, ay1 = ax0 + aw, ay0 + ah
    bx1, by1 = bx0 + bw, by0 + bh
    ix0, iy0 = max(ax0, bx0), max(ay0, by0)
    ix1, iy1 = min(ax1, bx1), min(ay1, by1)
    iw, ih = max(0, ix1 - ix0), max(0, iy1 - iy0)
    inter = iw * ih
    if inter == 0:
        return 0.0
    union = aw * ah + bw * bh - inter
    return inter / union if union > 0 else 0.0


def _mar(landmarks, width, height):
    """Mouth Aspect Ratio from inner-lip landmarks (normalized -> pixels)."""

    def px(idx):
        lm = landmarks[idx]
        return (lm.x * width, lm.y * height)

    tx, ty = px(_LIP_TOP)
    bx, by = px(_LIP_BOTTOM)
    lx, ly = px(_LIP_LEFT_CORNER)
    rx, ry = px(_LIP_RIGHT_CORNER)
    vertical = ((tx - bx) ** 2 + (ty - by) ** 2) ** 0.5
    horizontal = ((lx - rx) ** 2 + (ly - ry) ** 2) ** 0.5
    return vertical / horizontal if horizontal > 0 else 0.0


def _bbox(landmarks, width, height):
    """Pixel-integer (x, y, w, h) bounding box over all landmarks."""
    xs = [lm.x * width for lm in landmarks]
    ys = [lm.y * height for lm in landmarks]
    # MediaPipe normalized coords can fall slightly outside [0, 1] when a
    # landmark sits just past the frame edge, so the pixel values can be
    # negative. floor the min corner and ceil the max corner so the integer
    # box still fully covers the landmarks; int() would truncate toward zero
    # and pull a negative edge inward, shrinking the box (it feeds reframe).
    x0, y0 = math.floor(min(xs)), math.floor(min(ys))
    x1, y1 = math.ceil(max(xs)), math.ceil(max(ys))
    return (x0, y0, x1 - x0, y1 - y0)


def _analyze(video_path, max_faces, sample_fps):
    """Run MediaPipe Face Mesh over the clip and build per-face traces.

    Returns the `result` dict for the sidecar response. Raises ImportError
    if the operator has not installed the heavy deps (mediapipe, opencv).
    """
    import cv2  # type: ignore
    import mediapipe as mp  # type: ignore

    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RuntimeError(f"cannot open video: {video_path}")

    src_fps = cap.get(cv2.CAP_PROP_FPS) or float(sample_fps)
    stride = max(1, int(round(src_fps / max(1, sample_fps))))

    mesh = mp.solutions.face_mesh.FaceMesh(
        static_image_mode=False,
        max_num_faces=max_faces,
        refine_landmarks=True,
        min_detection_confidence=0.5,
    )

    # Stable identity: list of (speaker_id, last_bbox). Greedy IoU match.
    tracks = []
    faces = {}  # speaker_id -> {"bbox_trace": [...], "mouth_open_trace": [...]}
    next_id = 0
    frame_index = 0

    try:
        while True:
            ok, frame = cap.read()
            if not ok:
                break
            if frame_index % stride != 0:
                frame_index += 1
                continue

            height, width = frame.shape[:2]
            t_tk = int(round(frame_index / src_fps * _TICKS_PER_SECOND))
            rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            results = mesh.process(rgb)

            detections = []
            if results.multi_face_landmarks:
                for lms in results.multi_face_landmarks:
                    pts = lms.landmark
                    detections.append((_bbox(pts, width, height), _mar(pts, width, height)))

            # Greedy IoU association of detections to existing tracks.
            assigned = set()
            for det_bbox, det_mar in detections:
                best_iou, best_track = 0.0, None
                for ti, (sid, last_bbox) in enumerate(tracks):
                    if ti in assigned:
                        continue
                    score = _iou(det_bbox, last_bbox)
                    if score > best_iou:
                        best_iou, best_track = score, ti
                if best_track is not None and best_iou >= 0.3:
                    sid = tracks[best_track][0]
                    tracks[best_track] = (sid, det_bbox)
                    assigned.add(best_track)
                else:
                    sid = f"f{next_id}"
                    next_id += 1
                    tracks.append((sid, det_bbox))
                    assigned.add(len(tracks) - 1)
                    faces[sid] = {"bbox_trace": [], "mouth_open_trace": []}

                x, y, w, h = det_bbox
                faces[sid]["bbox_trace"].append(
                    {"t_tk": t_tk, "x": x, "y": y, "w": w, "h": h}
                )
                faces[sid]["mouth_open_trace"].append(
                    {"t_tk": t_tk, "mar": round(det_mar, 6)}
                )

            frame_index += 1
    finally:
        cap.release()
        mesh.close()

    ordered = [
        {
            "speaker_id": sid,
            "bbox_trace": faces[sid]["bbox_trace"],
            "mouth_open_trace": faces[sid]["mouth_open_trace"],
        }
        for sid in sorted(faces, key=lambda s: int(s[1:]))
    ]
    return {"faces": ordered}


def main():
    line = sys.stdin.readline()
    if not line.strip():
        _fail("face_mouth sidecar: empty request on stdin")

    try:
        request = json.loads(line)
    except json.JSONDecodeError as err:
        _fail(f"face_mouth sidecar: request JSON parse failed: {err}")

    if request.get("op") != "face_mouth":
        _fail(f"face_mouth sidecar: unexpected op {request.get('op')!r}")

    params = request.get("params", {})
    # `analysis_id` is intentionally unused here: caching is the engine's job
    # (it keys the frozen trace by analysis_id), the reference script just
    # computes the trace. Not reading it is not a wiring bug.
    video_path = params.get("video_path")
    if not video_path:
        _fail("face_mouth sidecar: missing params.video_path")
    max_faces = int(params.get("max_faces", 4))
    sample_fps = int(params.get("sample_fps", 15))

    try:
        result = _analyze(video_path, max_faces, sample_fps)
    except ImportError as err:
        _fail(
            "face_mouth sidecar: mediapipe/opencv not installed "
            f"(operator-install required): {err}"
        )
    except Exception as err:  # noqa: BLE001 — surface any analysis error loudly
        _fail(f"face_mouth sidecar: analysis failed: {err}")

    sys.stdout.write(json.dumps({"op": "face_mouth", "result": result}))
    sys.stdout.write("\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
