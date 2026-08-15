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

    def test_preserves_sentence_info_as_timestamped_segments(self) -> None:
        """Convert FunASR VAD sentence records into readable transcript paragraphs."""
        payload = local_funasr.normalize_results(
            [
                {
                    "text": "<|zh|>first second",
                    "sentence_info": [
                        {"start": 120, "end": 930, "sentence": "<|zh|>first"},
                        {"start": 1_100.4, "end": 2_300, "text": "<|zh|>second"},
                    ],
                }
            ],
            lambda value: value.split(">", 1)[-1],
        )

        self.assertEqual(payload["text"], "first\nsecond")
        self.assertEqual(
            payload["segments"],
            [
                {"start_ms": 120, "end_ms": 930, "text": "first"},
                {"start_ms": 1_100, "end_ms": 2_300, "text": "second"},
            ],
        )

    def test_keeps_text_when_sentence_metadata_is_invalid(self) -> None:
        """Keep readable text and omit invented timestamps for malformed sentence metadata."""
        payload = local_funasr.normalize_results(
            [
                {
                    "text": "<|zh|>content",
                    "sentence_info": [
                        {"start": 900, "end": 100, "text": "<|zh|>content"},
                    ],
                }
            ],
            lambda value: value.split(">", 1)[-1],
        )

        self.assertEqual(
            payload["segments"],
            [{"start_ms": None, "end_ms": None, "text": "content"}],
        )

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

    def test_generate_requests_vad_sentence_timestamps(self) -> None:
        """Request sentence metadata so local VAD boundaries survive the Python adapter."""
        calls: list[dict[str, object]] = []

        class FakeModel:
            """Capture inference arguments without loading an ASR model."""

            def generate(self, **kwargs: object) -> list[dict[str, str]]:
                """Record one deterministic generate call."""
                calls.append(kwargs)
                return [{"text": "fixture"}]

        result = local_funasr.generate_transcript(FakeModel(), r"D:\audio.wav", "zh")

        self.assertEqual(result, [{"text": "fixture"}])
        self.assertEqual(calls[0]["input"], r"D:\audio.wav")
        self.assertEqual(calls[0]["language"], "zh")
        self.assertTrue(calls[0]["sentence_timestamp"])


if __name__ == "__main__":
    unittest.main()
