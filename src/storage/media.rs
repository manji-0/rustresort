//! Media storage using Cloudflare R2
//!
//! Handles upload, delete, and URL generation for media files.
//! Files are served via R2 Custom Domain (CDN).

use aws_sdk_s3::Client as S3Client;

use crate::error::AppError;

/// Media storage service
///
/// Uploads media to Cloudflare R2 and returns public URLs.
pub struct MediaStorage {
    /// S3-compatible client for R2
    client: S3Client,
    /// Media bucket name
    bucket: String,
    /// Public URL base (Custom Domain)
    /// e.g., "https://media.example.com"
    public_url: String,
}

impl MediaStorage {
    /// Create new media storage client
    ///
    /// # Arguments
    /// * `config` - Storage configuration
    /// * `cloudflare_config` - Cloudflare credentials
    ///
    /// # Errors
    /// Returns error if S3 client initialization fails
    pub async fn new(
        config: &crate::config::MediaStorageConfig,
        cloudflare: &crate::config::CloudflareConfig,
    ) -> Result<Self, AppError> {
        use aws_config::BehaviorVersion;
        use aws_sdk_s3::config::{Credentials, Region};

        // R2 endpoint: https://{account_id}.r2.cloudflarestorage.com
        let endpoint = format!("https://{}.r2.cloudflarestorage.com", cloudflare.account_id);

        // Create credentials
        let credentials = Credentials::new(
            &cloudflare.r2_access_key_id,
            &cloudflare.r2_secret_access_key,
            None,
            None,
            "rustresort-r2",
        );

        // Build S3 config for R2
        let s3_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(&endpoint)
            .credentials_provider(credentials)
            .build();

        let client = S3Client::from_conf(s3_config);

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
            public_url: config.public_url.clone(),
        })
    }

    /// Upload media file
    ///
    /// # Arguments
    /// * `key` - S3 key (path) for the file
    /// * `data` - File contents
    /// * `content_type` - MIME type
    ///
    /// # Returns
    /// Public URL for the uploaded file
    ///
    /// # Example
    /// ```ignore
    /// let url = storage.upload(
    ///     "attachments/abc123.webp",
    ///     image_data,
    ///     "image/webp"
    /// ).await?;
    /// // Returns: https://media.example.com/attachments/abc123.webp
    /// ```
    pub async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String, AppError> {
        use aws_sdk_s3::primitives::ByteStream;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .cache_control("public, max-age=31536000") // 1 year
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("R2 upload failed: {}", e)))?;

        Ok(self.get_public_url(key))
    }

    /// Upload avatar image
    ///
    /// Stores in avatars/ prefix.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for the avatar
    /// * `data` - Image data (should be processed to WebP)
    ///
    /// # Returns
    /// (S3 key, Public URL)
    pub async fn upload_avatar(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        let key = format!("avatars/{}.webp", id);
        let url = self.upload(&key, data, "image/webp").await?;
        Ok((key, url))
    }

    /// Upload header image
    ///
    /// Stores in headers/ prefix.
    pub async fn upload_header(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        let key = format!("headers/{}.webp", id);
        let url = self.upload(&key, data, "image/webp").await?;
        Ok((key, url))
    }

    /// Upload status attachment
    ///
    /// Stores in attachments/ prefix.
    ///
    /// # Arguments
    /// * `id` - Unique identifier
    /// * `data` - File data
    /// * `content_type` - MIME type
    ///
    /// # Returns
    /// (S3 key, Public URL)
    pub async fn upload_attachment(
        &self,
        id: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<(String, String), AppError> {
        // Determine file extension from content type
        let ext = match content_type {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            _ => "bin",
        };

        let key = format!("attachments/{}.{}", id, ext);
        let url = self.upload(&key, data, content_type).await?;
        Ok((key, url))
    }

    /// Upload thumbnail
    ///
    /// Stores in thumbnails/ prefix.
    pub async fn upload_thumbnail(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        let key = format!("thumbnails/{}.webp", id);
        let url = self.upload(&key, data, "image/webp").await?;
        Ok((key, url))
    }

    /// Delete media file
    ///
    /// # Arguments
    /// * `key` - S3 key to delete
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("R2 delete failed: {}", e)))?;

        Ok(())
    }

    /// Get public URL for an S3 key
    ///
    /// # Arguments
    /// * `key` - S3 key
    ///
    /// # Returns
    /// Public URL via Custom Domain
    pub fn get_public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_url, key)
    }
}
