#!/usr/bin/env python3
"""Operator-installed reference sidecar for the `gen_image` op (issue #482).

This is a REFERENCE script, not crate-bundled model science. The engine ships
NO diffusion weights and pins NO vendor. The `verbreel-ai` crate spawns
`python3 <this>` and speaks the one-shot JSON sidecar protocol defined in
`crates/verbreel-ai/src/sidecar.rs`:

    stdin  (one line) : {"op":"gen_image","params":{...}}
    stdout (one line) : {"op":"gen_image","result":{...}}
    then exit.

The crate's end-to-end test drives a STUB python3 child with a fixed payload
and does NOT import this file, so the test never needs a real diffusion model.
Operators install the heavy deps (one of: a local SDXL / Flux-class checkpoint
via `diffusers` + `torch`, OR an API-backed generator client) to run this for
real. The header below names the dependency; nothing is bundled.

Mechanism (operator's choice — local diffusion or hosted API):
  * Build a generator from whatever the operator installed (the reference
    loader tries `diffusers` first, then an HTTP API client). If none is
    present this raises ImportError, surfaced loudly below — never a fake
    image.
  * Generate ONE still image from `prompt` at `width` x `height`, pinning the
    RNG to the explicit `seed` so the same prompt + params + seed reproduces
    the same image bytes (the engine's existing `seed` rule for
    particle/glitch effects).
  * Write the produced image (PNG) to a temp file and return its path. The
    engine imports that path through the content-addressed `asset.import`
    path: it hashes the bytes and freezes them under
    assets/<aa>/<sha256>.png as a normal `image`-kind asset. The CAS write is
    the ENGINE's job — this script only produces the bytes and reports where.

Wire contract (request -> result):
  request : {"op":"gen_image","params":{"prompt":"<text>","seed":42,
             "width":1024,"height":1024}}
  result  : {"op":"gen_image","result":{
             "image_path":"/tmp/vr-gen/<uuid>.png","width":1024,
             "height":1024,"seed":42}}

Determinism note: diffusion is the explicit non-deterministic seam. Pinning
`seed` makes a single generation reproducible; the engine then runs this ONCE
and freezes the produced image as a CAS-keyed asset — render stays
byte-deterministic against the frozen asset, never per-render inference. The
`seed` is echoed in the result so the caller can confirm the honored seed.
See issue #482.
"""

import json
import os
import sys
import tempfile
import uuid


def _fail(detail):
    """Exit non-zero with a stderr detail so the crate maps to ProcessExited."""
    sys.stderr.write(detail + "\n")
    sys.exit(1)


def _generate(prompt, seed, width, height):
    """Generate one still image and freeze it to a temp PNG.

    Returns the `result` dict for the sidecar response. Raises ImportError if
    the operator has not installed a generator backend.
    """
    # Operator-installed generation backend. The reference loader picks the
    # first installed of a local diffusers pipeline or an HTTP API client; if
    # none is present this raises ImportError, surfaced loudly below.
    from gen_backend import load_image_generator  # type: ignore

    generator = load_image_generator()

    # The operator-supplied generator must honor the seed for reproducibility;
    # it returns PNG-encoded bytes (no fabricated all-zero image).
    png_bytes = generator.generate(
        prompt=prompt, seed=seed, width=width, height=height
    )
    if not png_bytes:
        raise RuntimeError("image generator returned no bytes")

    out_dir = os.path.join(tempfile.gettempdir(), "vr-gen")
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, f"{uuid.uuid4().hex}.png")
    with open(out_path, "wb") as fh:
        fh.write(png_bytes)

    return {
        "image_path": out_path,
        "width": int(width),
        "height": int(height),
        "seed": int(seed),
    }


def main():
    line = sys.stdin.readline()
    if not line.strip():
        _fail("gen_image sidecar: empty request on stdin")

    try:
        request = json.loads(line)
    except json.JSONDecodeError as err:
        _fail(f"gen_image sidecar: request JSON parse failed: {err}")

    if request.get("op") != "gen_image":
        _fail(f"gen_image sidecar: unexpected op {request.get('op')!r}")

    params = request.get("params", {})
    prompt = params.get("prompt")
    if not prompt:
        _fail("gen_image sidecar: missing params.prompt")
    seed = params.get("seed")
    if seed is None:
        _fail("gen_image sidecar: missing params.seed")
    width = params.get("width")
    height = params.get("height")
    if not width or not height:
        _fail("gen_image sidecar: missing params.width / params.height")

    try:
        result = _generate(prompt, seed, width, height)
    except ImportError as err:
        _fail(
            "gen_image sidecar: diffusion backend not installed "
            f"(operator-install required — local SDXL/Flux via diffusers+torch, "
            f"or an API client): {err}"
        )
    except Exception as err:  # noqa: BLE001 — surface any generation error loudly
        _fail(f"gen_image sidecar: generation failed: {err}")

    sys.stdout.write(json.dumps({"op": "gen_image", "result": result}))
    sys.stdout.write("\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
