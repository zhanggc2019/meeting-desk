use crate::domain::{ProviderReadiness, PublicProviderConfig, PublicSettings};
use sha2::{Digest, Sha256};

pub const PRESET_MOCK: &str = "mock";
pub const PRESET_LOCAL_FUNASR: &str = "local_funasr";
pub const PRESET_DASHSCOPE_FUNASR_CN: &str = "dashscope_funasr_cn";
pub const PRESET_DASHSCOPE_FUNASR_INTL: &str = "dashscope_funasr_intl";
pub const PRESET_XIAOMI_MIMO_ASR: &str = "xiaomi_mimo_asr";
pub const PRESET_VOLCENGINE_ASR_FLASH: &str = "volcengine_asr_flash";
pub const PRESET_XIAOMI_MIMO_LLM: &str = "xiaomi_mimo_llm";
pub const PRESET_DEEPSEEK: &str = "deepseek";
pub const PRESET_ALIYUN_BAILIAN: &str = "aliyun_bailian";
pub const PRESET_CUSTOM_OPENAI: &str = "custom_openai_compatible";

pub const DASHSCOPE_FUNASR_CN_ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription";
pub const DASHSCOPE_FUNASR_INTL_ENDPOINT: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/audio/asr/transcription";
pub const XIAOMI_MIMO_ASR_ENDPOINT: &str = "https://api.xiaomimimo.com/v1/chat/completions";
pub const XIAOMI_MIMO_LLM_ENDPOINT: &str = XIAOMI_MIMO_ASR_ENDPOINT;
pub const VOLCENGINE_ASR_FLASH_ENDPOINT: &str =
    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash";
pub const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
pub const ALIYUN_BAILIAN_ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";
pub const LOCAL_FUNASR_ENDPOINT: &str = "local://model/SenseVoiceSmall";
pub const LOCAL_FUNASR_MODEL: &str = "SenseVoiceSmall";
pub const LOCAL_FUNASR_DEFAULT_TIMEOUT_MS: u64 = 3_600_000;
pub const LOCAL_FUNASR_MAX_TIMEOUT_MS: u64 = 14_400_000;

/// 从进程环境读取非秘密 Provider 默认配置。
pub fn provider_settings_from_environment() -> PublicSettings {
    evaluate_settings_readiness(PublicSettings {
        transcription: provider_from_environment("ASR"),
        minutes: provider_from_environment("LLM"),
    })
}

/// 从统一前缀读取一个 Provider 的公开环境配置。
fn provider_from_environment(prefix: &str) -> PublicProviderConfig {
    let defaults = PublicProviderConfig::default();
    let (default_preset, default_kind, default_endpoint, default_model) = provider_defaults(prefix);
    let kind = read_text(
        &format!("MEETING_DESK_{prefix}_PROVIDER_KIND"),
        default_kind,
    );
    let endpoint = read_text(&format!("MEETING_DESK_{prefix}_BASE_URL"), default_endpoint);
    let model = read_text(&format!("MEETING_DESK_{prefix}_MODEL"), default_model);
    let preset_id = std::env::var(format!("MEETING_DESK_{prefix}_PRESET_ID"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| infer_preset_from_endpoint(&endpoint, &model, default_preset));
    let secret_configured = environment_secret_exists(&format!("MEETING_DESK_{prefix}_API_KEY"));
    evaluate_provider_readiness(PublicProviderConfig {
        preset_id: preset_id.clone(),
        kind,
        endpoint,
        model,
        local_model_path: if prefix == "ASR" {
            read_text("MEETING_DESK_ASR_MODEL_DIR", "")
        } else {
            String::new()
        },
        credential_preset_id: secret_configured.then_some(preset_id.clone()),
        secret_configured,
        connect_timeout_ms: read_number(
            "MEETING_DESK_CONNECT_TIMEOUT_MS",
            defaults.connect_timeout_ms,
            1_000,
            60_000,
        ),
        request_timeout_ms: read_number(
            "MEETING_DESK_REQUEST_TIMEOUT_MS",
            if prefix == "ASR" && preset_id == PRESET_LOCAL_FUNASR {
                LOCAL_FUNASR_DEFAULT_TIMEOUT_MS
            } else {
                defaults.request_timeout_ms
            },
            5_000,
            if prefix == "ASR" && preset_id == PRESET_LOCAL_FUNASR {
                LOCAL_FUNASR_MAX_TIMEOUT_MS
            } else {
                600_000
            },
        ),
        max_retries: read_number("MEETING_DESK_MAX_RETRIES", defaults.max_retries, 0, 5),
        ready: false,
        readiness: ProviderReadiness::Incomplete,
        validation_message: String::new(),
    })
}

/// 根据可信环境地址推断预设；自定义地址会自动进入可编辑的 OpenAI-compatible 预设。
fn infer_preset_from_endpoint(endpoint: &str, model: &str, fallback: &str) -> String {
    match endpoint.trim() {
        LOCAL_FUNASR_ENDPOINT => PRESET_LOCAL_FUNASR.to_string(),
        DASHSCOPE_FUNASR_CN_ENDPOINT => PRESET_DASHSCOPE_FUNASR_CN.to_string(),
        DASHSCOPE_FUNASR_INTL_ENDPOINT => PRESET_DASHSCOPE_FUNASR_INTL.to_string(),
        XIAOMI_MIMO_ASR_ENDPOINT if model.trim() == "mimo-v2.5-asr" => {
            PRESET_XIAOMI_MIMO_ASR.to_string()
        }
        XIAOMI_MIMO_LLM_ENDPOINT => PRESET_XIAOMI_MIMO_LLM.to_string(),
        VOLCENGINE_ASR_FLASH_ENDPOINT => PRESET_VOLCENGINE_ASR_FLASH.to_string(),
        DEEPSEEK_ENDPOINT => PRESET_DEEPSEEK.to_string(),
        ALIYUN_BAILIAN_ENDPOINT => PRESET_ALIYUN_BAILIAN.to_string(),
        "" => fallback.to_string(),
        _ => PRESET_CUSTOM_OPENAI.to_string(),
    }
}

/// 返回转写与纪要服务的安全托管默认值，环境变量可在进程级高级覆盖。
fn provider_defaults(prefix: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match prefix {
        "ASR" => (
            PRESET_LOCAL_FUNASR,
            "local_funasr",
            LOCAL_FUNASR_ENDPOINT,
            LOCAL_FUNASR_MODEL,
        ),
        "LLM" => (
            PRESET_DEEPSEEK,
            "openai_compatible",
            DEEPSEEK_ENDPOINT,
            "deepseek-v4-flash",
        ),
        _ => (PRESET_CUSTOM_OPENAI, "openai_compatible", "", ""),
    }
}

/// 根据旧配置的类型和精确托管地址推断稳定预设标识，未知地址归入自定义预设。
pub fn infer_preset_id(provider: &PublicProviderConfig) -> String {
    infer_preset_from_endpoint(&provider.endpoint, &provider.model, PRESET_CUSTOM_OPENAI)
}

/// Returns a stable credential binding; custom endpoints receive an opaque endpoint fingerprint.
pub fn credential_binding_id(provider: &PublicProviderConfig) -> String {
    if provider.preset_id == PRESET_CUSTOM_OPENAI {
        let digest = hex::encode(Sha256::digest(provider.endpoint.trim().as_bytes()));
        format!("{PRESET_CUSTOM_OPENAI}:{}", &digest[..24])
    } else {
        provider.preset_id.clone()
    }
}

/// 根据公开字段和秘密存在标记计算安全的 Provider 就绪状态。
pub fn evaluate_provider_readiness(mut provider: PublicProviderConfig) -> PublicProviderConfig {
    if provider.kind == "mock" {
        provider.ready = false;
        provider.readiness = ProviderReadiness::Incomplete;
        provider.validation_message = "旧版演示配置已停用，请重新配置服务".to_string();
        return provider;
    }

    let is_local_funasr =
        provider.preset_id == PRESET_LOCAL_FUNASR && provider.kind == "local_funasr";
    let mut missing = Vec::new();
    if provider.endpoint.trim().is_empty() {
        missing.push("API 地址");
    }
    if provider.model.trim().is_empty() {
        missing.push("模型名");
    }
    if !is_local_funasr && !provider.secret_configured {
        missing.push("API Key");
    }

    let supported_kind = matches!(provider.kind.as_str(), "local_funasr" | "openai_compatible");
    if supported_kind && missing.is_empty() {
        provider.ready = true;
        provider.readiness = ProviderReadiness::Ready;
        provider.validation_message = if is_local_funasr {
            "本地 SenseVoiceSmall 配置已就绪".to_string()
        } else {
            "真实 Provider 配置已就绪".to_string()
        };
    } else {
        provider.ready = false;
        provider.readiness = ProviderReadiness::Incomplete;
        provider.validation_message = if !supported_kind {
            "Provider 类型无效，请重新配置".to_string()
        } else {
            format!("请补充：{}", missing.join("、"))
        };
    }
    provider
}

/// 重新计算转写与纪要两个 Provider 的公开就绪状态。
pub fn evaluate_settings_readiness(mut settings: PublicSettings) -> PublicSettings {
    settings.transcription = evaluate_provider_readiness(settings.transcription);
    settings.minutes = evaluate_provider_readiness(settings.minutes);
    settings
}

/// 读取非空环境字符串，否则使用安全默认值。
fn read_text(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// 读取带上下界的数字环境配置，非法值回退到默认值。
fn read_number<T>(name: &str, fallback: T, minimum: T, maximum: T) -> T
where
    T: std::str::FromStr + PartialOrd + Copy,
{
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<T>().ok())
        .filter(|value| *value >= minimum && *value <= maximum)
        .unwrap_or(fallback)
}

/// 只判断环境秘密是否已配置，不读取或暴露其内容。
fn environment_secret_exists(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造用于就绪状态测试的公开真实 Provider 配置。
    fn provider(endpoint: &str, model: &str, secret_configured: bool) -> PublicProviderConfig {
        PublicProviderConfig {
            preset_id: PRESET_CUSTOM_OPENAI.to_string(),
            kind: "openai_compatible".to_string(),
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            local_model_path: String::new(),
            credential_preset_id: secret_configured.then_some(PRESET_CUSTOM_OPENAI.to_string()),
            secret_configured,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            max_retries: 2,
            ready: false,
            readiness: ProviderReadiness::Incomplete,
            validation_message: String::new(),
        }
    }

    /// 验证完全未配置时会返回安全且可操作的缺项提示。
    #[test]
    fn reports_all_missing_real_provider_fields() {
        let evaluated = evaluate_provider_readiness(provider("", "", false));
        assert!(!evaluated.ready);
        assert_eq!(evaluated.readiness, ProviderReadiness::Incomplete);
        assert!(evaluated.validation_message.contains("API 地址"));
        assert!(evaluated.validation_message.contains("模型名"));
        assert!(evaluated.validation_message.contains("API Key"));
    }

    /// 验证只配置公开字段但缺少秘密时仍不视为真实就绪。
    #[test]
    fn requires_secret_for_real_provider_readiness() {
        let evaluated = evaluate_provider_readiness(provider(
            "https://api.example.test/v1",
            "example-model",
            false,
        ));
        assert!(!evaluated.ready);
        assert_eq!(evaluated.validation_message, "请补充：API Key");
    }

    /// 验证地址、模型和秘密存在标记齐全时才视为真实就绪。
    #[test]
    fn marks_complete_real_provider_as_ready() {
        let evaluated = evaluate_provider_readiness(provider(
            "https://api.example.test/v1",
            "example-model",
            true,
        ));
        assert!(evaluated.ready);
        assert_eq!(evaluated.readiness, ProviderReadiness::Ready);
    }

    /// 验证旧版 Mock 配置已停用且不能冒充真实配置就绪。
    #[test]
    fn rejects_legacy_mock_as_real_ready() {
        let legacy = PublicProviderConfig {
            preset_id: PRESET_MOCK.to_string(),
            kind: "mock".to_string(),
            ..PublicProviderConfig::default()
        };
        let evaluated = evaluate_provider_readiness(legacy);
        assert!(!evaluated.ready);
        assert_eq!(evaluated.readiness, ProviderReadiness::Incomplete);
        assert!(evaluated.validation_message.contains("已停用"));
    }

    /// 验证未配置环境覆盖时使用本地 SenseVoiceSmall 和 DeepSeek 默认值。
    #[test]
    fn defines_managed_environment_defaults() {
        assert_eq!(
            provider_defaults("ASR"),
            (
                PRESET_LOCAL_FUNASR,
                "local_funasr",
                LOCAL_FUNASR_ENDPOINT,
                LOCAL_FUNASR_MODEL
            )
        );
        assert_eq!(
            provider_defaults("LLM"),
            (
                PRESET_DEEPSEEK,
                "openai_compatible",
                DEEPSEEK_ENDPOINT,
                "deepseek-v4-flash"
            )
        );
    }

    /// 验证本地 FunASR 不需要 API Key 即可通过就绪检查。
    #[test]
    fn accepts_local_funasr_without_api_key() {
        let mut value = provider(LOCAL_FUNASR_ENDPOINT, LOCAL_FUNASR_MODEL, false);
        value.kind = "local_funasr".to_string();
        value.preset_id = PRESET_LOCAL_FUNASR.to_string();
        assert!(evaluate_provider_readiness(value).ready);
    }

    /// 验证已停用的在线 ASR 类型不会进入就绪状态。
    #[test]
    fn rejects_disabled_online_asr_provider_readiness() {
        let mut mimo = provider(XIAOMI_MIMO_ASR_ENDPOINT, "mimo-v2.5-asr", true);
        mimo.kind = "xiaomi_mimo".to_string();
        mimo.preset_id = PRESET_XIAOMI_MIMO_ASR.to_string();
        assert!(!evaluate_provider_readiness(mimo).ready);

        let mut volc = provider(VOLCENGINE_ASR_FLASH_ENDPOINT, "bigmodel", true);
        volc.kind = "volcengine_asr".to_string();
        volc.preset_id = PRESET_VOLCENGINE_ASR_FLASH.to_string();
        assert!(!evaluate_provider_readiness(volc).ready);
    }

    /// 验证百炼集中式 Chat Completions 地址能被精确识别为托管预设。
    #[test]
    fn infers_aliyun_bailian_managed_preset() {
        assert_eq!(
            infer_preset_from_endpoint(ALIYUN_BAILIAN_ENDPOINT, "qwen-plus", PRESET_CUSTOM_OPENAI),
            PRESET_ALIYUN_BAILIAN
        );
    }

    /// 验证新增转写端点只能通过精确官方地址识别为托管预设。
    #[test]
    fn infers_new_managed_asr_presets_from_exact_endpoints() {
        assert_eq!(
            infer_preset_from_endpoint(
                XIAOMI_MIMO_ASR_ENDPOINT,
                "mimo-v2.5-asr",
                PRESET_CUSTOM_OPENAI
            ),
            PRESET_XIAOMI_MIMO_ASR
        );
        assert_eq!(
            infer_preset_from_endpoint(
                VOLCENGINE_ASR_FLASH_ENDPOINT,
                "bigmodel",
                PRESET_CUSTOM_OPENAI
            ),
            PRESET_VOLCENGINE_ASR_FLASH
        );
    }

    /// 验证 MiMo 共用地址会依据模型区分 ASR 与会议纪要预设。
    #[test]
    fn distinguishes_xiaomi_mimo_presets_by_model() {
        assert_eq!(
            infer_preset_from_endpoint(XIAOMI_MIMO_LLM_ENDPOINT, "mimo-v2.5", PRESET_CUSTOM_OPENAI),
            PRESET_XIAOMI_MIMO_LLM
        );
        assert_eq!(
            infer_preset_from_endpoint(
                XIAOMI_MIMO_LLM_ENDPOINT,
                "mimo-v2.5-pro",
                PRESET_CUSTOM_OPENAI
            ),
            PRESET_XIAOMI_MIMO_LLM
        );
    }

    #[test]
    fn custom_credential_binding_changes_with_endpoint_without_exposing_it() {
        let first = provider("https://first.example.test/v1", "model", true);
        let second = provider("https://second.example.test/v1", "model", true);
        let first_binding = credential_binding_id(&first);
        assert_ne!(first_binding, credential_binding_id(&second));
        assert!(!first_binding.contains("first.example"));
    }

    /// 验证只配置一类真实 Provider 时，另一类不会被误报为就绪。
    #[test]
    fn evaluates_only_one_configured_provider() {
        let evaluated = evaluate_settings_readiness(PublicSettings {
            transcription: provider("https://api.example.test/v1", "asr-model", true),
            minutes: provider("", "", false),
        });
        assert!(evaluated.transcription.ready);
        assert!(!evaluated.minutes.ready);
    }

    /// 验证转写与纪要配置分别完整时，两者都会被标记为真实就绪。
    #[test]
    fn evaluates_both_configured_providers() {
        let evaluated = evaluate_settings_readiness(PublicSettings {
            transcription: provider("https://asr.example.test/v1", "asr-model", true),
            minutes: provider("https://llm.example.test/v1", "llm-model", true),
        });
        assert!(evaluated.transcription.ready);
        assert!(evaluated.minutes.ready);
    }
}
