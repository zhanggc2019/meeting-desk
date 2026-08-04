use std::sync::Arc;

use crate::config;
use crate::domain::PublicProviderConfig;

use super::{
    openai_chat_completions_minutes_mapping, MinutesProvider, OpenAiCompatibleMinutesProvider,
    ProviderCredentialPlacement, ProviderError, ProviderHttpConfig, ReplaySafety, RetryPolicy,
    TranscriptionProvider, VolcengineFlashTranscriptionProvider, XiaomiMimoTranscriptionProvider,
};

const MAX_TRANSCRIPT_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MINUTES_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// Builds a production transcription adapter from a backend-canonicalized preset.
pub fn build_transcription_provider(
    provider: &PublicProviderConfig,
) -> Result<Arc<dyn TranscriptionProvider>, ProviderError> {
    match provider.preset_id.as_str() {
        config::PRESET_XIAOMI_MIMO_ASR => {
            require_endpoint(provider, config::XIAOMI_MIMO_ASR_ENDPOINT)?;
            Ok(Arc::new(XiaomiMimoTranscriptionProvider::with_reqwest(
                http_config(
                    provider,
                    "xiaomi_mimo_asr",
                    ProviderCredentialPlacement::Bearer,
                    MAX_TRANSCRIPT_RESPONSE_BYTES,
                ),
            )?))
        }
        config::PRESET_VOLCENGINE_ASR_FLASH => {
            require_endpoint(provider, config::VOLCENGINE_ASR_FLASH_ENDPOINT)?;
            Ok(Arc::new(
                VolcengineFlashTranscriptionProvider::with_reqwest(http_config(
                    provider,
                    "volcengine_asr_flash",
                    ProviderCredentialPlacement::Header {
                        header_name: "X-Api-Key".to_string(),
                        prefix: None,
                    },
                    MAX_TRANSCRIPT_RESPONSE_BYTES,
                ))?,
            ))
        }
        _ => Err(ProviderError::configuration(
            "unsupported_transcription_provider",
            "当前语音转写预设尚未接入可执行适配器",
        )),
    }
}

/// Builds an OpenAI-compatible minutes adapter from a validated managed or custom preset.
pub fn build_minutes_provider(
    provider: &PublicProviderConfig,
) -> Result<Arc<dyn MinutesProvider>, ProviderError> {
    match provider.preset_id.as_str() {
        config::PRESET_XIAOMI_MIMO_LLM => {
            require_endpoint(provider, config::XIAOMI_MIMO_LLM_ENDPOINT)?;
        }
        config::PRESET_DEEPSEEK => require_endpoint(provider, config::DEEPSEEK_ENDPOINT)?,
        config::PRESET_ALIYUN_BAILIAN => {
            require_endpoint(provider, config::ALIYUN_BAILIAN_ENDPOINT)?;
        }
        config::PRESET_CUSTOM_OPENAI => {}
        _ => {
            return Err(ProviderError::configuration(
                "unsupported_minutes_provider",
                "当前会议纪要预设尚未接入可执行适配器",
            ));
        }
    }

    Ok(Arc::new(OpenAiCompatibleMinutesProvider::with_reqwest(
        http_config(
            provider,
            "openai_chat_completions_minutes",
            ProviderCredentialPlacement::Bearer,
            MAX_MINUTES_RESPONSE_BYTES,
        ),
        openai_chat_completions_minutes_mapping(),
    )?))
}

/// Creates a conservative HTTP profile: sent requests are never replayed automatically.
fn http_config(
    provider: &PublicProviderConfig,
    adapter_id: &str,
    auth: ProviderCredentialPlacement,
    max_response_bytes: u64,
) -> ProviderHttpConfig {
    ProviderHttpConfig {
        provider_id: provider.preset_id.clone(),
        adapter_id: adapter_id.to_string(),
        adapter_version: "1".to_string(),
        endpoint: provider.endpoint.clone(),
        model: provider.model.clone(),
        auth,
        connect_timeout_ms: provider.connect_timeout_ms,
        request_timeout_ms: provider.request_timeout_ms,
        overall_timeout_ms: provider.request_timeout_ms,
        max_response_bytes,
        max_concurrent: 2,
        min_request_interval_ms: 0,
        retry: RetryPolicy {
            max_retries: provider.max_retries,
            base_delay_ms: 500,
            max_delay_ms: 10_000,
            max_retry_after_ms: 60_000,
        },
        replay_safety: ReplaySafety::NeverAutomaticallyReplay,
        idempotency_header: None,
        allow_insecure_loopback: provider.endpoint.starts_with("http://127.0.0.1")
            || provider.endpoint.starts_with("http://localhost")
            || provider.endpoint.starts_with("http://[::1]"),
    }
}

/// Rejects a tampered managed endpoint before a credential can be attached.
fn require_endpoint(provider: &PublicProviderConfig, expected: &str) -> Result<(), ProviderError> {
    if provider.endpoint == expected {
        Ok(())
    } else {
        Err(ProviderError::configuration(
            "managed_endpoint_mismatch",
            "托管 Provider 地址与受信任预设不一致",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProviderReadiness, PublicProviderConfig};

    fn provider(preset_id: &str, endpoint: &str, model: &str) -> PublicProviderConfig {
        PublicProviderConfig {
            preset_id: preset_id.to_string(),
            kind: "openai_compatible".to_string(),
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            credential_preset_id: Some(preset_id.to_string()),
            secret_configured: true,
            connect_timeout_ms: 1_000,
            request_timeout_ms: 5_000,
            max_retries: 1,
            ready: true,
            readiness: ProviderReadiness::Ready,
            validation_message: String::new(),
        }
    }

    #[test]
    fn builds_only_executable_transcription_presets() {
        let mimo = provider(
            config::PRESET_XIAOMI_MIMO_ASR,
            config::XIAOMI_MIMO_ASR_ENDPOINT,
            "mimo-v2.5-asr",
        );
        assert!(build_transcription_provider(&mimo).is_ok());

        let dashscope = provider(
            config::PRESET_DASHSCOPE_FUNASR_CN,
            config::DASHSCOPE_FUNASR_CN_ENDPOINT,
            "fun-asr",
        );
        assert_eq!(
            build_transcription_provider(&dashscope)
                .err()
                .expect("unsupported adapter")
                .code,
            "unsupported_transcription_provider"
        );
    }

    #[test]
    fn rejects_tampered_managed_minutes_endpoint() {
        let deepseek = provider(
            config::PRESET_DEEPSEEK,
            "https://example.invalid/chat/completions",
            "deepseek-v4-flash",
        );
        assert_eq!(
            build_minutes_provider(&deepseek)
                .err()
                .expect("managed endpoint must be fixed")
                .code,
            "managed_endpoint_mismatch"
        );
    }
}
