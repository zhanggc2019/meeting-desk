use sha2::{Digest, Sha256};

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
    read_account(kind.account_name()).map(|value| {
        if value.is_some() {
            value
        } else {
            read_environment_secret(kind)
        }
    })
}

/// Saves a credential in a provider-binding-specific slot so failed config writes cannot misroute it.
pub fn save_secret_for_binding(
    kind: SecretKind,
    binding_id: &str,
    secret: &str,
) -> Result<(), SecretError> {
    let account = scoped_account_name(kind, binding_id);
    if secret.trim().is_empty() {
        return delete_account(&account);
    }
    keyring::Entry::new(SERVICE_NAME, &account)
        .map_err(|_| SecretError::CredentialStore)?
        .set_password(secret)
        .map_err(|_| SecretError::CredentialStore)
}

/// Reads only the exact scoped credential, with an explicit process environment fallback.
pub fn read_secret_for_binding(
    kind: SecretKind,
    binding_id: &str,
) -> Result<Option<String>, SecretError> {
    let account = scoped_account_name(kind, binding_id);
    match read_account(&account)? {
        Some(value) => Ok(Some(value)),
        None => Ok(read_environment_secret(kind)),
    }
}

/// Moves a legacy target credential into the current provider binding exactly once.
pub fn migrate_legacy_secret_for_binding(
    kind: SecretKind,
    binding_id: &str,
) -> Result<(), SecretError> {
    let scoped_account = scoped_account_name(kind, binding_id);
    if read_account(&scoped_account)?.is_some() {
        return delete_account(kind.account_name());
    }
    let Some(secret) = read_account(kind.account_name())? else {
        return Ok(());
    };
    save_secret_for_binding(kind, binding_id, &secret)?;
    delete_account(kind.account_name())
}

/// Deletes the current scoped credential and the legacy target slot without touching other presets.
pub fn delete_secret_for_binding(kind: SecretKind, binding_id: &str) -> Result<(), SecretError> {
    delete_account(&scoped_account_name(kind, binding_id))?;
    delete_secret(kind)
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

/// Reports whether the exact provider binding or explicit environment source is configured.
pub fn secret_is_configured_for_binding(
    kind: SecretKind,
    binding_id: &str,
) -> Result<bool, SecretError> {
    read_secret_for_binding(kind, binding_id).map(|value| value.is_some())
}

/// Reads one exact Credential Manager account without environment fallback.
fn read_account(account: &str) -> Result<Option<String>, SecretError> {
    let entry =
        keyring::Entry::new(SERVICE_NAME, account).map_err(|_| SecretError::CredentialStore)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(SecretError::CredentialStore),
    }
}

/// Deletes one exact Credential Manager account; a missing entry is already clean.
fn delete_account(account: &str) -> Result<(), SecretError> {
    let entry =
        keyring::Entry::new(SERVICE_NAME, account).map_err(|_| SecretError::CredentialStore)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(SecretError::CredentialStore),
    }
}

/// Reads a target-scoped development credential without persisting or logging it.
fn read_environment_secret(kind: SecretKind) -> Option<String> {
    std::env::var(kind.environment_name())
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Derives a fixed, non-sensitive account name without exposing custom endpoint text.
fn scoped_account_name(kind: SecretKind, binding_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(binding_id.as_bytes()));
    format!("{}-{}", kind.account_name(), &digest[..24])
}

#[cfg(test)]
mod tests {
    use super::{scoped_account_name, SecretKind};

    #[test]
    fn scoped_account_name_is_stable_and_hides_binding_text() {
        let binding = "custom:https://private.example.test/v1";
        let account = scoped_account_name(SecretKind::Minutes, binding);
        assert_eq!(account, scoped_account_name(SecretKind::Minutes, binding));
        assert!(!account.contains("private"));
        assert_ne!(
            account,
            scoped_account_name(SecretKind::Minutes, "custom:https://other.example.test/v1")
        );
    }
}
