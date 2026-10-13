use std::{io::Read, time::Duration};

use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use selenite_core::vault::Vault;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    /// OpenAI-compatible API root, e.g. https://provider.example/v1/.
    pub base_url: String,
    pub chat_model: String,
    pub embedding_model: String,
    /// Name of an environment variable, never an API key itself.
    pub api_key_env: Option<String>,
    #[serde(default = "default_context")]
    pub max_context_chars: usize,
    #[serde(default)]
    pub folders: Vec<String>,
}

fn default_context() -> usize {
    12_000
}

impl AiConfig {
    pub fn load(vault: &Vault) -> Result<Self> {
        let source = vault
            .read_metadata("ai.toml")?
            .context("Configure .selenite/ai.toml before using AI")?;
        let config: Self = toml::from_str(&source)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        endpoint(&self.base_url)?;
        ensure!(
            !self.chat_model.trim().is_empty() && !self.embedding_model.trim().is_empty(),
            "model names must not be empty"
        );
        ensure!(
            (1000..=100_000).contains(&self.max_context_chars),
            "max_context_chars must be between 1000 and 100000"
        );
        if let Some(name) = &self.api_key_env {
            ensure!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                "api_key_env must name an environment variable"
            );
        }
        for folder in &self.folders {
            selenite_core::validate_page_path(&std::path::Path::new(folder).join("scope.md"))?;
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}",
            self.base_url.trim_end_matches('/'),
            self.embedding_model
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub trait LlmProvider {
    fn name(&self) -> &str;
    fn complete(&self, messages: &[ChatMessage]) -> Result<String>;
}

pub trait EmbeddingProvider {
    fn fingerprint(&self) -> String;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// A bounded HTTP transport without redirect-following or persisted credentials.
pub struct HttpProvider {
    config: AiConfig,
    client: Client,
    key: Option<String>,
    base: Url,
}

impl HttpProvider {
    pub fn new(config: AiConfig) -> Result<Self> {
        config.validate()?;
        ensure!(config.enabled, "AI is disabled in .selenite/ai.toml");
        let key = config
            .api_key_env
            .as_ref()
            .map(|name| {
                std::env::var(name).with_context(|| format!("Set the {name} environment variable"))
            })
            .transpose()?;
        ensure!(
            key.as_ref().is_none_or(|key| !key.is_empty()),
            "configured API key is empty"
        );
        let base = endpoint(&config.base_url)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            config,
            client,
            key,
            base,
        })
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let mut request = self.client.post(self.base.join(path)?).json(&body);
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        let response = request.send().map_err(|_| {
            anyhow::anyhow!("model provider request failed (connection or timeout)")
        })?;
        ensure!(
            response.status().is_success(),
            "model provider returned HTTP {}",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        response.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "model provider response exceeds 8 MiB"
        );
        serde_json::from_slice(&bytes).context("model provider returned invalid JSON")
    }
}

impl LlmProvider for HttpProvider {
    fn name(&self) -> &str {
        &self.config.chat_model
    }

    fn complete(&self, messages: &[ChatMessage]) -> Result<String> {
        let value = self.post(
            "chat/completions",
            json!({"model":self.config.chat_model,"messages":messages,"stream":false}),
        )?;
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .context("provider omitted assistant text")?;
        ensure!(!text.trim().is_empty(), "provider returned an empty answer");
        Ok(text.to_owned())
    }
}

impl EmbeddingProvider for HttpProvider {
    fn fingerprint(&self) -> String {
        self.config.fingerprint()
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(
            texts.len() <= 32,
            "embedding batches are limited to 32 texts"
        );
        let value = self.post(
            "embeddings",
            json!({"model":self.config.embedding_model,"input":texts,"encoding_format":"float"}),
        )?;
        let values = value["data"]
            .as_array()
            .context("provider omitted embedding data")?;
        ensure!(
            values.len() == texts.len(),
            "embedding count does not match input count"
        );
        let mut ordered = vec![None; texts.len()];
        for entry in values {
            let index =
                usize::try_from(entry["index"].as_u64().context("embedding index missing")?)?;
            ensure!(
                index < ordered.len() && ordered[index].is_none(),
                "duplicate or invalid embedding index"
            );
            let vector: Vec<f32> = serde_json::from_value(entry["embedding"].clone())?;
            validate_vector(&vector)?;
            ordered[index] = Some(vector);
        }
        ordered
            .into_iter()
            .map(|vector| vector.context("missing embedding"))
            .collect()
    }
}

pub(crate) fn validate_vector(vector: &[f32]) -> Result<()> {
    ensure!(
        !vector.is_empty() && vector.len() <= 65_536,
        "invalid embedding dimensions"
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "non-finite embedding value"
    );
    ensure!(
        vector.iter().any(|value| *value != 0.0),
        "zero embedding vector"
    );
    Ok(())
}

fn endpoint(value: &str) -> Result<Url> {
    let mut url = Url::parse(value)?;
    let local = url
        .host_str()
        .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "AI endpoints require HTTPS (HTTP is allowed only on loopback)"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "endpoint must not contain credentials, query or fragment"
    );
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}
