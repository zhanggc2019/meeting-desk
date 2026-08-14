"""Run local SenseVoiceSmall inference without emitting transcript content to logs."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

LANGUAGE_PATTERN = re.compile(r"<\|(?P<language>zh|en|yue|ja|ko|nospeech)\|>")


def parse_args() -> argparse.Namespace:
    """Parse the narrow command contract used by the Rust adapter."""
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--model-dir", type=Path)
    parser.add_argument("--vad-model-dir", type=Path)
    parser.add_argument("--audio", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--language", default="auto")
    return parser.parse_args()


def detect_language(raw_text: str) -> str | None:
    """Extract SenseVoice's language tag before rich-text postprocessing removes it."""
    match = LANGUAGE_PATTERN.search(raw_text)
    if match is None or match.group("language") == "nospeech":
        return None
    return match.group("language")


def normalize_results(results: Any, postprocess: Any) -> dict[str, Any]:
    """Normalize FunASR result dictionaries into the bounded Rust JSON contract."""
    if not isinstance(results, list):
        raise ValueError("invalid result container")
    texts: list[str] = []
    detected_language: str | None = None
    for item in results:
        if not isinstance(item, dict) or not isinstance(item.get("text"), str):
            raise ValueError("invalid result item")
        raw_text = item["text"]
        detected_language = detected_language or detect_language(raw_text)
        text = postprocess(raw_text).strip()
        if text:
            texts.append(text)
    text = "\n".join(texts).strip()
    if not text:
        raise ValueError("empty transcript")
    return {"text": text, "language": detected_language, "segments": []}


def write_result(path: Path, payload: dict[str, Any]) -> None:
    """Write UTF-8 JSON only to the adapter-owned temporary output file."""
    path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")


def load_model(
    auto_model: Any,
    model_dir: Path | str,
    vad_model_dir: Path | str,
) -> Any:
    """Load local SenseVoice and VAD models with bounded speech segments."""
    return auto_model(
        model=str(model_dir),
        vad_model=str(vad_model_dir),
        vad_kwargs={"max_single_segment_time": 30_000},
        device="cpu",
        disable_update=True,
        trust_remote_code=False,
    )


def main() -> int:
    """Load local-only dependencies and run one offline inference operation."""
    args = parse_args()
    try:
        from funasr import AutoModel
        from funasr.utils.postprocess_utils import rich_transcription_postprocess
    except Exception:
        return 20
    if args.model_dir is None or args.vad_model_dir is None:
        return 23
    try:
        model = load_model(AutoModel, args.model_dir, args.vad_model_dir)
    except Exception:
        return 21
    if args.check:
        return 0
    if args.audio is None or args.output is None:
        return 23
    try:
        results = model.generate(
            input=str(args.audio),
            cache={},
            language=args.language,
            use_itn=True,
            batch_size_s=60,
        )
    except Exception:
        return 22
    try:
        write_result(args.output, normalize_results(results, rich_transcription_postprocess))
    except Exception:
        return 23
    return 0


if __name__ == "__main__":
    sys.exit(main())
