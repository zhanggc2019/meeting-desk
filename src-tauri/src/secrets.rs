const SERVICE_NAME: &str = "com.internal.meetingdesk";

/// 表示 Windows 凭据管理器中的两个受控秘密槽位。
#[derive(Debug, Clone, Copy)]
pub enum SecretKind {
    Transcription,
    Minutes,
}

impl SecretKind {
    /// 返回稳定且不包含秘密的凭据用户名。
    fn account_name(self) -> &'static str {
        match self {
            Self::Transcription => "transcription-api-key",
            Self::Minutes => "minutes-api-key",
        }
    }

    /// 返回可选的环境变量秘密来源。
    fn environment_name(self) -> &'static str {
        match self {
            Self::Transcription => "MEETING_DESK_ASR_API_KEY",
            Self::Minutes => "MEETING_DESK_LLM_API_KEY",
        }
    }
}

/// Windows 凭据管理器访问失败时返回的安全错误。
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("无法访问 Windows 凭据管理器")]
    CredentialStore,
}

/// 使用系统凭据管理器保存 API Key，秘密值不会进入 SQLite。
pub fn save_secret(kind: SecretKind, secret: &str) -> Result<(), SecretError> {
    if secret.trim().is_empty() {
        return delete_secret(kind);
    }
    keyring::Entry::new(SERVICE_NAME, kind.account_name())
        .map_err(|_| SecretError::CredentialStore)?
        .set_password(secret)
        .map_err(|_| SecretError::CredentialStore)
}

/// 从系统凭据管理器读取 API Key，仅供 Rust Provider 调用。
pub fn read_secret(kind: SecretKind) -> Result<Option<String>, SecretError> {
    let entry = keyring::Entry::new(SERVICE_NAME, kind.account_name())
        .map_err(|_| SecretError::CredentialStore)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(std::env::var(kind.environment_name())
            .ok()
            .filter(|value| !value.trim().is_empty())),
        Err(_) => Err(SecretError::CredentialStore),
    }
}

/// 删除指定秘密；不存在时视为成功。
pub fn delete_secret(kind: SecretKind) -> Result<(), SecretError> {
    let entry = keyring::Entry::new(SERVICE_NAME, kind.account_name())
        .map_err(|_| SecretError::CredentialStore)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(SecretError::CredentialStore),
    }
}

/// 仅返回秘密是否存在，绝不返回秘密值。
pub fn secret_is_configured(kind: SecretKind) -> Result<bool, SecretError> {
    read_secret(kind).map(|value| value.is_some())
}
