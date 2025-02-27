use crate::error::{ApiError, ApiErrorKind};
use crate::extractors::{
    message_id::MessageId, notification_headers::NotificationHeaders, subscription::Subscription,
};
use crate::server::AppState;
use actix_web::{dev::Payload, web, FromRequest, HttpRequest};
use autopush_common::util::{b64_encode_url, ms_since_epoch, sec_since_epoch};
use cadence::CountedExt;
use fernet::MultiFernet;
use futures::{future, FutureExt};
use std::collections::HashMap;
use uuid::Uuid;

/// Extracts notification data from `Subscription` and request data
#[derive(Clone, Debug)]
pub struct Notification {
    /// Unique message_id for this notification
    pub message_id: String,
    /// The subscription information block
    pub subscription: Subscription,
    /// Set of associated crypto headers
    pub headers: NotificationHeaders,
    /// UNIX timestamp in seconds
    pub timestamp: u64,
    /// UNIX timestamp in milliseconds
    pub sort_key_timestamp: u64,
    /// The encrypted notification body
    pub data: Option<String>,
}

impl FromRequest for Notification {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();

        async move {
            let subscription = Subscription::extract(&req).await?;
            let app_state = web::Data::<AppState>::extract(&req)
                .await
                .expect("No server state found");

            // Read data
            let data = web::Bytes::from_request(&req, &mut payload)
                .await
                .map_err(|e| {
                    debug!("▶▶ Request read payload error: {:?}", &e);
                    ApiErrorKind::PayloadError(e)
                })?;

            // Convert data to base64
            let data = if data.is_empty() {
                None
            } else {
                Some(b64_encode_url(&data.to_vec()))
            };

            let headers = NotificationHeaders::from_request(&req, data.is_some())?;
            let timestamp = sec_since_epoch();
            let sort_key_timestamp = ms_since_epoch();
            let message_id = Self::generate_message_id(
                &app_state.fernet,
                subscription.user.uaid,
                subscription.channel_id,
                headers.topic.as_deref(),
                sort_key_timestamp,
            );

            // Record the encoding if we have an encrypted payload
            if let Some(encoding) = &headers.encoding {
                if data.is_some() {
                    app_state
                        .metrics
                        .incr(&format!("updates.notification.encoding.{encoding}"))
                        .ok();
                }
            }

            Ok(Notification {
                message_id,
                subscription,
                headers,
                timestamp,
                sort_key_timestamp,
                data,
            })
        }
        .boxed_local()
    }
}

impl From<Notification> for autopush_common::notification::Notification {
    fn from(notification: Notification) -> Self {
        autopush_common::notification::Notification {
            channel_id: notification.subscription.channel_id,
            version: notification.message_id,
            ttl: notification.headers.ttl as u64,
            topic: notification.headers.topic.clone(),
            timestamp: notification.timestamp,
            data: notification.data,
            sortkey_timestamp: Some(notification.sort_key_timestamp),
            headers: {
                let headers: HashMap<String, String> = notification.headers.into();
                if headers.is_empty() {
                    None
                } else {
                    Some(headers)
                }
            },
        }
    }
}

impl Notification {
    /// Generate a message-id suitable for accessing the message
    ///
    /// For topic messages, a sort_key version of 01 is used, and the topic
    /// is included for reference:
    ///
    ///     Encrypted('01' : uaid.hex : channel_id.hex : topic)
    ///
    /// For non-topic messages, a sort_key version of 02 is used:
    ///
    ///     Encrypted('02' : uaid.hex : channel_id.hex : timestamp)
    fn generate_message_id(
        fernet: &MultiFernet,
        uaid: Uuid,
        channel_id: Uuid,
        topic: Option<&str>,
        timestamp: u64,
    ) -> String {
        let message_id = if let Some(topic) = topic {
            MessageId::WithTopic {
                uaid,
                channel_id,
                topic: topic.to_string(),
            }
        } else {
            MessageId::WithoutTopic {
                uaid,
                channel_id,
                timestamp,
            }
        };

        message_id.encrypt(fernet)
    }

    /// Serialize the notification for delivery to the connection server. Some
    /// fields in `autopush_common`'s `Notification` are marked with
    /// `#[serde(skip_serializing)]` so they are not shown to the UA. These
    /// fields are still required when delivering to the connection server, so
    /// we can't simply convert this notification type to that one and serialize
    /// via serde.
    pub fn serialize_for_delivery(&self) -> HashMap<&'static str, serde_json::Value> {
        let mut map = HashMap::new();

        map.insert(
            "channelID",
            serde_json::to_value(self.subscription.channel_id).unwrap(),
        );
        map.insert("version", serde_json::to_value(&self.message_id).unwrap());
        map.insert("ttl", serde_json::to_value(self.headers.ttl).unwrap());
        map.insert("topic", serde_json::to_value(&self.headers.topic).unwrap());
        map.insert("timestamp", serde_json::to_value(self.timestamp).unwrap());

        if let Some(data) = &self.data {
            map.insert("data", serde_json::to_value(data).unwrap());

            let headers: HashMap<_, _> = self.headers.clone().into();
            map.insert("headers", serde_json::to_value(headers).unwrap());
        }

        map
    }
}
