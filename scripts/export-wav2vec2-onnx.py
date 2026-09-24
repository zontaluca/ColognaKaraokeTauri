#!/usr/bin/env python3
"""Export wav2vec2 CTC fine-tunes to ONNX for use by `aligner-wav2vec2`.

Usage:
    pip install transformers onnx torch onnxscript
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

try:
    import torch
    import onnx
    import onnxscript  # required by torch.onnx export in newer torch versions
    from transformers import AutoModelForCTC, AutoTokenizer
except ModuleNotFoundError as exc:
    missing = exc.name or str(exc)
    raise SystemExit(
        f"Missing Python dependency '{missing}'. Install with: "
        "pip install torch onnx onnxscript transformers"
    )

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


def export_one(lang: str, hf_id: str, out_dir: Path, opset: int) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    print(f"[w2v-export] {lang}: {hf_id} -> {out_dir}")

    work = out_dir / "_export_tmp"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)

    model = AutoModelForCTC.from_pretrained(hf_id)
    model.eval()

    dummy_input = torch.randn(1, 16000, dtype=torch.float32)
    torch.onnx.export(
        model,
        dummy_input,
        work / "model.onnx",
        export_params=True,
        opset_version=opset,
        do_constant_folding=True,
        input_names=["input_values"],
        output_names=["logits"],
        dynamic_axes={
            "input_values": {0: "batch", 1: "sequence"},
            "logits": {0: "batch", 1: "sequence"},
        },
    )

    AutoTokenizer.from_pretrained(hf_id).save_pretrained(work)

    src_onnx = work / "model.onnx"
    if not src_onnx.exists():
        raise RuntimeError(f"no .onnx produced in {work}")
    shutil.move(str(src_onnx), str(out_dir / "model.onnx"))

    vocab = work / "vocab.json"
    if vocab.exists():
        shutil.move(str(vocab), str(out_dir / "vocab.json"))
    else:
        raise RuntimeError(f"vocab.json not found in {work} after tokenizer save")
    shutil.rmtree(work, ignore_errors=True)
    print(f"[w2v-export] {lang}: done")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--langs", nargs="+", default=["it", "en"], help="ISO 639-1 codes")
    parser.add_argument("--out", default=str(cache_dir()), help="output root")
    parser.add_argument("--opset", type=int, default=13, help="ONNX opset version")
    args = parser.parse_args()

    out_root = Path(args.out)
    for lang in args.langs:
        if lang not in LANG_TO_HF:
            print(f"[w2v-export] unknown language '{lang}', skipping")
            continue
        export_one(lang, LANG_TO_HF[lang], out_root / lang, args.opset)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
