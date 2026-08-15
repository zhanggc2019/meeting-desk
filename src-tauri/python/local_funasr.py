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


def normalize_timestamp(value: Any) -> int | None:
    """Convert a non-negative FunASR millisecond timestamp to an integer."""
    if isinstance(value, bool):
        return None
    try:
        timestamp = int(float(value))
    except (TypeError, ValueError, OverflowError):
        return None
    return timestamp if timestamp >= 0 else None


def normalize_sentence_segments(item: dict[str, Any], postprocess: Any) -> list[dict[str, Any]]:
    """Normalize FunASR sentence_info records while preserving real VAD boundaries."""
    raw_segments = item.get("sentence_info")
    if not isinstance(raw_segments, list):
        return []

    segments: list[dict[str, Any]] = []
    for raw_segment in raw_segments:
        if not isinstance(raw_segment, dict):
            continue
        raw_text = raw_segment.get("sentence") or raw_segment.get("text")
        if not isinstance(raw_text, str):
            continue
        text = postprocess(raw_text).strip()
        if not text:
            continue
        start_ms = normalize_timestamp(raw_segment.get("start", raw_segment.get("start_time")))
        end_ms = normalize_timestamp(raw_segment.get("end", raw_segment.get("end_time")))
        if start_ms is None or end_ms is None or end_ms < start_ms:
            start_ms = None
            end_ms = None
        segments.append({"start_ms": start_ms, "end_ms": end_ms, "text": text})
    return segments


def normalize_results(results: Any, postprocess: Any) -> dict[str, Any]:
    """Normalize FunASR result dictionaries into the bounded Rust JSON contract."""
    if not isinstance(results, list):
        raise ValueError("invalid result container")
    texts: list[str] = []
    segments: list[dict[str, Any]] = []
    detected_language: str | None = None
    for item in results:
        if not isinstance(item, dict) or not isinstance(item.get("text"), str):
            raise ValueError("invalid result item")
        raw_text = item["text"]
        detected_language = detected_language or detect_language(raw_text)
        item_segments = normalize_sentence_segments(item, postprocess)
        if item_segments:
            segments.extend(item_segments)
            texts.append("\n".join(segment["text"] for segment in item_segments))
        else:
            text = postprocess(raw_text).strip()
            if text:
                texts.append(text)
    text = "\n".join(texts).strip()
    if not text:
        raise ValueError("empty transcript")
    return {"text": text, "language": detected_language, "segments": segments}


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


def generate_transcript(model: Any, audio: Path | str, language: str) -> Any:
    """Run offline inference and request VAD-backed sentence timestamps for paragraph output."""
    return model.generate(
        input=str(audio),
        cache={},
        language=language,
        use_itn=True,
        batch_size_s=60,
        sentence_timestamp=True,
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
        results = generate_transcript(model, args.audio, args.language)
    except Exception:
        return 22
    try:
        write_result(args.output, normalize_results(results, rich_transcription_postprocess))
    except Exception:
        return 23
    return 0


if __name__ == "__main__":
    sys.exit(main())
