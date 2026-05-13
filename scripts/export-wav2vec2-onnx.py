#!/usr/bin/env python3
"""Export wav2vec2 CTC fine-tunes to ONNX for use by `aligner-wav2vec2`.

Usage:
    pip install transformers optimum[onnxruntime] onnx torch
    python3 scripts/export-wav2vec2-onnx.py --langs it en

Outputs to ~/Library/Caches/cologna-karaoke/wav2vec2/<lang>/{model.onnx,vocab.json}
(macOS) or the XDG cache equivalent on Linux.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

LANG_TO_HF = {
    "it": "jonatasgrosman/wav2vec2-large-xlsr-53-italian",
    "en": "jonatasgrosman/wav2vec2-large-xlsr-53-english",
    "es": "jonatasgrosman/wav2vec2-large-xlsr-53-spanish",
    "fr": "jonatasgrosman/wav2vec2-large-xlsr-53-french",
    "pt": "jonatasgrosman/wav2vec2-large-xlsr-53-portuguese",
    "de": "jonatasgrosman/wav2vec2-large-xlsr-53-german",
}


def cache_dir() -> Path:
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Caches" / "cologna-karaoke" / "wav2vec2"
    if sys.platform.startswith("linux"):
        base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
        return Path(base) / "cologna-karaoke" / "wav2vec2"
    return Path.home() / ".cache" / "cologna-karaoke" / "wav2vec2"


def export_one(lang: str, hf_id: str, out_dir: Path) -> None:
    from optimum.onnxruntime import ORTModelForCTC

    out_dir.mkdir(parents=True, exist_ok=True)
    print(f"[w2v-export] {lang}: {hf_id} -> {out_dir}")

    work = out_dir / "_export_tmp"
    if work.exists():
        shutil.rmtree(work)
    model = ORTModelForCTC.from_pretrained(hf_id, export=True)
    model.save_pretrained(work)

    src_onnx = work / "model.onnx"
    if not src_onnx.exists():
        candidates = list(work.glob("*.onnx"))
        if not candidates:
            raise RuntimeError(f"no .onnx produced in {work}")
        src_onnx = candidates[0]
    shutil.move(str(src_onnx), str(out_dir / "model.onnx"))

    vocab = work / "vocab.json"
    if vocab.exists():
        shutil.move(str(vocab), str(out_dir / "vocab.json"))
    shutil.rmtree(work, ignore_errors=True)
    print(f"[w2v-export] {lang}: done")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--langs", nargs="+", default=["it", "en"], help="ISO 639-1 codes")
    parser.add_argument("--out", default=str(cache_dir()), help="output root")
    args = parser.parse_args()

    out_root = Path(args.out)
    for lang in args.langs:
        if lang not in LANG_TO_HF:
            print(f"[w2v-export] unknown language '{lang}', skipping")
            continue
        export_one(lang, LANG_TO_HF[lang], out_root / lang)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
