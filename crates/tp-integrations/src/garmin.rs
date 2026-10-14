//! Garmin's unofficial mobile SSO and Connect activity read/upload protocols.
//! Callers own credential persistence and account/transfer coordination.
//! Uploads return only confirmed remote activity IDs and never resend the file.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROVIDER_ID: &str = "garmin";
const SSO_BASE: &str = "https://sso.garmin.com";
const API_BASE: &str = "https://connectapi.garmin.com";
const TOKEN_URL: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";
const SSO_CLIENT_ID: &str = "GCM_IOS_DARK";
const SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/ios";
const TOKEN_CLIENT_ID: &str = "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2";
const TICKET_GRANT: &str =
    "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const API_USER_AGENT: &str = "GCM-Android-5.23";
const GARMIN_USER_AGENT: &str = "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0";
const APP_VERSION: &str = "10861";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const UPLOAD_STATUS_CHECKS: usize = 6;
const UPLOAD_STATUS_INTERVAL: Duration = Duration::from_secs(2);
const UPLOAD_STATUS_PATH: &str = "/activity-service/activity/status/";
const ACTIVITY_PATH: &str = "/activity-service/activity/";
const MFA_LIFETIME: Duration = Duration::from_secs(300);
const TOKEN_REFRESH_MARGIN_S: u64 = 60;
// Garmin import-result message code, independent of the HTTP response status.
const DUPLICATE_ACTIVITY_MESSAGE_CODE: u64 = 202;
const FIT_SIGNATURE_OFFSET: usize = 8;
const FIT_SIGNATURE_END: usize = 12;
const MIN_FIT_HEADER_SIZE: usize = 12;
const MAX_ACTIVITY_ID_DIGITS: usize = 20;
const MAX_PAGE_SIZE: u32 = 100;
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_ORIGINAL_BYTES: usize = 32 * 1024 * 1024;
const CAPTURE_CHART_POINTS: &str = "2000";
const CAPTURE_POLYLINE_POINTS: &str = "0";

/// The caller persists exact response bytes and owns capture provenance.
#[derive(Clone, Copy)]
pub enum ActivityResource {
    Summary,
    Laps,
    Details,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GarminError {
    #[error("Invalid Garmin activity ID or page bounds.")]
    InvalidRequest,
    #[error("Garmin response exceeds the supported size limit.")]
    ResponseTooLarge,
    #[error("Could not reach Garmin.")]
    Network,
    #[error("Garmin authentication expired or was rejected.")]
    Authentication,
    #[error("Garmin requires a browser challenge.")]
    Challenge,
    #[error("Garmin rate limit exceeded.")]
    RateLimited,
    #[error("Garmin returned an unexpected response.")]
    Protocol,
    #[error("Garmin rejected the verification code.")]
    MfaRejected,
    #[error("Garmin verification expired.")]
    MfaExpired,
    #[error("The activity already exists in Garmin.")]
    Duplicate,
    #[error("Garmin rejected the FIT activity.")]
    UploadRejected,
    #[error("Garmin requires account setup or upload consent.")]
    UploadConsentRequired,
    #[error("Garmin may have received the activity, but its upload could not be confirmed.")]
    UploadUncertain,
    #[error("The supplied file is not a FIT activity.")]
    InvalidFit,
}

// Deliberately no Debug implementation: neither tokens nor MFA cookies belong in logs.
#[derive(Serialize, Deserialize)]
pub struct Tokens {
    access_token: String,
    refresh_token: String,
    client_id: String,
    expires_at_unix_s: Option<u64>,
}

impl Tokens {
    /// Whether the caller should refresh and durably store the returned tokens
    /// before starting an authenticated operation.
    pub fn needs_refresh(&self) -> bool {
        self.expires_at_unix_s
            .is_none_or(|expires| now_unix_s().saturating_add(TOKEN_REFRESH_MARGIN_S) >= expires)
    }
}

pub struct PendingMfa {
    client: GarminClient,
    method: String,
    started_at: Instant,
}

pub enum SignIn {
    Connected(Tokens),
    NeedsMfa(PendingMfa),
}

pub struct Account {
    pub id: String,
    pub display_name: Option<String>,
}

pub struct GarminClient {
    http: Client,
    sso_base: String,
    api_base: String,
    token_url: String,
}

impl GarminClient {
    /// Fetch one bounded page. Never implies a complete activity history.
    pub async fn list_activities(
        &self,
        tokens: &Tokens,
        start: u32,
        limit: u32,
    ) -> Result<Value, GarminError> {
        self.activity_page(tokens, start, limit, None).await
    }

    /// Fetch a single date-filtered page using Garmin calendar dates. Callers
    /// must pad for provider-local dates and independently filter UTC timestamps.
    pub async fn list_activities_by_date(
        &self,
        tokens: &Tokens,
        start: u32,
        limit: u32,
        start_date: &str,
        end_date: &str,
    ) -> Result<Value, GarminError> {
        const CALENDAR_DATE_FORMAT: &str = "%Y-%m-%d";
        const CALENDAR_DATE_LENGTH: usize = 10;
        let valid = |date: &str| {
            date.len() == CALENDAR_DATE_LENGTH
                && chrono::NaiveDate::parse_from_str(date, CALENDAR_DATE_FORMAT)
                    .is_ok_and(|parsed| parsed.format(CALENDAR_DATE_FORMAT).to_string() == date)
        };
        if !valid(start_date) || !valid(end_date) || start_date > end_date {
            return Err(GarminError::InvalidRequest);
        }
        self.activity_page(tokens, start, limit, Some((start_date, end_date)))
            .await
    }

    async fn activity_page(
        &self,
        tokens: &Tokens,
        start: u32,
        limit: u32,
        dates: Option<(&str, &str)>,
    ) -> Result<Value, GarminError> {
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(GarminError::InvalidRequest);
        }
        let mut request = native_headers(self.http.get(format!(
            "{}/activitylist-service/activities/search/activities",
            self.api_base
        )))
        .bearer_auth(&tokens.access_token)
        .query(&[("start", start), ("limit", limit)]);
        if let Some((start_date, end_date)) = dates {
            request = request.query(&[("startDate", start_date), ("endDate", end_date)]);
        }
        let response = request.send().await.map_err(|_| GarminError::Network)?;
        let bytes = bounded_response(response, MAX_JSON_BYTES).await?;
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| GarminError::Protocol)?;
        let activities = body.as_array().ok_or(GarminError::Protocol)?;
        if activities.len() > limit as usize
            || activities
                .iter()
                .any(|v| v.get("activityId").and_then(positive_id).is_none())
        {
            return Err(GarminError::Protocol);
        }
        Ok(body)
    }

    /// Read a selected resource once, without refresh, retry, or filesystem access.
    /// JSON is validated but returned unchanged. Original ZIP/FIT verification is
    /// the caller's responsibility; redirects are never followed with credentials.
    pub async fn read_activity(
        &self,
        tokens: &Tokens,
        activity_id: &str,
        resource: ActivityResource,
    ) -> Result<Vec<u8>, GarminError> {
        if activity_id.is_empty()
            || activity_id.len() > MAX_ACTIVITY_ID_DIGITS
            || activity_id.starts_with('0')
            || !activity_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(GarminError::InvalidRequest);
        }
        let path = match resource {
            ActivityResource::Summary => format!("{ACTIVITY_PATH}{activity_id}"),
            ActivityResource::Laps => format!("{ACTIVITY_PATH}{activity_id}/splits"),
            ActivityResource::Details => format!("{ACTIVITY_PATH}{activity_id}/details"),
            ActivityResource::Original => format!("/download-service/files/activity/{activity_id}"),
        };
        let mut request = native_headers(self.http.get(format!("{}{path}", self.api_base)))
            .bearer_auth(&tokens.access_token);
        if matches!(resource, ActivityResource::Details) {
            request = request.query(&[
                ("maxChartSize", CAPTURE_CHART_POINTS),
                ("maxPolylineSize", CAPTURE_POLYLINE_POINTS),
            ]);
        }
        // The download endpoint serves ZIP bytes and rejects JSON-only Accept
        // negotiation with HTTP 406. Keep JSON headers on structured reads.
        if matches!(resource, ActivityResource::Original) {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("*/*"),
            );
            request = request.headers(headers);
        }
        let maximum = if matches!(resource, ActivityResource::Original) {
            MAX_ORIGINAL_BYTES
        } else {
            MAX_JSON_BYTES
        };
        let response = request.send().await.map_err(|_| GarminError::Network)?;
        let bytes = bounded_response(response, maximum).await?;
        if !matches!(resource, ActivityResource::Original) {
            let body: Value = serde_json::from_slice(&bytes).map_err(|_| GarminError::Protocol)?;
            if !body.is_object() {
                return Err(GarminError::Protocol);
            }
            if matches!(resource, ActivityResource::Summary | ActivityResource::Laps)
                && body.get("activityId").and_then(positive_id).as_deref() != Some(activity_id)
            {
                return Err(GarminError::Protocol);
            }
            if matches!(resource, ActivityResource::Laps) && !body["lapDTOs"].is_array() {
                return Err(GarminError::Protocol);
            }
        }
        Ok(bytes)
    }

    /// Create a client for Garmin's production services with a private cookie jar.
    pub fn new() -> Result<Self, GarminError> {
        Self::with_endpoints(SSO_BASE, API_BASE, TOKEN_URL)
    }

    fn with_endpoints(sso: &str, api: &str, token: &str) -> Result<Self, GarminError> {
        Ok(Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .cookie_store(true)
                // Login and upload endpoints return JSON; never replay credentials to a redirect.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| GarminError::Network)?,
            sso_base: sso.into(),
            api_base: api.into(),
            token_url: token.into(),
        })
    }

    fn login_request(&self, path: &str) -> RequestBuilder {
        self.http
            .post(format!("{}{path}", self.sso_base))
            .query(&[
                ("clientId", SSO_CLIENT_ID),
                ("locale", "en-US"),
                ("service", SERVICE_URL),
            ])
            .header("User-Agent", LOGIN_USER_AGENT)
            .header("Accept", "application/json, text/plain, */*")
            .header("Origin", &self.sso_base)
    }

    pub async fn sign_in(self, email: &str, password: &str) -> Result<SignIn, GarminError> {
        let response = self.login_request("/mobile/api/login")
            .json(&json!({"username": email, "password": password, "rememberMe": true, "captchaToken": ""}))
            .send().await.map_err(|_| GarminError::Network)?;
        let body = json_response(response).await?;
        match login_status(&body)? {
            "SUCCESSFUL" => Ok(SignIn::Connected(self.exchange_ticket(&body).await?)),
            "MFA_REQUIRED" => Ok(SignIn::NeedsMfa(PendingMfa {
                client: self,
                method: body
                    .pointer("/customerMfaInfo/mfaLastMethodUsed")
                    .and_then(Value::as_str)
                    .unwrap_or("email")
                    .to_string(),
                started_at: Instant::now(),
            })),
            "INVALID_USERNAME_PASSWORD" => Err(GarminError::Authentication),
            "CAPTCHA_REQUIRED" => Err(GarminError::Challenge),
            _ => Err(GarminError::Protocol),
        }
    }

    async fn exchange_ticket(&self, body: &Value) -> Result<Tokens, GarminError> {
        let ticket = body
            .get("serviceTicketId")
            .and_then(Value::as_str)
            .filter(|ticket| !ticket.is_empty())
            .ok_or(GarminError::Protocol)?;
        let response = native_headers(self.http.post(&self.token_url))
            .basic_auth(TOKEN_CLIENT_ID, Some(""))
            .form(&[
                ("client_id", TOKEN_CLIENT_ID),
                ("service_ticket", ticket),
                ("grant_type", TICKET_GRANT),
                ("service_url", SERVICE_URL),
            ])
            .send()
            .await
            .map_err(|_| GarminError::Network)?;
        tokens_from_response(json_response(response).await?, TOKEN_CLIENT_ID, None)
    }

    pub async fn refresh(&self, tokens: &Tokens) -> Result<Tokens, GarminError> {
        let response = native_headers(self.http.post(&self.token_url))
            .basic_auth(&tokens.client_id, Some(""))
            .form(&[
                ("client_id", tokens.client_id.as_str()),
                ("refresh_token", tokens.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|_| GarminError::Network)?;
        if response.status() == StatusCode::BAD_REQUEST {
            return Err(GarminError::Authentication);
        }
        tokens_from_response(
            json_response(response).await?,
            &tokens.client_id,
            Some(&tokens.refresh_token),
        )
    }

    pub async fn account(&self, tokens: &Tokens) -> Result<Account, GarminError> {
        let response = native_headers(self.http.get(format!(
            "{}/userprofile-service/socialProfile",
            self.api_base
        )))
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(|_| GarminError::Network)?;
        let body = json_response(response).await?;
        let id = body
            .get("id")
            .and_then(positive_id)
            .ok_or(GarminError::Protocol)?;
        let display_name = body
            .get("fullName")
            .or_else(|| body.get("displayName"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string);
        Ok(Account { id, display_name })
    }

    /// Send the FIT once and return its confirmed remote activity ID. A pending
    /// upload is polled without resending; an uncertain outcome is an error.
    /// The caller supplies the multipart filename and owns deduplication receipts.
    pub async fn upload(
        &self,
        tokens: &Tokens,
        fit: Vec<u8>,
        filename: &str,
    ) -> Result<String, GarminError> {
        validate_fit(&fit)?;
        let file = reqwest::multipart::Part::bytes(fit)
            .file_name(filename.to_owned())
            .mime_str("application/octet-stream")
            .map_err(|_| GarminError::InvalidFit)?;
        let response = native_headers(
            self.http
                .post(format!("{}/upload-service/upload", self.api_base)),
        )
        .bearer_auth(&tokens.access_token)
        .multipart(reqwest::multipart::Form::new().part("file", file))
        .send()
        .await
        .map_err(|_| GarminError::UploadUncertain)?;
        let status = response.status();
        match status {
            StatusCode::UNAUTHORIZED => return Err(GarminError::Authentication),
            StatusCode::FORBIDDEN => return Err(GarminError::Challenge),
            StatusCode::TOO_MANY_REQUESTS => return Err(GarminError::RateLimited),
            StatusCode::CONFLICT => return Err(GarminError::Duplicate),
            StatusCode::PRECONDITION_FAILED => return Err(GarminError::UploadConsentRequired),
            _ if status.is_client_error() => return Err(GarminError::UploadRejected),
            _ if !status.is_success() => return Err(GarminError::UploadUncertain),
            _ => {}
        }
        if status == StatusCode::ACCEPTED {
            return self.confirm_upload(tokens, response).await;
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| GarminError::UploadUncertain)?;
        upload_id(&body)
    }

    async fn confirm_upload(
        &self,
        tokens: &Tokens,
        accepted: Response,
    ) -> Result<String, GarminError> {
        // A 202 acknowledges receipt, not a created Activity. Follow only the
        // status resource on our API origin; never resend the multipart POST.
        let poll_url = upload_location(&accepted, &self.api_base)?;
        if !poll_url.path().starts_with(UPLOAD_STATUS_PATH)
            || poll_url
                .path()
                .trim_start_matches(UPLOAD_STATUS_PATH)
                .is_empty()
        {
            return Err(GarminError::UploadUncertain);
        }
        for check in 0..UPLOAD_STATUS_CHECKS {
            if check > 0 {
                tokio::time::sleep(UPLOAD_STATUS_INTERVAL).await;
            }
            let response = native_headers(self.http.get(poll_url.clone()))
                .bearer_auth(&tokens.access_token)
                .send()
                .await
                .map_err(|_| GarminError::UploadUncertain)?;
            match response.status() {
                StatusCode::ACCEPTED => continue,
                StatusCode::CREATED => {
                    let location = upload_location(&response, &self.api_base)?;
                    return location
                        .path()
                        .strip_prefix(ACTIVITY_PATH)
                        .and_then(|id| positive_id(&Value::String(id.to_owned())))
                        .ok_or(GarminError::UploadUncertain);
                }
                StatusCode::OK => {
                    let body = response
                        .json()
                        .await
                        .map_err(|_| GarminError::UploadUncertain)?;
                    return upload_id(&body);
                }
                StatusCode::UNAUTHORIZED => return Err(GarminError::Authentication),
                StatusCode::FORBIDDEN => return Err(GarminError::Challenge),
                StatusCode::CONFLICT => return Err(GarminError::Duplicate),
                StatusCode::TOO_MANY_REQUESTS => return Err(GarminError::RateLimited),
                // Failure to read the status cannot prove the POST failed.
                _ => return Err(GarminError::UploadUncertain),
            }
        }
        Err(GarminError::UploadUncertain)
    }
}

fn upload_location(response: &Response, api_base: &str) -> Result<Url, GarminError> {
    let base = Url::parse(api_base).map_err(|_| GarminError::UploadUncertain)?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| base.join(value).ok())
        .ok_or(GarminError::UploadUncertain)?;
    if location.origin() != base.origin()
        || !location.username().is_empty()
        || location.password().is_some()
        || location.query().is_some()
        || location.fragment().is_some()
    {
        return Err(GarminError::UploadUncertain);
    }
    Ok(location)
}

impl PendingMfa {
    pub async fn complete(&self, code: &str) -> Result<Tokens, GarminError> {
        if self.started_at.elapsed() >= MFA_LIFETIME {
            return Err(GarminError::MfaExpired);
        }
        let response = self
            .client
            .login_request("/mobile/api/mfa/verifyCode")
            .json(
                &json!({"mfaMethod": self.method, "mfaVerificationCode": code,
                "rememberMyBrowser": true, "reconsentList": [], "mfaSetup": false}),
            )
            .send()
            .await
            .map_err(|_| GarminError::Network)?;
        let body = json_response(response).await?;
        match login_status(&body)? {
            "SUCCESSFUL" => self.client.exchange_ticket(&body).await,
            "CAPTCHA_REQUIRED" => Err(GarminError::Challenge),
            _ => Err(GarminError::MfaRejected),
        }
    }
}

fn native_headers(request: RequestBuilder) -> RequestBuilder {
    request
        .header("User-Agent", API_USER_AGENT)
        .header("X-Garmin-User-Agent", GARMIN_USER_AGENT)
        .header("X-Garmin-Paired-App-Version", APP_VERSION)
        .header("X-Garmin-Client-Platform", "Android")
        .header("X-App-Ver", APP_VERSION)
        .header("X-Lang", "en")
        .header("X-GCExperience", "GC5")
        .header("Accept", "application/json")
        .header("Accept-Language", "en-US,en;q=0.9")
}

async fn json_response(response: Response) -> Result<Value, GarminError> {
    let bytes = bounded_response(response, MAX_JSON_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|_| GarminError::Protocol)
}

async fn bounded_response(mut response: Response, maximum: usize) -> Result<Vec<u8>, GarminError> {
    match response.status() {
        StatusCode::UNAUTHORIZED => return Err(GarminError::Authentication),
        StatusCode::FORBIDDEN => return Err(GarminError::Challenge),
        StatusCode::TOO_MANY_REQUESTS => return Err(GarminError::RateLimited),
        status if !status.is_success() => return Err(GarminError::Protocol),
        _ => {}
    }
    if response
        .content_length()
        .is_some_and(|size| size > maximum as u64)
    {
        return Err(GarminError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| GarminError::Network)? {
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(GarminError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn login_status(body: &Value) -> Result<&str, GarminError> {
    let code = body.pointer("/error/status-code");
    if code == Some(&json!(429)) || code == Some(&json!("429")) {
        return Err(GarminError::RateLimited);
    }
    body.pointer("/responseStatus/type")
        .and_then(Value::as_str)
        .ok_or(GarminError::Protocol)
}

fn tokens_from_response(
    body: Value,
    client_id: &str,
    old_refresh: Option<&str>,
) -> Result<Tokens, GarminError> {
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(GarminError::Protocol)?
        .to_string();
    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .or(old_refresh)
        .filter(|s| !s.is_empty())
        .ok_or(GarminError::Protocol)?
        .to_string();
    // Claims are hints for expiry/client selection, never evidence of authentication.
    // The profile request verifies the session before a connection is persisted.
    let claims = access_token
        .split('.')
        .nth(1)
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let client_id = claims
        .as_ref()
        .and_then(|v| v.get("client_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(client_id)
        .to_string();
    let expires_at_unix_s = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(|seconds| now_unix_s().saturating_add(seconds))
        .or_else(|| {
            claims
                .as_ref()
                .and_then(|v| v.get("exp"))
                .and_then(Value::as_u64)
        });
    Ok(Tokens {
        access_token,
        refresh_token,
        client_id,
        expires_at_unix_s,
    })
}

fn now_unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn positive_id(value: &Value) -> Option<String> {
    let id = value.as_u64().or_else(|| value.as_str()?.parse().ok())?;
    (id > 0).then(|| id.to_string())
}

fn upload_id(body: &Value) -> Result<String, GarminError> {
    let result = body
        .get("detailedImportResult")
        .ok_or(GarminError::UploadUncertain)?;
    let failures = result
        .get("failures")
        .and_then(Value::as_array)
        .ok_or(GarminError::UploadUncertain)?;
    if !failures.is_empty() {
        let duplicate = failures.iter().any(|failure| {
            failure
                .get("messages")
                .and_then(Value::as_array)
                .is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message
                            .get("code")
                            .and_then(|code| code.as_u64().or_else(|| code.as_str()?.parse().ok()))
                            == Some(DUPLICATE_ACTIVITY_MESSAGE_CODE)
                    })
                })
        });
        return Err(if duplicate {
            GarminError::Duplicate
        } else {
            GarminError::UploadRejected
        });
    }
    let successes = result
        .get("successes")
        .and_then(Value::as_array)
        .ok_or(GarminError::UploadUncertain)?;
    if successes.len() != 1 {
        return Err(GarminError::UploadUncertain);
    }
    successes[0]
        .get("internalId")
        .and_then(positive_id)
        .ok_or(GarminError::UploadUncertain)
}

pub fn validate_fit(fit: &[u8]) -> Result<(), GarminError> {
    if fit.len() < MIN_FIT_HEADER_SIZE || fit[FIT_SIGNATURE_OFFSET..FIT_SIGNATURE_END] != *b".FIT" {
        return Err(GarminError::InvalidFit);
    }
    // This is a header check; callers are responsible for full FIT validity.
    Ok(())
}

// Private endpoint injection and MFA expiry keep protocol tests local.
// Public payload and credential compatibility tests live in tests/.
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
    const READ_BUFFER_BYTES: usize = 8192;
    const HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";
    const SYNTHETIC_FIT: &[u8] = b"\x0c\x20\x00\x00\x00\x00\x00\x00.FIT";

    // A local HTTP boundary exercises request bodies, cookies and one-shot POST
    // behavior; no account data, Garmin calls or vault entries are used.
    fn server(responses: Vec<(&str, Value, &str)>) -> (GarminClient, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses: Vec<_> = responses.into_iter().map(|(status, body, extra)| {
            let body = body.to_string();
            format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len())
        }).collect();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(SOCKET_TIMEOUT)).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; READ_BUFFER_BYTES];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes
                        .windows(HEADER_SEPARATOR.len())
                        .position(|part| part == HEADER_SEPARATOR)
                    {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + HEADER_SEPARATOR.len() + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (
            GarminClient::with_endpoints(&base, &base, &format!("{base}/token")).unwrap(),
            handle,
        )
    }

    fn tokens() -> Tokens {
        serde_json::from_value(json!({"access_token":"test-access", "refresh_token":"test-refresh", "client_id":"test-client"})).unwrap()
    }

    #[tokio::test]
    async fn date_page_preserves_bounds_and_does_not_paginate() {
        let (client, requests) = server(vec![("200 OK", json!([]), ""), ("200 OK", json!([]), "")]);
        client
            .list_activities_by_date(&tokens(), 100, 100, "2026-01-01", "2026-01-05")
            .await
            .unwrap();
        client
            .list_activities_by_date(&tokens(), 0, 100, "2000-02-29", "2024-02-29")
            .await
            .unwrap();
        let requests = requests.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("startDate=2000-02-29&endDate=2024-02-29"));
        assert!(requests[0].starts_with("GET /activitylist-service/activities/search/activities?start=100&limit=100&startDate=2026-01-01&endDate=2026-01-05 "));
        for (start, end) in [
            ("bad", "2026-01-05"),
            ("2026-01-06", "2026-01-05"),
            ("2026-02-31", "2026-03-01"),
            ("2026-13-01", "2027-01-01"),
            ("2026-02-29", "2026-03-01"),
            ("1900-02-29", "1900-03-01"),
            ("2026-01-01", "2026-04-31"),
            ("2026-00-01", "2026-01-01"),
            ("2026-01-00", "2026-01-01"),
            ("2026-1-01", "2026-01-01"),
        ] {
            assert_eq!(
                client
                    .list_activities_by_date(&tokens(), 0, 100, start, end)
                    .await
                    .unwrap_err(),
                GarminError::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn reads_one_bounded_page_and_preserves_resource_bytes() {
        let summary = json!({"activityId":42,"summaryDTO":{"averagePower":null}});
        let (client, requests) = server(vec![
            ("200 OK", json!([{"activityId":42}]), ""),
            ("200 OK", summary.clone(), ""),
            ("200 OK", json!({"activityId":42,"lapDTOs":[]}), ""),
            ("200 OK", json!({"metrics":[]}), ""),
            ("200 OK", json!({"opaque":"original response"}), ""),
        ]);
        assert_eq!(
            client
                .list_activities(&tokens(), 20, 2)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            client
                .read_activity(&tokens(), "42", ActivityResource::Summary)
                .await
                .unwrap(),
            summary.to_string().as_bytes()
        );
        client
            .read_activity(&tokens(), "42", ActivityResource::Laps)
            .await
            .unwrap();
        client
            .read_activity(&tokens(), "42", ActivityResource::Details)
            .await
            .unwrap();
        client
            .read_activity(&tokens(), "42", ActivityResource::Original)
            .await
            .unwrap();
        let requests = requests.join().unwrap();
        assert!(requests[0].starts_with(
            "GET /activitylist-service/activities/search/activities?start=20&limit=2 "
        ));
        assert!(requests[1].starts_with("GET /activity-service/activity/42 "));
        assert!(requests[2].starts_with("GET /activity-service/activity/42/splits "));
        assert!(requests[3].starts_with(
            "GET /activity-service/activity/42/details?maxChartSize=2000&maxPolylineSize=0 "
        ));
        assert!(requests[4].starts_with("GET /download-service/files/activity/42 "));
        assert!(requests[4].to_ascii_lowercase().contains("accept: */*\r\n"));
        assert!(!requests[4]
            .to_ascii_lowercase()
            .contains("accept: application/json"));
        assert!(requests[..4].iter().all(|r| r
            .to_ascii_lowercase()
            .contains("accept: application/json\r\n")));

        assert!(requests.iter().all(|r| r
            .to_ascii_lowercase()
            .contains("authorization: bearer test-access")));
    }

    #[tokio::test]
    async fn response_limits_cover_declared_and_chunked_bodies() {
        for headers_and_body in [
            "Content-Length: 99999999\r\n\r\n",
            "Transfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n0\r\n\r\n",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let response = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n{headers_and_body}");
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(SOCKET_TIMEOUT)).unwrap();
                let mut request = [0; READ_BUFFER_BYTES];
                assert!(stream.read(&mut request).unwrap() > 0);
                stream.write_all(response.as_bytes()).unwrap();
            });
            let client = GarminClient::with_endpoints(&base, &base, &base).unwrap();
            let response = client.http.get(&base).send().await.unwrap();
            assert_eq!(
                bounded_response(response, 3).await.unwrap_err(),
                GarminError::ResponseTooLarge
            );
            handle.join().unwrap();
        }
    }

    #[tokio::test]
    async fn rejects_invalid_bounds_and_paths_before_contacting_garmin() {
        let client = GarminClient::with_endpoints(
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
        )
        .unwrap();
        for limit in [0, 101] {
            assert_eq!(
                client
                    .list_activities(&tokens(), 0, limit)
                    .await
                    .unwrap_err(),
                GarminError::InvalidRequest
            );
        }
        for id in ["", "0", "01", "../42", "42?secret", "123456789012345678901"] {
            assert_eq!(
                client
                    .read_activity(&tokens(), id, ActivityResource::Summary)
                    .await
                    .unwrap_err(),
                GarminError::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn read_failures_are_sanitized_and_never_retried() {
        for (status, body, expected) in [
            (
                "401 Unauthorized",
                json!({"secret":"private"}),
                GarminError::Authentication,
            ),
            ("403 Forbidden", json!({}), GarminError::Challenge),
            ("429 Too Many Requests", json!({}), GarminError::RateLimited),
            ("302 Found", json!({}), GarminError::Protocol),
            ("200 OK", json!({"activityId":99}), GarminError::Protocol),
            ("200 OK", json!([]), GarminError::Protocol),
        ] {
            let (client, requests) = server(vec![(status, body, "")]);
            assert_eq!(
                client
                    .read_activity(&tokens(), "42", ActivityResource::Summary)
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(requests.join().unwrap().len(), 1);
        }
        let (client, requests) = server(vec![(
            "200 OK",
            json!([{"activityId":1},{"activityId":2}]),
            "",
        )]);
        assert_eq!(
            client.list_activities(&tokens(), 0, 1).await.unwrap_err(),
            GarminError::Protocol
        );
        requests.join().unwrap();
    }

    #[tokio::test]
    async fn signs_in_with_mfa_cookie_and_exchanges_service_ticket() {
        let (client, requests) = server(vec![
            (
                "200 OK",
                json!({"responseStatus":{"type":"MFA_REQUIRED"}, "customerMfaInfo":{"mfaLastMethodUsed":"email"}}),
                "Set-Cookie: mfa=test-session; Path=/\r\n",
            ),
            (
                "200 OK",
                json!({"responseStatus":{"type":"SUCCESSFUL"},"serviceTicketId":"test-ticket"}),
                "",
            ),
            (
                "200 OK",
                json!({"access_token":"test-access", "refresh_token":"test-refresh", "expires_in":3600}),
                "",
            ),
        ]);
        let SignIn::NeedsMfa(pending) = client
            .sign_in("test@example.invalid", "test-password")
            .await
            .unwrap()
        else {
            panic!("expected MFA")
        };
        let tokens = pending.complete("123456").await.unwrap();
        assert!(!tokens.needs_refresh());
        let requests = requests.join().unwrap();
        assert!(requests[0].starts_with("POST /mobile/api/login?"));
        assert!(requests[0].contains("GCM_IOS_DARK"));
        assert!(requests[0].contains("test-password"));
        assert!(requests[1].starts_with("POST /mobile/api/mfa/verifyCode?"));
        assert!(requests[1].contains("mfaVerificationCode"));
        assert!(requests[1].contains("cookie: mfa=test-session"));
        assert!(!requests[1].contains("test-password"));
        assert!(requests[2].contains("service_ticket=test-ticket"));
        assert!(requests[2]
            .contains("service_url=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fios"));
    }

    #[tokio::test]
    async fn signs_in_without_mfa_and_verifies_profile_identity() {
        let (client, requests) = server(vec![
            (
                "200 OK",
                json!({"responseStatus":{"type":"SUCCESSFUL"},"serviceTicketId":"test-ticket"}),
                "",
            ),
            (
                "200 OK",
                json!({"access_token":"test-access", "refresh_token":"test-refresh"}),
                "",
            ),
        ]);
        assert!(matches!(
            client
                .sign_in("test@example.invalid", "test-password")
                .await
                .unwrap(),
            SignIn::Connected(_)
        ));
        assert_eq!(requests.join().unwrap().len(), 2);
        let (client, requests) = server(vec![(
            "200 OK",
            json!({"id":123,"fullName":"Test Rider"}),
            "",
        )]);
        let account = client.account(&tokens()).await.unwrap();
        assert_eq!(account.id, "123");
        assert_eq!(account.display_name.as_deref(), Some("Test Rider"));
        assert!(requests.join().unwrap()[0].contains("authorization: Bearer test-access"));
    }

    #[tokio::test]
    async fn preserves_rotated_refresh_credentials() {
        let (client, requests) = server(vec![(
            "200 OK",
            json!({"access_token":"new-access", "refresh_token":"rotated-refresh", "expires_in":3600}),
            "",
        )]);
        let updated = client.refresh(&tokens()).await.unwrap();
        let saved = serde_json::to_value(&updated).unwrap();
        assert_eq!(saved["refresh_token"], "rotated-refresh");
        assert_eq!(saved["access_token"], "new-access");
        assert!(requests.join().unwrap()[0].contains("refresh_token=test-refresh"));
        let (client, requests) = server(vec![(
            "400 Bad Request",
            json!({"error":"invalid_grant"}),
            "",
        )]);
        assert!(matches!(
            client.refresh(&tokens()).await,
            Err(GarminError::Authentication)
        ));
        requests.join().unwrap();
    }

    #[tokio::test]
    async fn uploads_exact_fit_bytes_once_to_generic_upload_endpoint() {
        for remote_id in [json!(456), json!("456")] {
            let (client, requests) = server(vec![(
                "200 OK",
                json!({"detailedImportResult":{"successes":[{"internalId":remote_id}],"failures":[]}}),
                "",
            )]);
            assert_eq!(
                client
                    .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                    .await
                    .unwrap(),
                "456"
            );
            let requests = requests.join().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].starts_with("POST /upload-service/upload HTTP/1.1"));
            assert!(requests[0].contains("name=\"file\"; filename=\"ride.fit\""));
            assert!(requests[0]
                .as_bytes()
                .windows(SYNTHETIC_FIT.len())
                .any(|part| part == SYNTHETIC_FIT));
            assert!(requests[0].contains("authorization: Bearer test-access"));
        }
    }

    #[tokio::test]
    async fn confirms_asynchronous_upload_without_reposting_fit() {
        let (client, requests) = server(vec![
            (
                "202 Accepted",
                json!({"detailedImportResult":{"successes":[],"failures":[]}}),
                "Location: /activity-service/activity/status/123/test-upload\r\n",
            ),
            ("202 Accepted", json!({}), ""),
            (
                "201 Created",
                json!({}),
                "Location: /activity-service/activity/456\r\n",
            ),
        ]);
        assert_eq!(
            client
                .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                .await
                .unwrap(),
            "456"
        );
        let requests = requests.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST "))
                .count(),
            1
        );
        assert!(requests[1..].iter().all(|request| request
            .starts_with("GET /activity-service/activity/status/123/test-upload HTTP/1.1")
            && request.contains("authorization: Bearer test-access")
            && !request.contains("multipart/form-data")));
    }

    #[tokio::test]
    async fn asynchronous_upload_rejects_untrusted_or_missing_locations() {
        for location in [
            "",
            "Location: https://untrusted.invalid/activity-service/activity/status/123/x\r\n",
            "Location: /userprofile-service/socialProfile\r\n",
            "Location: /activity-service/activity/status/\r\n",
        ] {
            let (client, requests) = server(vec![("202 Accepted", json!({}), location)]);
            assert_eq!(
                client
                    .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                    .await,
                Err(GarminError::UploadUncertain)
            );
            assert_eq!(requests.join().unwrap().len(), 1);
        }
        for location in [
            "",
            "Location: https://untrusted.invalid/activity-service/activity/456\r\n",
            "Location: /activity-service/activity/0\r\n",
            "Location: /activity-service/activity/status/123/x\r\n",
        ] {
            let (client, requests) = server(vec![
                (
                    "202 Accepted",
                    json!({}),
                    "Location: /activity-service/activity/status/123/x\r\n",
                ),
                ("201 Created", json!({}), location),
            ]);
            assert_eq!(
                client
                    .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                    .await,
                Err(GarminError::UploadUncertain)
            );
            assert_eq!(requests.join().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn asynchronous_upload_status_failures_and_exhaustion_do_not_repost() {
        for (status, expected) in [
            ("404 Not Found", GarminError::UploadUncertain),
            ("409 Conflict", GarminError::Duplicate),
        ] {
            let (client, requests) = server(vec![
                (
                    "202 Accepted",
                    json!({}),
                    "Location: /activity-service/activity/status/123/x\r\n",
                ),
                (status, json!({}), ""),
            ]);
            assert_eq!(
                client
                    .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                    .await,
                Err(expected)
            );
            assert_eq!(requests.join().unwrap().len(), 2);
        }
        let mut responses = vec![(
            "202 Accepted",
            json!({}),
            "Location: /activity-service/activity/status/123/x\r\n",
        )];
        responses.extend((0..UPLOAD_STATUS_CHECKS).map(|_| ("202 Accepted", json!({}), "")));
        let (client, requests) = server(responses);
        assert_eq!(
            client
                .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                .await,
            Err(GarminError::UploadUncertain)
        );
        let requests = requests.join().unwrap();
        assert_eq!(requests.len(), 1 + UPLOAD_STATUS_CHECKS);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST "))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn upload_failures_never_confirm_or_resend_the_activity() {
        for (status, body, expected) in [
            ("401 Unauthorized", json!({}), GarminError::Authentication),
            ("403 Forbidden", json!({}), GarminError::Challenge),
            ("409 Conflict", json!({}), GarminError::Duplicate),
            ("429 Too Many Requests", json!({}), GarminError::RateLimited),
            ("400 Bad Request", json!({}), GarminError::UploadRejected),
            (
                "412 Precondition Failed",
                json!({}),
                GarminError::UploadConsentRequired,
            ),
            (
                "500 Internal Server Error",
                json!({}),
                GarminError::UploadUncertain,
            ),
            ("200 OK", json!({}), GarminError::UploadUncertain),
            (
                "200 OK",
                json!({"detailedImportResult":{"uploadId":123,"successes":[],"failures":[]}}),
                GarminError::UploadUncertain,
            ),
            (
                "200 OK",
                json!({"detailedImportResult":{"successes":[{}],"failures":[]}}),
                GarminError::UploadUncertain,
            ),
            (
                "200 OK",
                json!({"detailedImportResult":{"successes":[{"internalId":1},{"internalId":2}],"failures":[]}}),
                GarminError::UploadUncertain,
            ),
            (
                "200 OK",
                json!({"detailedImportResult":{"successes":[],"failures":[{"messages":null}]}}),
                GarminError::UploadRejected,
            ),
            (
                "200 OK",
                json!({"detailedImportResult":{"successes":[],"failures":[{"internalId":123,"messages":[{"code":DUPLICATE_ACTIVITY_MESSAGE_CODE}]}]}}),
                GarminError::Duplicate,
            ),
        ] {
            let (client, requests) = server(vec![(status, body, "")]);
            assert_eq!(
                client
                    .upload(&tokens(), SYNTHETIC_FIT.to_vec(), "ride.fit")
                    .await,
                Err(expected)
            );
            assert_eq!(requests.join().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn bad_mfa_can_retry_and_expired_mfa_does_not_make_a_request() {
        let (client, requests) = server(vec![
            (
                "200 OK",
                json!({"responseStatus":{"type":"MFA_REQUIRED"}}),
                "",
            ),
            (
                "200 OK",
                json!({"responseStatus":{"type":"INVALID_MFA_CODE"}}),
                "",
            ),
        ]);
        let SignIn::NeedsMfa(mut pending) = client
            .sign_in("test@example.invalid", "test-password")
            .await
            .unwrap()
        else {
            panic!("expected MFA")
        };
        assert!(matches!(
            pending.complete("bad-code").await,
            Err(GarminError::MfaRejected)
        ));
        pending.started_at = Instant::now() - MFA_LIFETIME;
        assert!(matches!(
            pending.complete("123456").await,
            Err(GarminError::MfaExpired)
        ));
        assert_eq!(requests.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn login_challenges_rate_limits_and_bad_passwords_are_sanitized() {
        for (body, expected) in [
            (
                json!({"responseStatus":{"type":"CAPTCHA_REQUIRED"}}),
                GarminError::Challenge,
            ),
            (
                json!({"error":{"status-code":"429"}}),
                GarminError::RateLimited,
            ),
            (
                json!({"responseStatus":{"type":"INVALID_USERNAME_PASSWORD"}}),
                GarminError::Authentication,
            ),
            (json!({"secret":"must-not-leak"}), GarminError::Protocol),
        ] {
            let (client, requests) = server(vec![("200 OK", body, "")]);
            let result = client
                .sign_in("test@example.invalid", "test-password")
                .await;
            assert!(matches!(result, Err(error) if error == expected));
            requests.join().unwrap();
            assert!(!expected.to_string().contains("must-not-leak"));
        }
    }
}
