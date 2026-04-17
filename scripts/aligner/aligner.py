#!/usr/bin/env python3
"""
Word-level alignment CLI.
Usage:
    aligner --audio PATH --out PATH --model tiny [--text-file PATH]

- With --text-file: uses stable_whisper forced alignment (maps provided lyrics to audio).
- Without: runs openai-whisper transcription with word_timestamps.
Emits JSON list [{word, start_ms, end_ms}, ...] to --out.
"""
import argparse
import json
import multiprocessing
import os
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audio", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--model", default="tiny")
    parser.add_argument("--text-file", default=None)
    args = parser.parse_args()

    # stable_whisper bundles whisper
    import stable_whisper
    import whisper
    try:
        import torch
        torch.set_num_threads(1)
    except Exception:
        pass

    model = stable_whisper.load_model(args.model)

    if args.text_file:
        with open(args.text_file, encoding="utf-8") as f:
            text = f.read().strip()
        # Detect language from audio
        clip = whisper.pad_or_trim(whisper.load_audio(args.audio))
        mel = whisper.log_mel_spectrogram(clip).to(model.device)
        _, probs = model.detect_language(mel)
        lang = max(probs, key=probs.get) if probs else "en"
        result = model.align(args.audio, text, language=lang)
    else:
        result = model.transcribe(args.audio, word_timestamps=True)

    words = []
    for seg in getattr(result, "segments", []) or []:
        for w in (getattr(seg, "words", None) or []):
            token = (getattr(w, "word", "") or "").strip()
            if not token:
                continue
            words.append({
                "word": token,
                "start_ms": int(float(w.start) * 1000),
                "end_ms": int(float(w.end) * 1000),
            })

    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(words, f, ensure_ascii=False, indent=2)
    return 0


if __name__ == "__main__":
    # PyInstaller-on-macOS: multiprocessing children re-exec the bundled binary.
    # freeze_support short-circuits child runs; fork avoids re-running argparse.
    multiprocessing.freeze_support()
    try:
        multiprocessing.set_start_method("fork", force=True)
    except (RuntimeError, ValueError):
        pass
    # Limit torch threads → fewer helper processes → less spurious stderr on shutdown
    os.environ.setdefault("OMP_NUM_THREADS", "1")
    os.environ.setdefault("MKL_NUM_THREADS", "1")
    try:
        sys.exit(main())
    except Exception as e:
        print(f"aligner error: {e}", file=sys.stderr)
        sys.exit(1)
