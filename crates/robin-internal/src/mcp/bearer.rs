/// NewBearerHTTPClient returns a reqwest::Client whose middleware attaches an
/// Authorization: Bearer <token> header to every outgoing request, unless the
/// caller has already set Authorization.
///
/// An empty token disables injection — the returned client behaves like the
/// default client. The resolver is expected to skip empty-token servers before
/// reaching this point; this defensive no-op exists so a programming error
/// doesn't crash the gateway.
pub fn new_bearer_http_client(token: String) -> reqwest::Client {
    // reqwest doesn't expose a RoundTripper equivalent, so we use a middleware
    // approach: wrap the reqwest::Client with a custom reqwest_middleware stack,
    // or — simpler here — build a reqwest::Client with a custom ClientBuilder.
    //
    // Since reqwest doesn't support per-request middleware out of the box
    // without reqwest-middleware, we implement bearer injection via a
    // BearerMiddleware that wraps the default client. We expose this as a
    // newtype that implements Deref<Target = reqwest::Client> so callers can
    // use it transparently.
    //
    // For this translation we use a thin wrapper that is exposed via
    // `BearerClient` — a struct that holds the token and delegates to a
    // reqwest::Client, injecting the header before each send.

    // Simpler approach: build a client with a default header when token is non-empty.
    let mut builder = reqwest::Client::builder();
    if !token.is_empty() {
        let mut headers = reqwest::header::HeaderMap::new();
        let value = format!("Bearer {}", token);
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&value) {
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
        builder = builder.default_headers(headers);
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// BearerClient is a thin wrapper that injects an Authorization: Bearer header
/// on requests that don't already have one. Mirrors Go's bearerRoundTripper.
///
/// NOTE: Since reqwest's `default_headers` always injects the header (even
/// when the caller sets their own), we expose BearerClient for callers that
/// need the "don't overwrite existing Authorization" behavior. Most callers
/// (MCP servers with static tokens) can use `new_bearer_http_client` directly.
pub struct BearerClient {
    token: String,
    inner: reqwest::Client,
}

impl BearerClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            inner: reqwest::Client::new(),
        }
    }

    /// Build a reqwest::RequestBuilder with Bearer injection applied.
    /// If the request already has an Authorization header this is a no-op.
    pub async fn send(
        &self,
        mut request: reqwest::Request,
    ) -> anyhow::Result<reqwest::Response> {
        if !self.token.is_empty()
            && !request.headers().contains_key(reqwest::header::AUTHORIZATION)
        {
            let value = format!("Bearer {}", self.token);
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&value) {
                request.headers_mut().insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        self.inner.execute(request).await
            .map_err(|e| anyhow::anyhow!("bearer send: {}", e))
    }
}