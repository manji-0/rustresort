use std::sync::Arc;

use axum::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openssl::{
    bn::BigNumContext,
    ec::{EcGroup, EcKey, PointConversionForm},
    nid::Nid,
    pkey::PKey,
};
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

use crate::data::{Database, PushPayload, PushSubscription};
use crate::error::AppError;

const PUSH_VAPID_PRIVATE_KEY_SETTING_KEY: &str = "push.vapid.private_pem";
const PUSH_VAPID_PUBLIC_KEY_SETTING_KEY: &str = "push.vapid.public_key";

#[async_trait]
pub trait WebPushSender: Send + Sync {
    async fn send(
        &self,
        subscription: &PushSubscription,
        payload: &PushPayload,
    ) -> Result<(), AppError>;
    async fn server_key(&self) -> Result<String, AppError>;
}

pub struct DbWebPushSender {
    db: Arc<Database>,
    subject: String,
    client: IsahcWebPushClient,
}

impl DbWebPushSender {
    pub fn new(db: Arc<Database>, subject: String) -> Result<Self, AppError> {
        Ok(Self {
            db,
            subject,
            client: IsahcWebPushClient::new().map_err(|error| {
                AppError::internal(format!("Failed to initialize web push client: {}", error))
            })?,
        })
    }

    async fn ensure_vapid_keys(&self) -> Result<(String, String), AppError> {
        let public_key = self
            .db
            .get_setting(PUSH_VAPID_PUBLIC_KEY_SETTING_KEY)
            .await?;
        let private_pem = self
            .db
            .get_setting(PUSH_VAPID_PRIVATE_KEY_SETTING_KEY)
            .await?;

        if let (Some(public_key), Some(private_pem)) = (public_key, private_pem) {
            return Ok((public_key, private_pem));
        }

        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(|error| {
            AppError::internal(format!("Failed to load P-256 group: {}", error))
        })?;
        let ec_key = EcKey::generate(&group).map_err(|error| {
            AppError::internal(format!("Failed to generate VAPID key: {}", error))
        })?;
        let private_key_pem = PKey::from_ec_key(ec_key.clone())
            .and_then(|key| key.private_key_to_pem_pkcs8())
            .map_err(|error| {
                AppError::internal(format!("Failed to encode VAPID private key: {}", error))
            })?;

        let mut context = BigNumContext::new().map_err(|error| {
            AppError::internal(format!("Failed to allocate OpenSSL context: {}", error))
        })?;
        let public_key_bytes = ec_key
            .public_key()
            .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut context)
            .map_err(|error| {
                AppError::internal(format!("Failed to encode VAPID public key: {}", error))
            })?;
        let public_key = URL_SAFE_NO_PAD.encode(public_key_bytes);
        let private_key_pem = String::from_utf8(private_key_pem).map_err(|error| {
            AppError::internal(format!("Failed to decode VAPID private key PEM: {}", error))
        })?;

        self.db
            .set_setting(PUSH_VAPID_PUBLIC_KEY_SETTING_KEY, &public_key)
            .await?;
        self.db
            .set_setting(PUSH_VAPID_PRIVATE_KEY_SETTING_KEY, &private_key_pem)
            .await?;
        Ok((public_key, private_key_pem))
    }
}

#[async_trait]
impl WebPushSender for DbWebPushSender {
    async fn send(
        &self,
        subscription: &PushSubscription,
        payload: &PushPayload,
    ) -> Result<(), AppError> {
        let (_, private_key_pem) = self.ensure_vapid_keys().await?;
        let subscription_info = SubscriptionInfo::new(
            subscription.endpoint.clone(),
            subscription.p256dh.clone(),
            subscription.auth.clone(),
        );

        let mut signature_builder =
            VapidSignatureBuilder::from_pem(private_key_pem.as_bytes(), &subscription_info)
                .map_err(|error| {
                    AppError::Federation(format!("Failed to build VAPID signer: {}", error))
                })?;
        signature_builder.add_claim("sub", self.subject.clone());
        let vapid_signature = signature_builder.build().map_err(|error| {
            AppError::Federation(format!("Failed to build VAPID signature: {}", error))
        })?;

        let body = serde_json::to_vec(payload).map_err(|error| {
            AppError::Validation(format!("Failed to serialize push payload: {}", error))
        })?;
        let mut message_builder = WebPushMessageBuilder::new(&subscription_info);
        message_builder.set_payload(ContentEncoding::Aes128Gcm, &body);
        message_builder.set_vapid_signature(vapid_signature);
        let message = message_builder.build().map_err(|error| {
            AppError::Federation(format!("Failed to build push message: {}", error))
        })?;

        self.client
            .send(message)
            .await
            .map_err(|error| AppError::Federation(format!("Web Push delivery failed: {}", error)))
    }

    async fn server_key(&self) -> Result<String, AppError> {
        let (public_key, _) = self.ensure_vapid_keys().await?;
        Ok(public_key)
    }
}
