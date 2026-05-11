use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Converts text into float vectors for semantic search.
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
}

/// Known OpenAI embedding model names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EmbeddingModel {
    TextEmbeddingAda002,
    TextEmbedding3Small,
    TextEmbedding3Large,
    Custom(String),
}

impl EmbeddingModel {
    fn as_str(&self) -> &str {
        match self {
            Self::TextEmbeddingAda002 => "text-embedding-ada-002",
            Self::TextEmbedding3Small => "text-embedding-3-small",
            Self::TextEmbedding3Large => "text-embedding-3-large",
            Self::Custom(s) => s.as_str(),
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "text-embedding-3-small" => Self::TextEmbedding3Small,
            "text-embedding-3-large" => Self::TextEmbedding3Large,
            "text-embedding-ada-002" | "" => Self::TextEmbeddingAda002,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// Uses the OpenAI embeddings API (or any OpenAI-compatible endpoint such as
/// LiteLLM) to produce text embeddings.
pub struct OpenAiEmbedder {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: EmbeddingModel,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    input: &'a [String],
    model: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl OpenAiEmbedder {
    /// Creates an Embedder backed by an OpenAI-compatible embeddings endpoint.
    /// If `base_url` is empty the official OpenAI API is used.
    /// If `model` is empty or unrecognised it defaults to `text-embedding-ada-002`.
    pub fn new(api_key: &str, base_url: &str, model: &str) -> Self {
        let base_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };

        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url,
            model: EmbeddingModel::from_str(model),
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url);

        let req = EmbeddingRequest {
            input: &texts,
            model: self.model.as_str(),
        };

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .context("embedding request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("embedding request failed: status={} body={}", status, body);
        }

        let body: EmbeddingResponse = resp.json().await.context("decode embedding response")?;

        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }
}

#[cfg(test)]
#[path = "embedder_test.rs"]
mod tests;