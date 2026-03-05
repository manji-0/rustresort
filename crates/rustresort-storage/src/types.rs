use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct MediaStorageConfig {
    pub bucket: String,
    pub public_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackupStorageConfig {
    pub enabled: bool,
    pub bucket: String,
    pub interval_seconds: u64,
    pub retention_count: usize,
    #[serde(default)]
    pub encryption: BackupEncryptionConfig,
}

#[derive(Clone, Default, Deserialize)]
pub struct BackupEncryptionConfig {
    #[serde(default)]
    pub enabled: bool,
    pub key: Option<String>,
}

impl std::fmt::Debug for BackupEncryptionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupEncryptionConfig")
            .field("enabled", &self.enabled)
            .field(
                "key",
                &self.key.as_ref().map(|_| "<redacted>").unwrap_or("<none>"),
            )
            .finish()
    }
}

#[derive(Clone, Deserialize)]
pub struct CloudflareConfig {
    pub account_id: String,
    pub r2_access_key_id: String,
    pub r2_secret_access_key: String,
}

impl std::fmt::Debug for CloudflareConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareConfig")
            .field("account_id", &self.account_id)
            .field("r2_access_key_id", &"<redacted>")
            .field("r2_secret_access_key", &"<redacted>")
            .finish()
    }
}
