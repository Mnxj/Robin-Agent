/// ClientCredentialsConfig is the minimal config needed to perform an OAuth2
/// client-credentials grant against a token endpoint (e.g. AWS Cognito).
#[derive(Debug, Clone)]
pub struct ClientCredentialsConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// single scope; multi-scope can be space-separated
    pub scope: String,
}

/// OAuthToken is a cached OAuth2 token.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: String,
    /// Token expiry (RFC3339 format). None means long-lived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<chrono::DateTime<chrono::Utc>>,
    /// Expiry as Unix timestamp seconds (from JSON `expires_in`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_unix: Option<i64>,
}

impl OAuthToken {
    /// tokenUsable returns true when the access token exists and isn't due to
    /// expire within 30 seconds. The 30s buffer avoids a race where a request
    /// lands just as the token expires server-side.
    pub fn is_usable(&self) -> bool {
        if self.access_token.is_empty() {
            return false;
        }
        match self.expiry {
            None => true, // long-lived token
            Some(exp) => {
                let now = chrono::Utc::now();
                exp - now > chrono::Duration::seconds(30)
            }
        }
    }
}

/// ClientCredentialsHTTPClient holds the state needed to inject an OAuth2
/// bearer token obtained via the client-credentials grant into every request,
/// auto-refreshing when the token nears expiry.
pub struct ClientCredentialsClient {
    cfg: ClientCredentialsConfig,
    token: tokio::sync::Mutex<Option<OAuthToken>>,
    inner: reqwest::Client,
}

impl ClientCredentialsClient {
    /// Create a new ClientCredentialsClient. No network call is made here;
    /// the first request will obtain a token.
    pub fn new(cfg: ClientCredentialsConfig) -> Self {
        Self {
            cfg,
            token: tokio::sync::Mutex::new(None),
            inner: reqwest::Client::new(),
        }
    }

    /// Fetch a fresh token from the token endpoint using the client-credentials grant.
    async fn fetch_token(&self) -> anyhow::Result<OAuthToken> {
        let mut params = vec![
            ("grant_type", "client_credentials"),
            ("client_id", self.cfg.client_id.as_str()),
            ("client_secret", self.cfg.client_secret.as_str()),
        ];
        // Scope is optional.
        let scope_owned;
        if !self.cfg.scope.is_empty() {
            scope_owned = self.cfg.scope.clone();
            params.push(("scope", scope_owned.as_str()));
        }

        let resp = self.inner.post(&self.cfg.token_url)
            .basic_auth(&self.cfg.client_id, Some(&self.cfg.client_secret))
            .form(&params)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("fetch token: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("token endpoint {}: {}", status, body));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| anyhow::anyhow!("decode token response: {}", e))?;

        let access_token = body.get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))?
            .to_owned();
        let token_type = body.get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_owned();
        let refresh_token = body.get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let expiry = body.get("expires_in")
            .and_then(|v| v.as_i64())
            .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs));

        Ok(OAuthToken { access_token, refresh_token, token_type, expiry, expiry_unix: None })
    }

    /// Get a valid access token, refreshing if necessary.
    pub async fn token(&self) -> anyhow::Result<String> {
        let mut guard = self.token.lock().await;
        if let Some(t) = guard.as_ref() {
            if t.is_usable() {
                return Ok(t.access_token.clone());
            }
        }
        let fresh = self.fetch_token().await?;
        let tok = fresh.access_token.clone();
        *guard = Some(fresh);
        Ok(tok)
    }

    /// Execute a request with Bearer token injection.
    pub async fn execute(&self, request: reqwest::Request) -> anyhow::Result<reqwest::Response> {
        let tok = self.token().await?;
        let mut req = request;
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", tok)) {
            req.headers_mut().insert(reqwest::header::AUTHORIZATION, v);
        }
        self.inner.execute(req).await
            .map_err(|e| anyhow::anyhow!("client credentials send: {}", e))
    }
}

/// new_client_credentials_http_client returns a reqwest::Client whose default
/// headers include a Bearer token obtained via the client-credentials grant.
///
/// This is a synchronous-friendly approach: the token is fetched eagerly
/// before returning. For a fully lazy approach, use ClientCredentialsClient
/// directly.
pub async fn new_client_credentials_http_client(
    cfg: ClientCredentialsConfig,
) -> anyhow::Result<reqwest::Client> {
    let client = ClientCredentialsClient::new(cfg);
    let token = client.token().await?;
    // Build a reqwest Client with the token as a default header.
    let mut headers = reqwest::header::HeaderMap::new();
    let value = format!("Bearer {}", token);
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&value) {
        headers.insert(reqwest::header::AUTHORIZATION, v);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| anyhow::anyhow!("build http client: {}", e))
}