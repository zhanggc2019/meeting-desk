"""Unit tests for the local offline FunASR subprocess contract."""

import importlib.util
from pathlib import Path
import unittest

MODULE_PATH = Path(__file__).with_name("local_funasr.py")
MODULE_SPEC = importlib.util.spec_from_file_location("local_funasr", MODULE_PATH)
if MODULE_SPEC is None or MODULE_SPEC.loader is None:
    raise RuntimeError("unable to load local FunASR module")
local_funasr = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(local_funasr)


class LocalFunAsrTests(unittest.TestCase):
    """Verify result normalization and bounded local VAD model loading."""

    def test_detects_supported_language_tag(self) -> None:
        """Extract a supported SenseVoice language tag without retaining rich text."""
        self.assertEqual(local_funasr.detect_language("<|zh|><|NEUTRAL|>content"), "zh")
        self.assertIsNone(local_funasr.detect_language("<|nospeech|>"))

    def test_normalizes_multiple_results(self) -> None:
        """Join non-empty chunks and preserve only the first detected language."""
        payload = local_funasr.normalize_results(
            [
                {"text": "<|zh|>first"},
                {"text": "<|en|>second"},
            ],
            lambda value: value.split(">", 1)[-1],
        )
        self.assertEqual(payload["text"], "first\nsecond")
        self.assertEqual(payload["language"], "zh")
        self.assertEqual(payload["segments"], [])

    def test_rejects_empty_or_malformed_results(self) -> None:
        """Reject empty transcripts and provider results with an unknown shape."""
        with self.assertRaises(ValueError):
            local_funasr.normalize_results([], lambda value: value)
        with self.assertRaises(ValueError):
            local_funasr.normalize_results([{"unknown": "value"}], lambda value: value)

    def test_load_model_enables_local_vad_with_thirty_second_segments(self) -> None:
        """Pass both local model paths and the hard VAD segment limit to FunASR."""
        calls: list[dict[str, object]] = []

        def fake_auto_model(**kwargs: object) -> object:
            """Capture trusted AutoModel arguments without loading model dependencies."""
            calls.append(kwargs)
            return object()

        model = local_funasr.load_model(
            fake_auto_model,
            r"D:\Models\SenseVoiceSmall",
            r"D:\Models\fsmn-vad",
        )

        self.assertIsNotNone(model)
        self.assertEqual(
            calls,
            [
                {
                    "model": r"D:\Models\SenseVoiceSmall",
                    "vad_model": r"D:\Models\fsmn-vad",
                    "vad_kwargs": {"max_single_segment_time": 30_000},
                    "device": "cpu",
                    "disable_update": True,
                    "trust_remote_code": False,
                }
            ],
        )


if __name__ == "__main__":
    unittest.main()
