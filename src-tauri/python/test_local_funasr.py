"""Unit tests for the local FunASR result normalization boundary."""

import unittest

from local_funasr import detect_language, load_model, normalize_results


class LocalFunAsrNormalizationTests(unittest.TestCase):
    """Verify that model-specific rich text becomes the stable adapter contract."""

    def test_detects_supported_language_tag(self) -> None:
        """Extract a supported SenseVoice language tag without retaining rich text."""
        self.assertEqual(detect_language("<|zh|><|NEUTRAL|>content"), "zh")
        self.assertIsNone(detect_language("<|nospeech|>"))

    def test_normalizes_multiple_results(self) -> None:
        """Join non-empty chunks and preserve only the first detected language."""
        payload = normalize_results(
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
            normalize_results([], lambda value: value)
        with self.assertRaises(ValueError):
            normalize_results([{"unknown": "value"}], lambda value: value)

    def test_load_model_uses_offline_cpu_configuration(self) -> None:
        """Instantiate the selected local model during both checks and inference."""
        calls = []

        def fake_auto_model(**kwargs):
            """Capture the model constructor arguments without loading a real model."""
            calls.append(kwargs)
            return object()

        model = load_model(fake_auto_model, "model/SenseVoiceSmall")

        self.assertIsNotNone(model)
        self.assertEqual(
            calls,
            [{
                "model": "model/SenseVoiceSmall",
                "device": "cpu",
                "disable_update": True,
                "trust_remote_code": False,
            }],
        )


if __name__ == "__main__":
    unittest.main()
