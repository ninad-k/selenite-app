# Ask your vault

AI is opt-in and provider-independent behind `LlmProvider` and
`EmbeddingProvider`. The included HTTP adapter speaks the OpenAI-compatible
chat-completions and float-embeddings formats. A local compatible server may use
HTTP on loopback; other endpoints require HTTPS. Redirects are disabled, requests
time out, responses are bounded, and provider error bodies are not logged.

Create `.selenite/ai.toml` in the vault (or use Services & settings):

```toml
enabled = false
base_url = "http://127.0.0.1:1234/v1/"
chat_model = "your-chat-model"
embedding_model = "your-embedding-model"
max_context_chars = 12000
folders = ["Projects", "Journal"]
# api_key_env = "SELENITE_LLM_KEY"
```

Use model names provided by your configured service. Set `enabled = true` when
ready. If authentication is required, set the named environment variable before
launching Selenite; never put the actual key in this file, Markdown, frontend
storage, or the index. An empty `folders` list includes the entire vault.

`selenite ai-index <vault>` explicitly sends scoped source chunks to the embedding
provider. It creates `.selenite/cache/ai-vectors.json`, containing model identity,
source fingerprints and vectors. It never stores the only copy of source text.
The cache can be rebuilt from Markdown plus the configured embedding service.
Changing model identity requires rebuilding vectors.

`selenite ask <vault> "<question>"` rereads notes, refreshes BM25 search, and combines
BM25 and cosine-vector ranks using reciprocal-rank fusion. Stale source hashes
are excluded, so an old vector cannot supply deleted or edited source text.
Without a compatible cache, retrieval uses BM25. It sends the question and up to
eight bounded source excerpts to the chat provider. With a vector cache, the
question is also sent to the embedding provider. Desktop buttons make these
transfers explicit; opening a vault or settings makes no model request.

Answers include numbered citations with vault-relative paths, original line
ranges, source hashes and excerpts. Note content is delimited as untrusted data.
The response must cite only retrieved source IDs. This validates citation
identity, not the truth of every generated claim; the cited Markdown remains the
evidence the reader can inspect. AI never edits a note or invokes another tool.

The JSON cache and brute-force cosine scan are intentionally small and portable.
There is no vector database service, hidden model download, or mandatory account.
Provider transport and retrieval tests run locally with deterministic model
doubles and a loopback HTTP server. Real provider credentials, model availability,
cost and answer quality have not been validated against a live account.

Protocol references: [chat API](https://developers.openai.com/api/reference/resources/chat)
and [embeddings API](https://developers.openai.com/api/reference/resources/embeddings/methods/create).
