use std::{path::Path, time::Duration};

use reqwest::{redirect::Policy, StatusCode};
use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::AppState;
use crate::commands::CommandError;
use crate::config;
use crate::domain::{PublicProviderConfig, PublicSettings};
use crate::providers::LocalFunAsrProvider;
use crate::secrets::{self, SecretKind};

const SETTINGS_KEY: &str = "provider_settings_v1";

/// 接收单个 Provider 的配置；Key 只用于写入系统凭据管理器。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsInput {
    #[serde(default)]
    pub preset_id: String,
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub local_model_path: Option<String>,
    pub api_key: Option<String>,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub max_retries: u32,
}

/// 接收转写和纪要两个独立 Provider 的设置。
#[derive(Deserialize)]
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
    load_evaluated_settings(state.inner())
}

/// 打开系统目录选择器，并只返回通过文件完整性校验的本地模型路径。
#[tauri::command]
pub fn select_local_model_directory(app: AppHandle) -> Result<Option<String>, CommandError> {
    let Some(selected) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|_| {
        CommandError::new("local_model_path_invalid", "无法读取所选模型目录", false)
    })?;
    Ok(Some(validate_local_model_path(Some(
        path.to_string_lossy().as_ref(),
    ))?))
}

/// 供后端命令复用的公开设置读取入口，会迁移旧配置并重新计算密钥就绪状态。
pub(crate) fn load_evaluated_settings(state: &AppState) -> Result<PublicSettings, CommandError> {
    let mut settings = load_public_settings(state)?;
    settings.transcription = migrate_public_provider(
        settings.transcription,
        ProviderTarget::Transcription,
        legacy_secret_configured(SecretKind::Transcription)?,
    );
    migrate_bound_legacy_secret(&settings.transcription, SecretKind::Transcription)?;
    settings.transcription =
        refresh_secret_status(settings.transcription, SecretKind::Transcription)?;
    settings.minutes = migrate_public_provider(
        settings.minutes,
        ProviderTarget::Minutes,
        legacy_secret_configured(SecretKind::Minutes)?,
    );
    migrate_bound_legacy_secret(&settings.minutes, SecretKind::Minutes)?;
    settings.minutes = refresh_secret_status(settings.minutes, SecretKind::Minutes)?;
    Ok(config::evaluate_settings_readiness(settings))
}

/// 校验并保存非秘密配置，将可选 Key 写入 Windows 凭据管理器。
#[tauri::command]
pub fn save_provider_settings(
    state: State<'_, AppState>,
    input: SaveProviderSettingsInput,
) -> Result<PublicSettings, CommandError> {
    let previous = load_evaluated_settings(state.inner())?;
    let transcription = resolve_provider(
        &input.transcription,
        ProviderTarget::Transcription,
        previous.transcription.credential_preset_id,
        previous.transcription.secret_configured || has_new_secret(&input.transcription),
    )?;
    let minutes = resolve_provider(
        &input.minutes,
        ProviderTarget::Minutes,
        previous.minutes.credential_preset_id,
        previous.minutes.secret_configured || has_new_secret(&input.minutes),
    )?;
    if let Some(secret) = input
        .minutes
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        save_secret(SecretKind::Minutes, &minutes, secret)?;
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
pub fn delete_provider_secret(
    state: State<'_, AppState>,
    kind: String,
) -> Result<bool, CommandError> {
    let secret_kind = parse_secret_kind(&kind)?;
    let settings = load_evaluated_settings(state.inner())?;
    let provider = match secret_kind {
        SecretKind::Transcription => settings.transcription,
        SecretKind::Minutes => settings.minutes,
    };
    let binding_id = config::credential_binding_id(&provider);
    secrets::delete_secret_for_binding(secret_kind, &binding_id).map_err(|_| credential_error())?;
    Ok(true)
}

/// 使用不含会议正文的无副作用请求检查服务地址可达性和常见认证错误。
#[tauri::command]
pub async fn test_provider_connection(
    state: State<'_, AppState>,
    target: String,
) -> Result<ProviderConnectionResult, CommandError> {
    let settings = load_evaluated_settings(state.inner())?;
    let (provider, secret_kind) = match target.as_str() {
        "transcription" => (settings.transcription, SecretKind::Transcription),
        "minutes" => (settings.minutes, SecretKind::Minutes),
        _ => {
            return Err(CommandError::new(
                "settings_invalid",
                "Provider 类型无效",
                false,
            ))
        }
    };
    if provider.preset_id == config::PRESET_LOCAL_FUNASR {
        let local = LocalFunAsrProvider::discover_with_model_directory(
            provider.model.clone(),
            (!provider.local_model_path.trim().is_empty())
                .then(|| provider.local_model_path.clone().into()),
        );
        return Ok(match local.check_runtime(Duration::from_secs(120)).await {
            Ok(()) => ProviderConnectionResult {
                ok: true,
                safe_message: "本地 SenseVoiceSmall、FSMN-VAD 模型与 FunASR 运行环境可用"
                    .to_string(),
            },
            Err(error) => ProviderConnectionResult {
                ok: false,
                safe_message: error.safe_message,
            },
        });
    }
    let binding_id = provider.credential_preset_id.as_deref().ok_or_else(|| {
        CommandError::new(
            "provider_not_configured",
            "请先保存当前 Provider 的 API Key",
            false,
        )
    })?;
    let api_key = secrets::read_secret_for_binding(secret_kind, binding_id)
        .map_err(|_| credential_error())?
        .filter(|value| !value.trim().is_empty());
    let Some(api_key) = api_key.filter(|_| provider.secret_configured) else {
        return Ok(ProviderConnectionResult {
            ok: false,
            safe_message: "请先保存 API Key".to_string(),
        });
    };
    probe_provider_endpoint(&provider, &api_key).await
}

/// 对已校验地址发起不携带业务正文的 GET，并禁止重定向与响应正文读取。
async fn probe_provider_endpoint(
    provider: &PublicProviderConfig,
    api_key: &str,
) -> Result<ProviderConnectionResult, CommandError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(provider.connect_timeout_ms))
        .timeout(Duration::from_millis(
            provider.request_timeout_ms.min(30_000),
        ))
        .redirect(Policy::none())
        .build()
        .map_err(|_| CommandError::new("connection_test_failed", "无法创建连接测试", true))?;
    let request = client.get(&provider.endpoint);
    let request = if provider.kind == "volcengine_asr" {
        request.header("X-Api-Key", api_key)
    } else {
        request.bearer_auth(api_key)
    };
    let response = request.send().await.map_err(map_connection_probe_error)?;
    Ok(classify_probe_status(response.status()))
}

/// 将连接探测网络错误归类为不包含地址、密钥或远端正文的安全提示。
fn map_connection_probe_error(error: reqwest::Error) -> CommandError {
    let message = if error.is_timeout() {
        "连接测试超时，请检查网络、代理或服务地址"
    } else if error.is_connect() {
        "无法连接服务，请检查网络、代理或服务地址"
    } else {
        "连接测试失败，请稍后重试"
    };
    CommandError::new("connection_test_failed", message, true)
}

/// 只根据 HTTP 状态分类可达性，不读取或展示远端响应正文。
fn classify_probe_status(status: StatusCode) -> ProviderConnectionResult {
    let (ok, safe_message) = if status.is_success() || status == StatusCode::METHOD_NOT_ALLOWED {
        (
            true,
            "服务地址可达；本测试不发送音频或提示词，密钥、模型和业务字段将在实际处理时进一步验证",
        )
    } else {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                (false, "服务可达，但 API Key 未通过认证")
            }
            StatusCode::NOT_FOUND => (false, "服务可达，但接口路径不存在，请检查服务地址"),
            StatusCode::TOO_MANY_REQUESTS => (false, "服务可达，但当前触发限流，请稍后重试"),
            value if value.is_server_error() => (false, "服务可达，但服务端暂时不可用"),
            value if value.is_redirection() => (false, "服务返回重定向，出于安全原因未继续访问"),
            _ => (false, "服务已响应，但当前状态无法确认配置有效"),
        }
    };
    ProviderConnectionResult {
        ok,
        safe_message: safe_message.to_string(),
    }
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
    legacy_secret_exists: bool,
) -> PublicProviderConfig {
    if target == ProviderTarget::Transcription {
        provider.preset_id = config::PRESET_LOCAL_FUNASR.to_string();
        provider.kind = "local_funasr".to_string();
        provider.endpoint = config::LOCAL_FUNASR_ENDPOINT.to_string();
        provider.model = config::LOCAL_FUNASR_MODEL.to_string();
        provider.credential_preset_id = None;
        provider.secret_configured = false;
        provider.request_timeout_ms = provider
            .request_timeout_ms
            .max(config::LOCAL_FUNASR_DEFAULT_TIMEOUT_MS);
        provider.max_retries = 0;
        return provider;
    }
    let was_legacy_mock = provider.preset_id == config::PRESET_MOCK || provider.kind == "mock";
    if was_legacy_mock {
        provider.preset_id = match target {
            ProviderTarget::Transcription => config::PRESET_LOCAL_FUNASR,
            ProviderTarget::Minutes => config::PRESET_DEEPSEEK,
        }
        .to_string();
        provider.credential_preset_id = None;
        provider.secret_configured = false;
    }
    let legacy_without_preset = provider.preset_id.trim().is_empty();
    if legacy_without_preset {
        provider.preset_id = config::infer_preset_id(&provider);
    }
    let stored_binding = provider.credential_preset_id.clone();
    canonicalize_managed_provider(&mut provider, target);
    let expected_binding = config::credential_binding_id(&provider);
    if stored_binding.as_deref() == Some(provider.preset_id.as_str())
        || (!was_legacy_mock && stored_binding.is_none() && legacy_secret_exists)
    {
        provider.credential_preset_id = Some(expected_binding);
    }
    provider.secret_configured = false;
    provider
}

/// Recomputes secret readiness from the current provider-specific credential binding.
fn refresh_secret_status(
    mut provider: PublicProviderConfig,
    kind: SecretKind,
) -> Result<PublicProviderConfig, CommandError> {
    let expected_binding = config::credential_binding_id(&provider);
    let binding_matches =
        provider.credential_preset_id.as_deref() == Some(expected_binding.as_str());
    provider.secret_configured = binding_matches
        && secrets::secret_is_configured_for_binding(kind, &expected_binding)
            .map_err(|_| credential_error())?;
    Ok(provider)
}

/// Migrates only when the public config is already bound to this exact provider identity.
fn migrate_bound_legacy_secret(
    provider: &PublicProviderConfig,
    kind: SecretKind,
) -> Result<(), CommandError> {
    let expected_binding = config::credential_binding_id(provider);
    if provider.credential_preset_id.as_deref() == Some(expected_binding.as_str()) {
        secrets::migrate_legacy_secret_for_binding(kind, &expected_binding)
            .map_err(|_| credential_error())?;
    }
    Ok(())
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
    let (kind, endpoint, model) = match preset_id.as_str() {
        config::PRESET_LOCAL_FUNASR => (
            "local_funasr".to_string(),
            config::LOCAL_FUNASR_ENDPOINT.to_string(),
            managed_model(
                &input.model,
                &[config::LOCAL_FUNASR_MODEL],
                config::LOCAL_FUNASR_MODEL,
            )?,
        ),
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
        config::PRESET_XIAOMI_MIMO_ASR => (
            "xiaomi_mimo".to_string(),
            config::XIAOMI_MIMO_ASR_ENDPOINT.to_string(),
            managed_model(&input.model, &["mimo-v2.5-asr"], "mimo-v2.5-asr")?,
        ),
        config::PRESET_VOLCENGINE_ASR_FLASH => (
            "volcengine_asr".to_string(),
            config::VOLCENGINE_ASR_FLASH_ENDPOINT.to_string(),
            managed_model(&input.model, &["bigmodel"], "bigmodel")?,
        ),
        config::PRESET_XIAOMI_MIMO_LLM => (
            "openai_compatible".to_string(),
            config::XIAOMI_MIMO_LLM_ENDPOINT.to_string(),
            managed_model(&input.model, &["mimo-v2.5", "mimo-v2.5-pro"], "mimo-v2.5")?,
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
        config::PRESET_ALIYUN_BAILIAN => (
            "openai_compatible".to_string(),
            config::ALIYUN_BAILIAN_ENDPOINT.to_string(),
            managed_model(
                &input.model,
                &["qwen-plus", "qwen-flash", "qwen-max"],
                "qwen-plus",
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
    let local_model_path = if preset_id == config::PRESET_LOCAL_FUNASR {
        validate_local_model_path(input.local_model_path.as_deref())?
    } else {
        String::new()
    };
    let mut provider = PublicProviderConfig {
        preset_id,
        kind,
        endpoint,
        model,
        local_model_path,
        credential_preset_id: None,
        secret_configured: false,
        connect_timeout_ms: input.connect_timeout_ms,
        request_timeout_ms: input.request_timeout_ms,
        max_retries: input.max_retries,
        ready: false,
        readiness: crate::domain::ProviderReadiness::Incomplete,
        validation_message: String::new(),
    };
    let expected_binding = config::credential_binding_id(&provider);
    provider.credential_preset_id = if provider.preset_id == config::PRESET_LOCAL_FUNASR {
        None
    } else if has_new_secret(input) {
        Some(expected_binding.clone())
    } else {
        previous_credential_preset_id
    };
    let secret_configured = secret_exists_after_save
        && provider.credential_preset_id.as_deref() == Some(expected_binding.as_str());
    provider.secret_configured = secret_configured;
    Ok(config::evaluate_provider_readiness(provider))
}

/// 将托管预设恢复为后端固定字段，避免旧配置或本地篡改改变供应商地址。
fn canonicalize_managed_provider(provider: &mut PublicProviderConfig, target: ProviderTarget) {
    match (provider.preset_id.as_str(), target) {
        (config::PRESET_LOCAL_FUNASR, ProviderTarget::Transcription) => {
            provider.kind = "local_funasr".to_string();
            provider.endpoint = config::LOCAL_FUNASR_ENDPOINT.to_string();
            provider.model = config::LOCAL_FUNASR_MODEL.to_string();
            provider.credential_preset_id = None;
            provider.secret_configured = false;
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
        (config::PRESET_XIAOMI_MIMO_ASR, ProviderTarget::Transcription) => {
            provider.kind = "xiaomi_mimo".to_string();
            provider.endpoint = config::XIAOMI_MIMO_ASR_ENDPOINT.to_string();
            provider.model = "mimo-v2.5-asr".to_string();
        }
        (config::PRESET_VOLCENGINE_ASR_FLASH, ProviderTarget::Transcription) => {
            provider.kind = "volcengine_asr".to_string();
            provider.endpoint = config::VOLCENGINE_ASR_FLASH_ENDPOINT.to_string();
            provider.model = "bigmodel".to_string();
        }
        (config::PRESET_XIAOMI_MIMO_LLM, ProviderTarget::Minutes) => {
            provider.kind = "openai_compatible".to_string();
            provider.endpoint = config::XIAOMI_MIMO_LLM_ENDPOINT.to_string();
            if !matches!(provider.model.as_str(), "mimo-v2.5" | "mimo-v2.5-pro") {
                provider.model = "mimo-v2.5".to_string();
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
        (config::PRESET_ALIYUN_BAILIAN, ProviderTarget::Minutes) => {
            provider.kind = "openai_compatible".to_string();
            provider.endpoint = config::ALIYUN_BAILIAN_ENDPOINT.to_string();
            if !matches!(
                provider.model.as_str(),
                "qwen-plus" | "qwen-flash" | "qwen-max"
            ) {
                provider.model = "qwen-plus".to_string();
            }
        }
        (config::PRESET_CUSTOM_OPENAI, ProviderTarget::Minutes) => {
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
        local_model_path: input.local_model_path.clone().unwrap_or_default(),
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

/// 校验用户选择的是本机绝对目录，且模型关键文件均存在并非空文件。
fn validate_local_model_path(value: Option<&str>) -> Result<String, CommandError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(String::new());
    };
    let path = Path::new(value);
    if !path.is_absolute() || value.starts_with(r"\\") || value.starts_with("//") {
        return Err(CommandError::new(
            "local_model_path_invalid",
            "请选择本机磁盘上的 SenseVoiceSmall 模型目录",
            false,
        ));
    }
    LocalFunAsrProvider::validate_model_directory(path).map_err(|error| {
        CommandError::new("local_model_path_invalid", error.safe_message, false)
    })?;
    Ok(value.to_string())
}

/// 校验预设是否存在且允许用于当前目标服务。
fn validate_preset_target(preset_id: &str, target: ProviderTarget) -> Result<(), CommandError> {
    let supported = matches!(
        (preset_id, target),
        (config::PRESET_LOCAL_FUNASR, ProviderTarget::Transcription)
            | (config::PRESET_DEEPSEEK, ProviderTarget::Minutes)
            | (config::PRESET_XIAOMI_MIMO_LLM, ProviderTarget::Minutes)
            | (config::PRESET_ALIYUN_BAILIAN, ProviderTarget::Minutes)
            | (config::PRESET_CUSTOM_OPENAI, ProviderTarget::Minutes)
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
    let max_request_timeout =
        if input.preset_id == config::PRESET_LOCAL_FUNASR || input.kind == "local_funasr" {
            config::LOCAL_FUNASR_MAX_TIMEOUT_MS
        } else {
            600_000
        };
    if !(1_000..=60_000).contains(&input.connect_timeout_ms)
        || !(5_000..=max_request_timeout).contains(&input.request_timeout_ms)
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

/// 仅接受 HTTPS，开发与自建服务调试时允许本机回环 HTTP 地址。
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
            "API 地址必须使用 HTTPS；仅本机回环服务可使用 HTTP",
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
fn save_secret(
    kind: SecretKind,
    provider: &PublicProviderConfig,
    secret: &str,
) -> Result<(), CommandError> {
    let binding_id = provider
        .credential_preset_id
        .as_deref()
        .ok_or_else(|| CommandError::new("settings_invalid", "Provider 凭据绑定无效", false))?;
    secrets::save_secret_for_binding(kind, binding_id, secret).map_err(|_| credential_error())
}

/// 查询秘密是否存在，并隐藏系统错误细节。
fn legacy_secret_configured(kind: SecretKind) -> Result<bool, CommandError> {
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
            local_model_path: None,
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

    /// 验证本机 HTTP 调试服务被允许。
    #[test]
    fn accepts_local_development_endpoint() {
        let mut input = valid_input();
        input.endpoint = "http://127.0.0.1:3000/v1".into();
        assert!(resolve_provider(&input, ProviderTarget::Minutes, None, false).is_ok());
    }

    /// 验证无正文探测只把成功状态和方法不允许视为服务地址可达。
    #[test]
    fn classifies_safe_connection_probe_statuses() {
        assert!(classify_probe_status(StatusCode::OK).ok);
        assert!(classify_probe_status(StatusCode::ACCEPTED).ok);
        assert!(classify_probe_status(StatusCode::METHOD_NOT_ALLOWED).ok);
        assert!(!classify_probe_status(StatusCode::UNAUTHORIZED).ok);
        assert!(!classify_probe_status(StatusCode::NOT_FOUND).ok);
        assert!(!classify_probe_status(StatusCode::TOO_MANY_REQUESTS).ok);
        assert!(!classify_probe_status(StatusCode::BAD_GATEWAY).ok);
    }

    /// 验证保存输入转换后会立即计算公开就绪状态且不会包含 Key。
    #[test]
    fn saved_public_provider_reports_readiness_without_secret_value() {
        let public = resolve_provider(
            &valid_input(),
            ProviderTarget::Minutes,
            Some(config::credential_binding_id(&PublicProviderConfig {
                preset_id: config::PRESET_CUSTOM_OPENAI.to_string(),
                kind: "openai_compatible".to_string(),
                endpoint: "https://api.example.test/v1".to_string(),
                model: "test-model".to_string(),
                local_model_path: String::new(),
                credential_preset_id: None,
                secret_configured: false,
                connect_timeout_ms: 10_000,
                request_timeout_ms: 60_000,
                max_retries: 2,
                ready: false,
                readiness: crate::domain::ProviderReadiness::Incomplete,
                validation_message: String::new(),
            })),
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
        let mut migrated = migrate_public_provider(parsed, ProviderTarget::Minutes, true);
        assert_eq!(migrated.preset_id, config::PRESET_CUSTOM_OPENAI);
        let expected_binding = config::credential_binding_id(&migrated);
        assert_eq!(
            migrated.credential_preset_id.as_deref(),
            Some(expected_binding.as_str())
        );
        migrated.secret_configured = true;
        let evaluated = config::evaluate_provider_readiness(migrated);
        assert!(evaluated.ready);
    }

    /// 验证尚未接通异步上传链路的 FunASR 预设不能保存为正式配置。
    #[test]
    fn rejects_unimplemented_funasr_preset() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_DASHSCOPE_FUNASR_CN.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model = "fun-asr-mtl".to_string();
        assert!(resolve_provider(&input, ProviderTarget::Transcription, None, false).is_err());
    }

    /// 验证本地 FunASR 固定模型边界、无需凭据并允许长会议超时。
    #[test]
    fn local_funasr_uses_fixed_fields_without_credential() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_LOCAL_FUNASR.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model.clear();
        input.request_timeout_ms = config::LOCAL_FUNASR_DEFAULT_TIMEOUT_MS;
        let public = resolve_provider(&input, ProviderTarget::Transcription, None, false)
            .expect("local FunASR settings");
        assert_eq!(public.kind, "local_funasr");
        assert_eq!(public.endpoint, config::LOCAL_FUNASR_ENDPOINT);
        assert_eq!(public.model, config::LOCAL_FUNASR_MODEL);
        assert!(public.credential_preset_id.is_none());
        assert!(!public.secret_configured);
        assert!(public.ready);
    }

    /// 验证本地 FunASR 会校验并保存用户选择的模型目录。
    #[test]
    fn local_funasr_persists_selected_model_directory() {
        let fixture = tempfile::tempdir().expect("fixture");
        let model_dir = fixture.path().join("SenseVoiceSmall");
        std::fs::create_dir_all(&model_dir).expect("model directory");
        for file_name in ["config.yaml", "model.pt", "tokens.json"] {
            std::fs::write(model_dir.join(file_name), b"fixture").expect("model file");
        }
        let vad_model_dir = fixture.path().join("fsmn-vad");
        std::fs::create_dir_all(&vad_model_dir).expect("VAD model directory");
        for file_name in ["config.yaml", "configuration.json", "model.pt", "am.mvn"] {
            std::fs::write(vad_model_dir.join(file_name), b"fixture").expect("VAD model file");
        }
        let mut input = valid_input();
        input.preset_id = config::PRESET_LOCAL_FUNASR.to_string();
        input.kind = "local_funasr".to_string();
        input.model = config::LOCAL_FUNASR_MODEL.to_string();
        input.local_model_path = Some(model_dir.to_string_lossy().into_owned());
        input.request_timeout_ms = config::LOCAL_FUNASR_DEFAULT_TIMEOUT_MS;

        let public = resolve_provider(&input, ProviderTarget::Transcription, None, false)
            .expect("selected model directory");

        assert_eq!(
            public.local_model_path,
            model_dir.to_string_lossy().as_ref()
        );
    }

    /// 验证已停用的 Xiaomi MiMo 在线 ASR 预设不能保存。
    #[test]
    fn rejects_disabled_xiaomi_mimo_asr() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_XIAOMI_MIMO_ASR.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model.clear();
        assert!(resolve_provider(&input, ProviderTarget::Transcription, None, false).is_err());
    }

    /// 验证已停用的火山引擎在线 ASR 预设不能保存。
    #[test]
    fn rejects_disabled_volcengine_asr() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_VOLCENGINE_ASR_FLASH.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model.clear();
        assert!(resolve_provider(&input, ProviderTarget::Transcription, None, false).is_err());
    }

    /// 验证 MiMo 大模型预设固定官方地址并允许两个公开文本模型。
    #[test]
    fn managed_xiaomi_mimo_llm_uses_fixed_contract() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_XIAOMI_MIMO_LLM.to_string();
        input.kind = "untrusted_kind".to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model = "mimo-v2.5-pro".to_string();
        let public = resolve_provider(&input, ProviderTarget::Minutes, None, false)
            .expect("MiMo LLM 托管预设应使用固定字段");
        assert_eq!(public.kind, "openai_compatible");
        assert_eq!(public.endpoint, config::XIAOMI_MIMO_LLM_ENDPOINT);
        assert_eq!(public.model, "mimo-v2.5-pro");
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

    /// 验证托管百炼预设固定地址并使用稳定的默认模型。
    #[test]
    fn managed_aliyun_bailian_uses_fixed_endpoint_and_default_model() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_ALIYUN_BAILIAN.to_string();
        input.endpoint = "https://attacker.example.test/collect".to_string();
        input.model.clear();
        let public = resolve_provider(&input, ProviderTarget::Minutes, None, false)
            .expect("托管百炼输入应使用固定值");
        assert_eq!(public.endpoint, config::ALIYUN_BAILIAN_ENDPOINT);
        assert_eq!(public.model, "qwen-plus");
    }

    /// 验证托管预设拒绝白名单外的模型。
    #[test]
    fn managed_presets_reject_unknown_models() {
        let mut deepseek = valid_input();
        deepseek.preset_id = config::PRESET_DEEPSEEK.to_string();
        deepseek.model = "unknown-llm".to_string();
        assert!(resolve_provider(&deepseek, ProviderTarget::Minutes, None, false).is_err());

        let mut mimo_llm = valid_input();
        mimo_llm.preset_id = config::PRESET_XIAOMI_MIMO_LLM.to_string();
        mimo_llm.model = "unknown-mimo".to_string();
        assert!(resolve_provider(&mimo_llm, ProviderTarget::Minutes, None, false).is_err());

        let mut bailian = valid_input();
        bailian.preset_id = config::PRESET_ALIYUN_BAILIAN.to_string();
        bailian.model = "unknown-qwen".to_string();
        assert!(resolve_provider(&bailian, ProviderTarget::Minutes, None, false).is_err());

        let mut mimo = valid_input();
        mimo.preset_id = config::PRESET_XIAOMI_MIMO_ASR.to_string();
        mimo.model = "unknown-asr".to_string();
        assert!(resolve_provider(&mimo, ProviderTarget::Transcription, None, false).is_err());

        let mut volc = valid_input();
        volc.preset_id = config::PRESET_VOLCENGINE_ASR_FLASH.to_string();
        volc.model = "unknown-asr".to_string();
        assert!(resolve_provider(&volc, ProviderTarget::Transcription, None, false).is_err());
    }

    /// 验证正式设置接口不再接受旧版 Mock 预设。
    #[test]
    fn rejects_legacy_mock_preset_from_settings_input() {
        let mut input = valid_input();
        input.preset_id = config::PRESET_MOCK.to_string();
        assert!(resolve_provider(&input, ProviderTarget::Minutes, None, false).is_err());
    }

    /// 验证旧版 Mock 配置不会把遗留真实凭据绑定到新默认 Provider。
    #[test]
    fn legacy_mock_migration_does_not_rebind_legacy_credential() {
        let legacy = PublicProviderConfig {
            preset_id: config::PRESET_MOCK.to_string(),
            kind: "mock".to_string(),
            credential_preset_id: Some(config::PRESET_MOCK.to_string()),
            secret_configured: true,
            ..PublicProviderConfig::default()
        };

        let migrated = migrate_public_provider(legacy, ProviderTarget::Minutes, true);

        assert_eq!(migrated.preset_id, config::PRESET_DEEPSEEK);
        assert!(migrated.credential_preset_id.is_none());
        assert!(!migrated.secret_configured);
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
            local_model_path: String::new(),
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

    /// 验证旧版在线或自定义 ASR 配置会迁移为本地 SenseVoiceSmall。
    #[test]
    fn legacy_custom_transcription_migrates_to_local_funasr() {
        let custom = PublicProviderConfig {
            preset_id: config::PRESET_CUSTOM_OPENAI.to_string(),
            kind: "openai_compatible".to_string(),
            endpoint: "https://asr.example.test/v1".to_string(),
            model: "custom-asr".to_string(),
            local_model_path: String::new(),
            credential_preset_id: None,
            secret_configured: true,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            max_retries: 2,
            ready: true,
            readiness: crate::domain::ProviderReadiness::Ready,
            validation_message: String::new(),
        };

        let migrated = migrate_public_provider(custom, ProviderTarget::Transcription, true);
        let evaluated = config::evaluate_provider_readiness(migrated);

        assert_eq!(evaluated.preset_id, config::PRESET_LOCAL_FUNASR);
        assert_eq!(evaluated.kind, "local_funasr");
        assert_eq!(evaluated.endpoint, config::LOCAL_FUNASR_ENDPOINT);
        assert_eq!(evaluated.model, config::LOCAL_FUNASR_MODEL);
        assert!(!evaluated.secret_configured);
        assert!(evaluated.ready);
    }
}
