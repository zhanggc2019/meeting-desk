use serde::Deserialize;
use tauri::State;

use crate::app_state::AppState;
use crate::commands::CommandError;
use crate::config;
use crate::domain::{PublicProviderConfig, PublicSettings};
use crate::secrets::{self, SecretKind};

const SETTINGS_KEY: &str = "provider_settings_v1";

/// 接收单个 Provider 的配置；Key 只用于写入系统凭据管理器。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsInput {
    #[serde(default)]
    pub preset_id: String,
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub max_retries: u32,
}

/// 接收转写和纪要两个独立 Provider 的设置。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderSettingsInput {
    pub transcription: ProviderSettingsInput,
    pub minutes: ProviderSettingsInput,
}

/// 表示不泄露响应正文的连接检查结果。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResult {
    pub ok: bool,
    pub safe_message: String,
}

/// 标识一次设置操作所针对的服务，防止跨服务使用不相容预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderTarget {
    Transcription,
    Minutes,
}

/// 返回公开 Provider 配置以及两个 Key 是否已配置。
#[tauri::command]
pub fn get_public_settings(state: State<'_, AppState>) -> Result<PublicSettings, CommandError> {
    let mut settings = load_public_settings(&state)?;
    settings.transcription = migrate_public_provider(
        settings.transcription,
        ProviderTarget::Transcription,
        secret_configured(SecretKind::Transcription)?,
    );
    settings.minutes = migrate_public_provider(
        settings.minutes,
        ProviderTarget::Minutes,
        secret_configured(SecretKind::Minutes)?,
    );
    Ok(config::evaluate_settings_readiness(settings))
}

/// 校验并保存非秘密配置，将可选 Key 写入 Windows 凭据管理器。
#[tauri::command]
pub fn save_provider_settings(
    state: State<'_, AppState>,
    input: SaveProviderSettingsInput,
) -> Result<PublicSettings, CommandError> {
    let previous = load_public_settings(&state)?;
    let transcription_secret_exists = secret_configured(SecretKind::Transcription)?;
    let minutes_secret_exists = secret_configured(SecretKind::Minutes)?;
    let previous_transcription = migrate_public_provider(
        previous.transcription,
        ProviderTarget::Transcription,
        transcription_secret_exists,
    );
    let previous_minutes = migrate_public_provider(
        previous.minutes,
        ProviderTarget::Minutes,
        minutes_secret_exists,
    );
    let transcription = resolve_provider(
        &input.transcription,
        ProviderTarget::Transcription,
        previous_transcription.credential_preset_id,
        transcription_secret_exists || has_new_secret(&input.transcription),
    )?;
    let minutes = resolve_provider(
        &input.minutes,
        ProviderTarget::Minutes,
        previous_minutes.credential_preset_id,
        minutes_secret_exists || has_new_secret(&input.minutes),
    )?;
    if let Some(secret) = input
        .transcription
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        save_secret(SecretKind::Transcription, secret)?;
    }
    if let Some(secret) = input
        .minutes
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        save_secret(SecretKind::Minutes, secret)?;
    }
    let public = PublicSettings {
        transcription,
        minutes,
    };
    let serialized = serde_json::to_string(&public)
        .map_err(|_| CommandError::new("settings_invalid", "无法保存本地配置", false))?;
    state.repository.set_setting(SETTINGS_KEY, &serialized)?;
    Ok(public)
}

/// 删除指定 Provider 的系统凭据。
#[tauri::command]
pub fn delete_provider_secret(kind: String) -> Result<bool, CommandError> {
    let secret_kind = parse_secret_kind(&kind)?;
    secrets::delete_secret(secret_kind).map_err(|_| credential_error())?;
    Ok(true)
}

/// 检查 Mock 可用性；真实 Provider 在契约未验证前明确返回阻塞状态。
#[tauri::command]
pub fn test_provider_connection(
    state: State<'_, AppState>,
    target: String,
) -> Result<ProviderConnectionResult, CommandError> {
    let settings = get_public_settings(state)?;
    let provider = match target.as_str() {
        "transcription" => settings.transcription,
        "minutes" => settings.minutes,
        _ => {
            return Err(CommandError::new(
                "settings_invalid",
                "Provider 类型无效",
                false,
            ))
        }
    };
    if provider.kind == "mock" {
        return Ok(ProviderConnectionResult {
            ok: true,
            safe_message: "Mock Provider 可用".to_string(),
        });
    }
    if !provider.secret_configured {
        return Ok(ProviderConnectionResult {
            ok: false,
            safe_message: "请先保存 API Key".to_string(),
        });
    }
    Ok(ProviderConnectionResult {
        ok: false,
        safe_message: "真实 Provider 字段尚未完成最小音频验证，未发送网络请求".to_string(),
    })
}

/// 从本地设置读取公开配置，不存在时回退到可由环境变量覆盖的托管默认值。
fn load_public_settings(state: &AppState) -> Result<PublicSettings, CommandError> {
    state
        .repository
        .get_setting(SETTINGS_KEY)?
        .map(|value| serde_json::from_str::<PublicSettings>(&value))
        .transpose()
        .map_err(|_| CommandError::new("settings_invalid", "本地配置格式无效", false))
        .map(|value| value.unwrap_or_else(config::provider_settings_from_environment))
}

/// 将旧公开配置迁移到预设模型，并只在凭据绑定与当前预设一致时公开已配置状态。
fn migrate_public_provider(
    mut provider: PublicProviderConfig,
    target: ProviderTarget,
    secret_exists: bool,
) -> PublicProviderConfig {
    let legacy_without_preset = provider.preset_id.trim().is_empty();
    if legacy_without_preset {
        provider.preset_id = config::infer_preset_id(&provider);
        if provider.credential_preset_id.is_none() && secret_exists {
            provider.credential_preset_id = Some(provider.preset_id.clone());
        }
    }
    canonicalize_managed_provider(&mut provider, target);
    provider.secret_configured = secret_exists
        && provider.credential_preset_id.as_deref() == Some(provider.preset_id.as_str());
    provider
}

/// 按目标服务和预设可信解析保存输入，托管地址与类型不采信前端字段。
fn resolve_provider(
    input: &ProviderSettingsInput,
    target: ProviderTarget,
    previous_credential_preset_id: Option<String>,
    secret_exists_after_save: bool,
) -> Result<PublicProviderConfig, CommandError> {
    validate_limits(input)?;
    let preset_id = effective_preset_id(input);
    validate_preset_target(&preset_id, target)?;
    let credential_preset_id = if has_new_secret(input) {
        Some(preset_id.clone())
    } else {
        previous_credential_preset_id
    };
    let (kind, endpoint, model) = match preset_id.as_str() {
        config::PRESET_MOCK => ("mock".to_string(), String::new(), String::new()),
        config::PRESET_DASHSCOPE_FUNASR_CN => (
            "dashscope_funasr".to_string(),
            config::DASHSCOPE_FUNASR_CN_ENDPOINT.to_string(),
            managed_model(&input.model, &["fun-asr", "fun-asr-mtl"], "fun-asr")?,
        ),
        config::PRESET_DASHSCOPE_FUNASR_INTL => (
            "dashscope_funasr".to_string(),
            config::DASHSCOPE_FUNASR_INTL_ENDPOINT.to_string(),
            managed_model(&input.model, &["fun-asr", "fun-asr-mtl"], "fun-asr")?,
        ),
        config::PRESET_DEEPSEEK => (
            "openai_compatible".to_string(),
            config::DEEPSEEK_ENDPOINT.to_string(),
            managed_model(
                &input.model,
                &["deepseek-v4-flash", "deepseek-v4-pro"],
                "deepseek-v4-flash",
            )?,
        ),
        config::PRESET_CUSTOM_OPENAI => {
            validate_endpoint(&input.endpoint)?;
            let model = input.model.trim();
            if model.is_empty() {
                return Err(CommandError::new(
                    "settings_invalid",
                    "自定义 Provider 必须配置模型名",
                    false,
                ));
            }
            (
                "openai_compatible".to_string(),
                input.endpoint.trim().to_string(),
                model.to_string(),
            )
        }
        _ => unreachable!("预设已在目标校验中穷举"),
    };
    let secret_configured =
        secret_exists_after_save && credential_preset_id.as_deref() == Some(preset_id.as_str());
    Ok(config::evaluate_provider_readiness(PublicProviderConfig {
        preset_id,
        kind,
        endpoint,
        model,
        credential_preset_id,
        secret_configured,
        connect_timeout_ms: input.connect_timeout_ms,
        request_timeout_ms: input.request_timeout_ms,
        max_retries: input.max_retries,
        ready: false,
        readiness: crate::domain::ProviderReadiness::Incomplete,
        validation_message: String::new(),
    }))
}

/// 将托管预设恢复为后端固定字段，避免旧配置或本地篡改改变供应商地址。
fn canonicalize_managed_provider(provider: &mut PublicProviderConfig, target: ProviderTarget) {
    match (provider.preset_id.as_str(), target) {
        (config::PRESET_MOCK, _) => {
            provider.kind = "mock".to_string();
            provider.endpoint.clear();
            provider.model.clear();
        }
        (config::PRESET_DASHSCOPE_FUNASR_CN, ProviderTarget::Transcription) => {
            provider.kind = "dashscope_funasr".to_string();
            provider.endpoint = config::DASHSCOPE_FUNASR_CN_ENDPOINT.to_string();
            if !matches!(provider.model.as_str(), "fun-asr" | "fun-asr-mtl") {
                provider.model = "fun-asr".to_string();
            }
        }
        (config::PRESET_DASHSCOPE_FUNASR_INTL, ProviderTarget::Transcription) => {
            provider.kind = "dashscope_funasr".to_string();
            provider.endpoint = config::DASHSCOPE_FUNASR_INTL_ENDPOINT.to_string();
            if !matches!(provider.model.as_str(), "fun-asr" | "fun-asr-mtl") {
                provider.model = "fun-asr".to_string();
            }
        }
        (config::PRESET_DEEPSEEK, ProviderTarget::Minutes) => {
            provider.kind = "openai_compatible".to_string();
            provider.endpoint = config::DEEPSEEK_ENDPOINT.to_string();
            if !matches!(
                provider.model.as_str(),
                "deepseek-v4-flash" | "deepseek-v4-pro"
            ) {
                provider.model = "deepseek-v4-flash".to_string();
            }
        }
        (config::PRESET_CUSTOM_OPENAI, _) => {
            provider.kind = "openai_compatible".to_string();
        }
        _ => {
            provider.kind = "invalid".to_string();
            provider.endpoint.clear();
            provider.model.clear();
        }
    }
}

/// 兼容旧前端未发送 presetId 的输入，按类型和精确地址推断预设。
fn effective_preset_id(input: &ProviderSettingsInput) -> String {
    if !input.preset_id.trim().is_empty() {
        return input.preset_id.trim().to_string();
    }
    config::infer_preset_id(&PublicProviderConfig {
        preset_id: String::new(),
        kind: input.kind.clone(),
        endpoint: input.endpoint.clone(),
        model: input.model.clone(),
        credential_preset_id: None,
        secret_configured: false,
        connect_timeout_ms: input.connect_timeout_ms,
        request_timeout_ms: input.request_timeout_ms,
        max_retries: input.max_retries,
        ready: false,
        readiness: crate::domain::ProviderReadiness::Incomplete,
        validation_message: String::new(),
    })
}

/// 校验预设是否存在且允许用于当前目标服务。
fn validate_preset_target(preset_id: &str, target: ProviderTarget) -> Result<(), CommandError> {
    let supported = matches!(
        (preset_id, target),
        (config::PRESET_MOCK, _)
            | (
                config::PRESET_DASHSCOPE_FUNASR_CN,
                ProviderTarget::Transcription
            )
            | (
                config::PRESET_DASHSCOPE_FUNASR_INTL,
                ProviderTarget::Transcription
            )
            | (config::PRESET_DEEPSEEK, ProviderTarget::Minutes)
            | (config::PRESET_CUSTOM_OPENAI, _)
    );
    if supported {
        Ok(())
    } else {
        Err(CommandError::new(
            "settings_invalid",
            "Provider 预设与目标服务不兼容",
            false,
        ))
    }
}

/// 校验托管预设的模型白名单，空值使用该预设的默认模型。
fn managed_model(value: &str, allowed: &[&str], fallback: &str) -> Result<String, CommandError> {
    let model = value.trim();
    if model.is_empty() {
        return Ok(fallback.to_string());
    }
    if allowed.contains(&model) {
        Ok(model.to_string())
    } else {
        Err(CommandError::new(
            "settings_invalid",
            "所选模型不属于当前 Provider 预设",
            false,
        ))
    }
}

/// 返回保存输入是否携带新的非空 Key，仅用于更新凭据绑定，不读取秘密内容。
fn has_new_secret(input: &ProviderSettingsInput) -> bool {
    input
        .api_key
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

/// 校验超时与重试范围，避免异常值绕过前端限制。
fn validate_limits(input: &ProviderSettingsInput) -> Result<(), CommandError> {
    if !(1_000..=60_000).contains(&input.connect_timeout_ms)
        || !(5_000..=600_000).contains(&input.request_timeout_ms)
        || input.max_retries > 5
    {
        return Err(CommandError::new(
            "settings_invalid",
            "连接超时、请求超时或重试次数超出允许范围",
            false,
        ));
    }
    Ok(())
}

/// 仅接受 HTTPS，开发时允许本机回环 HTTP mock server。
fn validate_endpoint(value: &str) -> Result<(), CommandError> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| CommandError::new("settings_invalid", "API 地址格式无效", false))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CommandError::new(
            "settings_invalid",
            "API 地址不得包含凭据、查询参数或片段",
            false,
        ));
    }
    let localhost = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && localhost) {
        return Err(CommandError::new(
            "settings_invalid",
            "API 地址必须使用 HTTPS；仅本机 mock 可使用 HTTP",
            false,
        ));
    }
    Ok(())
}

/// 把前端公开字符串映射为固定秘密槽位。
fn parse_secret_kind(value: &str) -> Result<SecretKind, CommandError> {
    match value {
        "transcription" => Ok(SecretKind::Transcription),
        "minutes" => Ok(SecretKind::Minutes),
        _ => Err(CommandError::new("settings_invalid", "秘密类型无效", false)),
    }
}

/// 保存一个 Key，并隐藏系统错误细节。
fn save_secret(kind: SecretKind, secret: &str) -> Result<(), CommandError> {
    secrets::save_secret(kind, secret).map_err(|_| credential_error())
}

/// 查询秘密是否存在，并隐藏系统错误细节。
fn secret_configured(kind: SecretKind) -> Result<bool, CommandError> {
    secrets::secret_is_configured(kind).map_err(|_| credential_error())
}

/// 创建统一的凭据管理器安全错误。
fn credential_error() -> CommandError {
    CommandError::new(
        "credential_store_error",
        "无法访问 Windows 凭据管理器",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造有效的真实 Provider 配置用于纯校验测试。
    fn valid_input() -> ProviderSettingsInput {
        ProviderSettingsInput {
            preset_id: config::PRESET_CUSTOM_OPENAI.into(),
            kind: "openai_compatible".into(),
            endpoint: "https://api.example.test/v1".into(),
            model: "test-model".into(),
            api_key: None,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            max_retries: 2,
        }
    }

    /// 验证非 HTTPS 外部地址会被拒绝。
    #[test]
    fn rejects_insecure_remote_endpoint() {
        let mut input = valid_input();
        input.endpoint = "http://example.test/v1".into();
        assert!(resolve_provider(&input, ProviderTarget::Minutes, None, false).is_err());
    }

    /// 验证本机 HTTP mock server 被允许。
    #[test]
    fn accepts_local_mock_endpoint() {
        let mut input = valid_input();
        input.endpoint = "http://127.0.0.1:3000/v1".into();
        assert!(resolve_provider(&input, ProviderTarget::Minutes, None, false).is_ok());
    }

    /// 验证保存输入转换后会立即计算公开就绪状态且不会包含 Key。
    #[test]
    fn saved_public_provider_reports_readiness_without_secret_value() {
        let public = resolve_provider(
            &valid_input(),
            ProviderTarget::Minutes,
            Some(config::PRESET_CUSTOM_OPENAI.to_string()),
            true,
        )
        .expect("自定义 Provider 应通过校验");
        assert!(public.ready);
        assert!(public.secret_configured);
        assert_eq!(public.validation_message, "真实 Provider 配置已就绪");
    }

    /// 验证旧版公开配置缺少就绪字段时仍可读取，并可重新计算状态。
    #[test]
    fn legacy_public_provider_json_remains_readable() {
        let legacy = r#"{
            "kind":"openai_compatible",
            "endpoint":"https://api.example.test/v1",
            "model":"example-model",
            "secretConfigured":true,
            "connectTimeoutMs":10000,
            "requestTimeoutMs":60000,
            "maxRetries":2
        }"#;
        let parsed: PublicProviderConfig = serde_json::from_str(legacy).expect("旧配置应可读取");
        let migrated = migrate_public_provider(parsed, ProviderTarget::Minutes, true);
        assert_eq!(migrated.preset_id, config::PRESET_CUSTOM_OPENAI);
        assert_eq!(
            migrated.credential_preset_id.as_deref(),
            Some(config::PRESET_CUSTOM_OPENAI)
        );
        let evaluated = config::evaluate_provider_readiness(migrated);
        assert!(evaluated.ready);
    }

    /// 验证托管 FunASR 预设忽略伪造的客户端类型与地址。
    #[test]
    fn managed_funasr_endpoint_cannot_be_overridden() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_DASHSCOPE_FUNASR_CN.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model = "fun-asr-mtl".to_string();
        let public = resolve_provider(&input, ProviderTarget::Transcription, None, false)
            .expect("托管 FunASR 输入应使用固定地址");
        assert_eq!(public.kind, "dashscope_funasr");
        assert_eq!(public.endpoint, config::DASHSCOPE_FUNASR_CN_ENDPOINT);
    }

    /// 验证托管 DeepSeek 预设忽略伪造地址并使用默认模型。
    #[test]
    fn managed_deepseek_uses_fixed_endpoint_and_default_model() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_DEEPSEEK.to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model.clear();
        let public = resolve_provider(&input, ProviderTarget::Minutes, None, false)
            .expect("托管 DeepSeek 输入应使用固定值");
        assert_eq!(public.endpoint, config::DEEPSEEK_ENDPOINT);
        assert_eq!(public.model, "deepseek-v4-flash");
    }

    /// 验证托管预设拒绝白名单外的模型。
    #[test]
    fn managed_presets_reject_unknown_models() {
        let mut funasr = valid_input();
        funasr.preset_id = config::PRESET_DASHSCOPE_FUNASR_CN.to_string();
        funasr.model = "unknown-asr".to_string();
        assert!(resolve_provider(&funasr, ProviderTarget::Transcription, None, false).is_err());

        let mut deepseek = valid_input();
        deepseek.preset_id = config::PRESET_DEEPSEEK.to_string();
        deepseek.model = "unknown-llm".to_string();
        assert!(resolve_provider(&deepseek, ProviderTarget::Minutes, None, false).is_err());
    }

    /// 验证切换预设但未输入新 Key 时不会静默复用旧预设凭据。
    #[test]
    fn switching_preset_does_not_reuse_existing_credential() {
        let input = valid_input();
        let public = resolve_provider(
            &input,
            ProviderTarget::Minutes,
            Some(config::PRESET_DEEPSEEK.to_string()),
            true,
        )
        .expect("自定义 Provider 输入应通过字段校验");
        assert_eq!(
            public.credential_preset_id.as_deref(),
            Some(config::PRESET_DEEPSEEK)
        );
        assert!(!public.secret_configured);
        assert!(!public.ready);
    }

    /// 验证旧配置按精确托管地址推断预设，近似地址不会被误判为托管服务。
    #[test]
    fn legacy_preset_inference_requires_exact_managed_endpoint() {
        let mut managed = PublicProviderConfig {
            preset_id: String::new(),
            kind: "openai_compatible".to_string(),
            endpoint: config::DEEPSEEK_ENDPOINT.to_string(),
            model: "deepseek-v4-flash".to_string(),
            credential_preset_id: None,
            secret_configured: true,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            max_retries: 2,
            ready: false,
            readiness: crate::domain::ProviderReadiness::Incomplete,
            validation_message: String::new(),
        };
        let migrated = migrate_public_provider(managed.clone(), ProviderTarget::Minutes, true);
        assert_eq!(migrated.preset_id, config::PRESET_DEEPSEEK);

        managed.endpoint.push('/');
        let custom = migrate_public_provider(managed, ProviderTarget::Minutes, true);
        assert_eq!(custom.preset_id, config::PRESET_CUSTOM_OPENAI);
    }
}
