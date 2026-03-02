#[derive(Debug, Clone)]
pub struct MediaStorageConfig {
    pub bucket: String,
    pub public_url: String,
}

#[derive(Debug, Clone)]
pub struct BackupStorageConfig {
    pub enabled: bool,
    pub bucket: String,
    pub interval_seconds: u64,
    pub retention_count: usize,
    pub encryption: BackupEncryptionConfig,
}

#[derive(Debug, Clone, Default)]
pub struct BackupEncryptionConfig {
    pub enabled: bool,
    pub key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CloudflareConfig {
    pub account_id: String,
    pub r2_access_key_id: String,
    pub r2_secret_access_key: String,
}
