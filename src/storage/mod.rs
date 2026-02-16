//! Cloudflare R2 storage module
//!
//! Handles:
//! - Media file upload/download (public bucket)
//! - Database backup (private bucket)

mod backup;
mod media;

pub use backup::BackupService;
pub use media::MediaStorage;

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
