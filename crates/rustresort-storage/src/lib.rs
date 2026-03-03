mod backup;
mod error;
mod media;
mod types;

pub use backup::{BackupInfo, BackupService};
pub use error::{StorageError, StorageResult};
pub use media::MediaStorage;
pub use types::{
    BackupEncryptionConfig, BackupStorageConfig, CloudflareConfig, MediaStorageConfig,
};

pub(crate) fn build_r2_http_client() -> aws_sdk_s3::config::SharedHttpClient {
    use aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder;

    let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .enable_http2()
        .build();

    HyperClientBuilder::new().build(https_connector)
}
