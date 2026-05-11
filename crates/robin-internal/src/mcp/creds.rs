use std::io::{BufRead, BufReader};
use std::path::Path;

/// OAuthTokenStore is a persisted OAuth2 token. Fields map directly to the
/// OAuth2 JSON response; Expiry is computed from expires_in and stored as
/// an absolute UTC timestamp.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthTokenStore {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: String,
    /// Absolute expiry as a UTC datetime. None means the token is long-lived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<chrono::DateTime<chrono::Utc>>,
}

impl OAuthTokenStore {
    /// Returns true when the access token exists and isn't due to expire
    /// within 30 seconds. The 30s buffer matches the oauth2 library's
    /// internal expiryDelta and avoids a race where a request lands just
    /// as the token expires server-side.
    pub fn is_usable(&self) -> bool {
        if self.access_token.is_empty() {
            return false;
        }
        match self.expiry {
            None => true,
            Some(exp) => {
                let now = chrono::Utc::now();
                (exp - now) > chrono::Duration::seconds(30)
            }
        }
    }
}

/// load_token reads a persisted OAuthTokenStore. Returns (None) when the file
/// doesn't exist — that's the "first start, no cached token" case and isn't
/// an error.
pub fn load_token(path: &str) -> anyhow::Result<Option<OAuthTokenStore>> {
    if path.is_empty() {
        return Err(anyhow::anyhow!("token store path is empty"));
    }
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::anyhow!("{}", e)),
    };
    let tok: OAuthTokenStore = serde_json::from_slice(&data)
        .map_err(|e| anyhow::anyhow!("decode token file {}: {}", path, e))?;
    Ok(Some(tok))
}

/// save_token writes the token to path with mode 0600, creating the parent
/// directory at 0700 if needed. Atomic via write-temp-then-rename so a crash
/// mid-write can't leave a corrupt file that breaks the next start.
pub fn save_token(path: &str, tok: &OAuthTokenStore) -> anyhow::Result<()> {
    if path.is_empty() {
        return Err(anyhow::anyhow!("token store path is empty"));
    }
    let dir = Path::new(path)
        .parent()
        .ok_or_else(|| anyhow::anyhow!("token store path has no parent directory"))?;

    // Create parent dir with mode 0700.
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("create token dir: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| anyhow::anyhow!("chmod token dir: {}", e))?;
    }

    let data = serde_json::to_vec_pretty(tok)
        .map_err(|e| anyhow::anyhow!("encode token: {}", e))?;

    // Write to a temp file then rename atomically.
    let tmp_path = format!("{}.tmp-{}", path, std::process::id());
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .map_err(|e| anyhow::anyhow!("create temp file: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| anyhow::anyhow!("chmod temp file: {}", e))?;
        }
        f.write_all(&data)
            .map_err(|e| anyhow::anyhow!("write temp file: {}", e))?;
    }

    std::fs::rename(&tmp_path, path)
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            anyhow::anyhow!("rename token file: {}", e)
        })?;

    Ok(())
}

// ── LoadEnvFile / RequireKeys ─────────────────────────────────────────────────

/// LoadEnvFile reads a minimal dotenv-style file and returns its KEY=VALUE
/// pairs as a map. Lines that are blank or start with '#' are skipped. Value
/// is everything after the first '=', with surrounding whitespace trimmed.
/// No shell expansion, no quoting rules.
pub fn load_env_file(path: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let f = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("open env file: {}", e))?;
    let reader = BufReader::new(f);
    let mut out = std::collections::HashMap::new();
    let mut line_no = 0usize;
    for line_result in reader.lines() {
        line_no += 1;
        let line = line_result.map_err(|e| anyhow::anyhow!("read env file: {}", e))?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let eq = line.find('=').ok_or_else(|| {
            anyhow::anyhow!("env file {}: line {}: missing '='", path, line_no)
        })?;
        let key = line[..eq].trim();
        if key.is_empty() {
            return Err(anyhow::anyhow!("env file {}: line {}: empty key", path, line_no));
        }
        let val = line[eq + 1..].trim();
        out.insert(key.to_owned(), val.to_owned());
    }
    Ok(out)
}

/// RequireKeys returns an error listing any missing keys from the supplied env
/// map. Used by the harness to fail fast with a clear message.
pub fn require_keys(
    env: &std::collections::HashMap<String, String>,
    keys: &[&str],
) -> anyhow::Result<()> {
    let mut missing: Vec<&str> = Vec::new();
    for &k in keys {
        if !env.contains_key(k) {
            missing.push(k);
        }
    }
    if !missing.is_empty() {
        return Err(anyhow::anyhow!("missing required env keys: {}", missing.join(", ")));
    }
    Ok(())
}