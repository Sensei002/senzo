//! Optional Supabase-backed accounts and profile-scoped cloud synchronization.
//!
//! Authentication secrets are stored only via the OS credential store. The app contains
//! no Supabase service-role key: every sync request carries a user access token and
//! the API/database enforce the authenticated user's RLS policies.

use std::{sync::Arc, time::Duration};

use reqwest::{Client, StatusCode, redirect::Policy};
use senzo_domain::{RemoteSyncMutation, SyncOutboxRepository};
use senzo_platform::{CredentialError, CredentialStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

const SESSION_REFRESH_MARGIN_MS: i64 = 60_000;
const SYNC_BATCH_SIZE: usize = 100;
const SYNC_MAX_PULL_PAGES: usize = 20;
const SYNC_PROVIDER: &str = "supabase";

#[derive(Debug, Error)]
pub enum CloudError {
    #[error("cloud account service URL must be an absolute HTTPS URL")]
    InvalidBaseUrl,
    #[error("cloud account and sync are not configured")]
    NotConfigured,
    #[error("local OS credential storage failed: {0}")]
    Credential(#[from] CredentialError),
    #[error("cloud service transport failed: {0}")]
    Transport(String),
    #[error("cloud service rejected the request (HTTP {0})")]
    Http(u16),
    #[error("the cloud account session is invalid or expired")]
    Unauthorized,
    #[error("cloud service returned an invalid response")]
    InvalidResponse,
    #[error("local sync storage failed: {0}")]
    Repository(#[from] senzo_domain::RepositoryError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub user_id: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignupResult {
    pub confirmation_required: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pushed: usize,
    pub pulled: usize,
    pub next_cursor: String,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct CloudService {
    base_url: Url,
    client: Client,
    credential_store: Arc<dyn CredentialStore>,
}

#[derive(Debug, Deserialize)]
struct SessionEnvelope {
    session: Option<SessionPayload>,
    confirmation_required: Option<bool>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionPayload {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    user: Option<UserPayload>,
}

#[derive(Debug, Deserialize)]
struct UserPayload {
    id: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PullEnvelope {
    mutations: Vec<PulledMutation>,
    next_cursor: i64,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct PulledMutation {
    cursor: i64,
    entity_type: String,
    entity_id: String,
    operation: String,
    payload: Value,
    changed_at_ms: i64,
}

impl CloudService {
    pub fn new(base_url: &str, credential_store: Arc<dyn CredentialStore>) -> Result<Self, CloudError> {
        let base_url = Url::parse(base_url).map_err(|_| CloudError::InvalidBaseUrl)?;
        let local_http = base_url.scheme() == "http"
            && matches!(base_url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (base_url.scheme() != "https" && !local_http)
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(CloudError::InvalidBaseUrl);
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .redirect(Policy::none())
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| CloudError::Transport(error.without_url().to_string()))?;
        Ok(Self { base_url, client, credential_store })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url.as_str().trim_end_matches('/'), path)
    }

    fn session_key(profile_id: &str) -> Result<String, CloudError> {
        if profile_id.trim().is_empty() || profile_id.len() > 128 || profile_id.chars().any(char::is_control) {
            return Err(CloudError::InvalidResponse);
        }
        Ok(format!("cloud-session:{profile_id}"))
    }

    async fn post_json<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        access_token: Option<&str>,
    ) -> Result<(StatusCode, Value), CloudError> {
        let mut request = self.client.post(self.endpoint(path)).json(body);
        if let Some(token) = access_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await
            .map_err(|error| CloudError::Transport(error.without_url().to_string()))?;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|_| CloudError::InvalidResponse)?;
        Ok((status, body))
    }

    fn decode_session(payload: SessionPayload) -> Result<CloudSession, CloudError> {
        let user = payload.user.ok_or(CloudError::InvalidResponse)?;
        let access_token = payload.access_token.filter(|value| !value.is_empty() && value.len() <= 8192)
            .ok_or(CloudError::InvalidResponse)?;
        let refresh_token = payload.refresh_token.filter(|value| !value.is_empty() && value.len() <= 4096)
            .ok_or(CloudError::InvalidResponse)?;
        let user_id = user.id.filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(CloudError::InvalidResponse)?;
        let expires_in = payload.expires_in.filter(|value| (60..=31_536_000).contains(value))
            .ok_or(CloudError::InvalidResponse)?;
        Ok(CloudSession {
            access_token,
            refresh_token,
            expires_at_ms: now_ms().saturating_add(expires_in.saturating_mul(1000)),
            user_id,
            email: user.email,
        })
    }

    fn store_session(&self, profile_id: &str, session: &CloudSession) -> Result<(), CloudError> {
        let encoded = serde_json::to_string(session).map_err(|_| CloudError::InvalidResponse)?;
        self.credential_store.set(&Self::session_key(profile_id)?, &encoded)?;
        Ok(())
    }

    pub fn stored_session(&self, profile_id: &str) -> Result<Option<CloudSession>, CloudError> {
        let Some(raw) = self.credential_store.get(&Self::session_key(profile_id)?)? else {
            return Ok(None);
        };
        let session: CloudSession = serde_json::from_str(&raw).map_err(|_| CloudError::InvalidResponse)?;
        if session.access_token.is_empty() || session.refresh_token.is_empty() || session.user_id.is_empty() {
            return Err(CloudError::InvalidResponse);
        }
        Ok(Some(session))
    }

    pub async fn signup(
        &self,
        profile_id: &str,
        email: &str,
        password: &str,
        display_name: Option<&str>,
    ) -> Result<SignupResult, CloudError> {
        validate_email_password(email, password)?;
        let (status, body) = self.post_json(
            "/api/account/signup",
            &json!({"email":email,"password":password,"display_name":display_name}),
            None,
        ).await?;
        if status == StatusCode::SERVICE_UNAVAILABLE { return Err(CloudError::NotConfigured); }
        if !status.is_success() { return Err(map_http(status)); }
        let envelope: SessionEnvelope = serde_json::from_value(body).map_err(|_| CloudError::InvalidResponse)?;
        if envelope.confirmation_required.unwrap_or(false) {
            return Ok(SignupResult { confirmation_required: true, message: envelope.message });
        }
        let session = Self::decode_session(envelope.session.ok_or(CloudError::InvalidResponse)?)?;
        self.store_session(profile_id, &session)?;
        Ok(SignupResult { confirmation_required: false, message: None })
    }

    pub async fn login(&self, profile_id: &str, email: &str, password: &str) -> Result<CloudSession, CloudError> {
        validate_email_password(email, password)?;
        let (status, body) = self.post_json(
            "/api/account/login", &json!({"email":email,"password":password}), None,
        ).await?;
        if status == StatusCode::SERVICE_UNAVAILABLE { return Err(CloudError::NotConfigured); }
        if !status.is_success() { return Err(map_http(status)); }
        let envelope: SessionEnvelope = serde_json::from_value(body).map_err(|_| CloudError::InvalidResponse)?;
        let session = Self::decode_session(envelope.session.ok_or(CloudError::InvalidResponse)?)?;
        self.store_session(profile_id, &session)?;
        Ok(session)
    }

    pub async fn session(&self, profile_id: &str) -> Result<Option<CloudSession>, CloudError> {
        let Some(session) = self.stored_session(profile_id)? else { return Ok(None) };
        if session.expires_at_ms > now_ms().saturating_add(SESSION_REFRESH_MARGIN_MS) {
            return Ok(Some(session));
        }
        match self.refresh(profile_id, &session.refresh_token).await {
            Ok(refreshed) => Ok(Some(refreshed)),
            Err(CloudError::Unauthorized) => {
                // A revoked/invalid refresh token is terminal. A network or server
                // error is not: retain the refresh token so the user can retry online.
                let _ = self.credential_store.delete(&Self::session_key(profile_id)?);
                Err(CloudError::Unauthorized)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn refresh(&self, profile_id: &str, refresh_token: &str) -> Result<CloudSession, CloudError> {
        if refresh_token.is_empty() || refresh_token.len() > 4096 {
            return Err(CloudError::InvalidResponse);
        }
        let (status, body) = self.post_json(
            "/api/account/refresh", &json!({"refresh_token":refresh_token}), None,
        ).await?;
        if !status.is_success() { return Err(map_http(status)); }
        let envelope: SessionEnvelope = serde_json::from_value(body).map_err(|_| CloudError::InvalidResponse)?;
        let session = Self::decode_session(envelope.session.ok_or(CloudError::InvalidResponse)?)?;
        self.store_session(profile_id, &session)?;
        Ok(session)
    }

    pub async fn logout(&self, profile_id: &str) -> Result<(), CloudError> {
        let key = Self::session_key(profile_id)?;
        let session = self.stored_session(profile_id)?;
        let remote = if let Some(session) = session {
            self.post_json("/api/account/logout", &json!({}), Some(&session.access_token)).await
                .and_then(|(status, _)| if status.is_success() { Ok(()) } else { Err(map_http(status)) })
        } else { Ok(()) };
        self.credential_store.delete(&key)?;
        remote
    }

    pub async fn sync_profile(
        &self,
        profile_id: &str,
        repository: &dyn SyncOutboxRepository,
    ) -> Result<SyncReport, CloudError> {
        let session = self.session(profile_id).await?.ok_or(CloudError::Unauthorized)?;
        let mut report = SyncReport::default();

        for _ in 0..100 {
            let batch = repository.list_pending_sync_mutations(profile_id, now_ms(), SYNC_BATCH_SIZE)?;
            if batch.is_empty() { break; }
            let outgoing = batch.iter().map(|item| {
                // The API validates a bounded serialized payload, parses it server-side,
                // and passes the decoded JSON value to the authenticated database RPC.
                let _: Value = serde_json::from_str(&item.payload_json)
                    .map_err(|_| CloudError::InvalidResponse)?;
                Ok(json!({
                    "id":item.id,
                    "entity_type":item.entity_type,
                    "entity_id":item.entity_id,
                    "operation":item.operation,
                    "payload_json":item.payload_json,
                    "changed_at_ms":item.changed_at_ms
                }))
            }).collect::<Result<Vec<_>, CloudError>>()?;
            let (status, _) = self.post_json(
                "/api/sync",
                &json!({"operation":"push","profile_id":profile_id,"mutations":outgoing}),
                Some(&session.access_token),
            ).await?;
            if !status.is_success() {
                for item in &batch {
                    let exponent = item.attempt_count.min(7);
                    let delay = 30_000_i64.saturating_mul(1_i64 << exponent).min(3_600_000);
                    let _ = repository.record_sync_mutation_failure(
                        profile_id, &item.id, &format!("cloud push returned HTTP {}", status.as_u16()),
                        now_ms().saturating_add(delay),
                    );
                }
                return Err(map_http(status));
            }
            let ids = batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
            repository.mark_sync_mutations_applied(profile_id, &ids)?;
            report.pushed += batch.len();
        }

        let current = repository.get_sync_checkpoint(profile_id, SYNC_PROVIDER)?
            .unwrap_or_else(|| "0".to_owned());
        let mut cursor = current.parse::<i64>().map_err(|_| CloudError::InvalidResponse)?;
        for _ in 0..SYNC_MAX_PULL_PAGES {
            let (status, body) = self.post_json(
                "/api/sync",
                &json!({"operation":"pull","profile_id":profile_id,"cursor":cursor,"limit":SYNC_BATCH_SIZE}),
                Some(&session.access_token),
            ).await?;
            if !status.is_success() { return Err(map_http(status)); }
            let page: PullEnvelope = serde_json::from_value(body).map_err(|_| CloudError::InvalidResponse)?;
            let mutations = page.mutations.into_iter().map(|item| {
                let payload_json = serde_json::to_string(&item.payload).map_err(|_| CloudError::InvalidResponse)?;
                Ok(RemoteSyncMutation {
                    cursor: item.cursor,
                    entity_type: item.entity_type,
                    entity_id: item.entity_id,
                    operation: item.operation,
                    payload_json,
                    changed_at_ms: item.changed_at_ms,
                })
            }).collect::<Result<Vec<_>, CloudError>>()?;
            if page.next_cursor < cursor { return Err(CloudError::InvalidResponse); }
            repository.apply_remote_sync_page(
                profile_id, SYNC_PROVIDER, &page.next_cursor.to_string(), &mutations, now_ms(),
            )?;
            report.pulled += mutations.len();
            cursor = page.next_cursor;
            report.has_more = page.has_more;
            if !page.has_more || mutations.is_empty() { break; }
        }
        report.next_cursor = cursor.to_string();
        Ok(report)
    }
}

fn validate_email_password(email: &str, password: &str) -> Result<(), CloudError> {
    if email.len() > 254 || !email.contains('@') || email.chars().any(char::is_control)
        || !(8..=128).contains(&password.len())
    {
        return Err(CloudError::InvalidResponse);
    }
    Ok(())
}

fn map_http(status: StatusCode) -> CloudError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        CloudError::Unauthorized
    } else {
        CloudError::Http(status.as_u16())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_service_requires_secure_http_origin_and_rejects_credentials() {
        let credentials: Arc<dyn CredentialStore> = Arc::new(TestCredentials);
        assert!(CloudService::new("https://senzo-api.vercel.app", credentials.clone()).is_ok());
        assert!(CloudService::new("http://senzo-api.vercel.app", credentials.clone()).is_err());
        assert!(CloudService::new("https://user:pass@example.test", credentials.clone()).is_err());
        assert!(CloudService::new("file:///tmp/senzo", credentials).is_err());
    }

    struct TestCredentials;
    impl CredentialStore for TestCredentials {
        fn get(&self, _: &str) -> Result<Option<String>, CredentialError> { Ok(None) }
        fn set(&self, _: &str, _: &str) -> Result<(), CredentialError> { Ok(()) }
        fn delete(&self, _: &str) -> Result<(), CredentialError> { Ok(()) }
    }

    #[test]
    fn cloud_session_decode_requires_complete_identity_and_tokens() {
        let payload = SessionPayload {
            access_token: Some("access".to_owned()),
            refresh_token: Some("refresh".to_owned()),
            expires_in: Some(3600),
            user: Some(UserPayload {
                id: Some("user-1".to_owned()),
                email: Some("test@example.com".to_owned()),
            }),
        };
        let session = CloudService::decode_session(payload).unwrap();
        assert_eq!(session.user_id, "user-1");
        assert!(session.expires_at_ms > 0);
    }
}
