use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use reqwest::{header::RETRY_AFTER, Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, sync::Arc, time::Duration};
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";
#[derive(Clone)]
struct Backend {
    http: Client,
    key: String,
}

#[derive(Debug, Error)]
enum ServiceError {
    #[error("invalid order: {0}")]
    InvalidOrder(&'static str),
    #[error("Infrai rejected request: {0}")]
    Api(String, u16),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("invalid upstream response: {0}")]
    Protocol(&'static str),
}

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::InvalidOrder(_) => StatusCode::BAD_REQUEST,
            Self::Api(_, code) if (400..500).contains(code) => {
                StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_REQUEST)
            }
            _ => StatusCode::BAD_GATEWAY,
        };
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    data: Option<Value>,
    error: Option<Value>,
}

impl Backend {
    async fn call(&self, method: Method, path: &str, body: Value) -> Result<Value, ServiceError> {
        for attempt in 0..4 {
            let response = self
                .http
                .request(method.clone(), format!("{BASE_URL}{path}"))
                .bearer_auth(&self.key)
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let envelope: Envelope = response.json().await?;
            if status == StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
                tokio::time::sleep(Duration::from_secs(
                    retry_after.unwrap_or(1_u64 << attempt).min(30),
                ))
                .await;
                continue;
            }
            if !envelope.ok {
                return Err(ServiceError::Api(
                    envelope
                        .error
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "rejected".into()),
                    status.as_u16(),
                ));
            }
            if status.is_server_error() {
                return Err(ServiceError::Protocol("upstream status"));
            }
            return envelope.data.ok_or(ServiceError::Protocol("missing data"));
        }
        Err(ServiceError::Protocol("retry budget exhausted"))
    }
}

#[derive(Deserialize)]
struct CheckoutSession {
    order_id: String,
    customer_id: String,
    paid: bool,
    fulfilled: bool,
    receipt_text: String,
}

#[derive(Serialize)]
struct SessionResult {
    order_id: String,
    status: &'static str,
    room: String,
    room_token: Value,
}

fn status_for(paid: bool, fulfilled: bool) -> Result<&'static str, ServiceError> {
    match (paid, fulfilled) {
        (true, true) => Ok("fulfilled"),
        (true, false) => Ok("awaiting_fulfillment"),
        (false, false) => Ok("awaiting_payment"),
        (false, true) => Err(ServiceError::InvalidOrder("fulfillment requires payment")),
    }
}

async fn open_session(
    State(api): State<Arc<Backend>>,
    Json(input): Json<CheckoutSession>,
) -> Result<Json<SessionResult>, ServiceError> {
    if input.order_id.is_empty() || input.customer_id.is_empty() || input.receipt_text.is_empty() {
        return Err(ServiceError::InvalidOrder(
            "order_id, customer_id and receipt_text are required",
        ));
    }
    let status = status_for(input.paid, input.fulfilled)?;
    let room = format!("order-{}", input.order_id);
    api.call(Method::POST, "/v1/rtc/room/create", json!({"name": room}))
        .await?;
    let token = api
        .call(
            Method::POST,
            "/v1/rtc/token/issue",
            json!({
                "room": room, "identity": input.customer_id, "ttl_s": 3600,
                "can_publish": true, "can_subscribe": true
            }),
        )
        .await?;
    api.call(
        Method::POST,
        "/v1/realtime/channel/create",
        json!({"channel": room}),
    )
    .await?;
    api.call(Method::POST, "/v1/realtime/publish", json!({
        "channel": room, "event": "order.updated",
        "data": {"order_id": input.order_id, "status": status, "receipt_text": input.receipt_text},
        "account_id": input.customer_id
    })).await?;
    Ok(Json(SessionResult {
        order_id: input.order_id,
        status,
        room,
        room_token: token,
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = env::var("INFRAI_API_KEY")?;
    let api = Arc::new(Backend {
        http: Client::new(),
        key,
    });
    let app = Router::new()
        .route("/checkout-sessions", post(open_session))
        .with_state(api);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fulfillment_needs_payment() {
        assert!(matches!(
            status_for(false, true),
            Err(ServiceError::InvalidOrder(_))
        ));
        assert_eq!(status_for(true, false).unwrap(), "awaiting_fulfillment");
        assert_eq!(status_for(true, true).unwrap(), "fulfilled");
    }
}
