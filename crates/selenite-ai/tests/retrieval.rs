use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use selenite_ai::{
    AiConfig, ChatMessage, EmbeddingProvider, LlmProvider, ask, rebuild_embeddings, retrieve,
};
use selenite_core::vault::Vault;

struct Models {
    answer: &'static str,
}
impl EmbeddingProvider for Models {
    fn fingerprint(&self) -> String {
        "test-semantic-v1".to_owned()
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        assert!(
            texts.iter().all(|text| text.chars().count() < 2000),
            "source chunk exceeded its target bound"
        );
        Ok(texts
            .iter()
            .map(|text| {
                let text = text.to_lowercase();
                if text.contains("automobile") || text.contains("car") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
}
impl LlmProvider for Models {
    fn name(&self) -> &str {
        "test-model"
    }
    fn complete(&self, messages: &[ChatMessage]) -> Result<String> {
        assert!(messages[0].content.contains("untrusted"));
        assert!(messages[1].content.contains("Source records"));
        Ok(self.answer.to_owned())
    }
}

fn fixture() -> (Vault, AiConfig) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".local/tests/ai")
        .join(format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("Transport.md"),
        "# Travel\n\nAn automobile carries passengers.\n",
    )
    .unwrap();
    fs::write(
        root.join("Garden.md"),
        "# Garden\n\nPlant rosemary and thyme.\n",
    )
    .unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: "http://127.0.0.1:1234/v1".to_owned(),
        chat_model: "chat".to_owned(),
        embedding_model: "embed".to_owned(),
        api_key_env: None,
        max_context_chars: 12_000,
        folders: Vec::new(),
    };
    (Vault::open(root).unwrap(), config)
}

#[test]
fn semantic_retrieval_rebuilds_deterministically_and_cites_disk() {
    let (vault, config) = fixture();
    let model = Models {
        answer: "Passengers travel by car [S1].",
    };
    let stats = rebuild_embeddings(&vault, &config, &model).unwrap();
    assert_eq!(stats.pages, 2);
    assert_eq!(stats.dimensions, 2);
    let results = retrieve(&vault, &config, &model, "car", 2).unwrap();
    assert_eq!(results[0].path, "Transport.md");
    assert!(results[0].excerpt.contains("automobile"));
    assert_eq!(results[0].start_line, 1);
    let cache = vault
        .read_metadata("cache/ai-vectors.json")
        .unwrap()
        .unwrap();
    assert!(!cache.contains("passengers"));
    rebuild_embeddings(&vault, &config, &model).unwrap();
    assert_eq!(
        vault
            .read_metadata("cache/ai-vectors.json")
            .unwrap()
            .unwrap(),
        cache
    );
    let answer = ask(&vault, &config, &model, "car").unwrap();
    assert!(answer.text.contains("[S1]"));
    assert_eq!(answer.citations[0].path, "Transport.md");
}

#[test]
fn changed_notes_cannot_leak_stale_vector_context() {
    let (vault, config) = fixture();
    let model = Models {
        answer: "Answer [S99].",
    };
    rebuild_embeddings(&vault, &config, &model).unwrap();
    let previous = vault.read_source("Transport.md").unwrap();
    vault
        .write_page(
            "Transport.md",
            &previous,
            "# Transport\n\nA brand new bicycle.\n",
        )
        .unwrap();
    let results = retrieve(&vault, &config, &model, "car", 20).unwrap();
    assert!(
        results
            .iter()
            .all(|citation| !citation.excerpt.contains("automobile"))
    );
    assert!(
        results
            .iter()
            .all(|citation| citation.path != "Transport.md")
    );
    assert!(
        ask(&vault, &config, &model, "rosemary")
            .unwrap_err()
            .to_string()
            .contains("not retrieved")
    );
}

#[test]
fn opt_in_scope_and_provider_validation() {
    let (vault, mut config) = fixture();
    let model = Models { answer: "[S1]" };
    config.enabled = false;
    assert!(rebuild_embeddings(&vault, &config, &model).is_err());
    config.enabled = true;
    config.folders = vec!["Private".to_owned()];
    assert_eq!(
        rebuild_embeddings(&vault, &config, &model).unwrap().chunks,
        0
    );
    assert!(
        ask(&vault, &config, &model, "car")
            .unwrap()
            .citations
            .is_empty()
    );
    config.base_url = "http://example.com/v1".to_owned();
    assert!(config.validate().is_err());
    config.base_url = "https://secret@example.com/v1".to_owned();
    assert!(config.validate().is_err());
}

#[test]
fn blank_lines_are_bounded_and_unterminated_citations_are_rejected() {
    let (vault, config) = fixture();
    vault
        .create_page(
            "Spacing.md",
            &format!("# Spacing\n{}A car.\n", "\n".repeat(6000)),
        )
        .unwrap();
    let model = Models {
        answer: "A car [S1",
    };
    rebuild_embeddings(&vault, &config, &model).unwrap();
    assert!(
        ask(&vault, &config, &model, "car")
            .unwrap_err()
            .to_string()
            .contains("unterminated")
    );
}
