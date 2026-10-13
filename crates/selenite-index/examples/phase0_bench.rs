//! Reproducible benchmark. Never deletes or modifies existing vault sources.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, bail, ensure};
use selenite_core::vault::Vault;
use selenite_index::{Index, path_key};

const NOTES: usize = 5_000;
const QUERIES: &[&str] = &[
    "selenite",
    "astronomy",
    "\"local knowledge\"",
    "title:Note",
    "unique00420",
    "astronomy AND tasks",
];

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|value| value.to_str()) {
        Some("generate") if args.len() == 2 => generate(Path::new(&args[1])),
        Some("measure") if args.len() == 3 || args.len() == 4 => measure(
            Path::new(&args[1]),
            Path::new(&args[2]),
            args.get(3).map(Path::new),
        ),
        _ => bail!(
            "usage: phase0_bench generate <new-vault> | measure <vault> <snapshot.tsv> [expected.tsv]"
        ),
    }
}

fn note_path(number: usize) -> PathBuf {
    PathBuf::from(format!("Notes-{:02}", number / 100)).join(format!("Note-{number:05}.md"))
}

fn note_source(number: usize) -> String {
    let topic = [
        "astronomy",
        "gardening",
        "rust",
        "history",
        "music",
        "design",
        "science",
        "travel",
    ][number % 8];
    let next = path_key(&note_path((number + 1) % NOTES).with_extension(""));
    let mut source = format!(
        "---\nkind: benchmark\nordinal: {number}\ntags: [{topic}, notes]\ncomplete: false\nrating: 4.5\n---\n# Note {number:05}\n\nSelenite keeps local knowledge in ordinary Markdown files.\nThis {topic} note contains unique{number:05} and links to [[{next}]]. ^summary\n\n## Tasks\n\n- [ ] Review the evidence\n- [x] Keep the original source\n\n"
    );
    for section in 0..6 {
        source.push_str(&format!("Paragraph {section}: local knowledge connects {topic}, projects, tasks and ideas. A rebuild reads the vault without changing its notes. Café and 東京 remain UTF-8 text.\n\n"));
    }
    source.push_str("```text\n[[This-is-code]]\n```\n");
    source
}

fn generate(root: &Path) -> Result<()> {
    ensure!(
        !root.exists(),
        "fixture destination must not exist: {}",
        root.display()
    );
    let started = Instant::now();
    fs::create_dir_all(root)?;
    for number in 0..NOTES {
        let path = root.join(note_path(number));
        fs::create_dir_all(path.parent().context("note parent")?)?;
        fs::write(path, note_source(number))?;
    }
    println!(
        "Generated {NOTES} Markdown notes in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn sources(vault: &Vault) -> Result<BTreeMap<PathBuf, String>> {
    vault
        .markdown_paths()?
        .into_iter()
        .map(|path| {
            let source = vault.read_source(&path)?;
            Ok((path, source))
        })
        .collect()
}

fn measure(root: &Path, output: &Path, expected: Option<&Path>) -> Result<()> {
    let vault = Vault::open(root)?;
    let before = sources(&vault)?;
    ensure!(
        before.len() == NOTES,
        "expected {NOTES} notes, found {}",
        before.len()
    );
    for number in 0..NOTES {
        ensure!(
            before.get(&note_path(number)) == Some(&note_source(number)),
            "fixture source changed: {}",
            note_path(number).display()
        );
    }
    let opened = Instant::now();
    let index = Index::open(root)?;
    ensure!(index.is_ready()?, "run selenite rebuild before measuring");
    let mut metrics = format!(
        "notes={NOTES}\ncache_open_ms={:.3}\nprofile=release\nsamples_per_query=50\n",
        opened.elapsed().as_secs_f64() * 1000.0
    );
    let mut snapshot = String::new();
    let mut slowest_p95: f64 = 0.0;
    for query in QUERIES {
        for _ in 0..5 {
            index.search(query, NOTES)?;
        }
        let mut samples = Vec::new();
        for _ in 0..50 {
            let start = Instant::now();
            std::hint::black_box(index.search(query, NOTES)?);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        let hits = index.search(query, NOTES)?;
        slowest_p95 = slowest_p95.max(samples[47]);
        metrics.push_str(&format!(
            "query={query:?} hits={} p50_ms={:.3} p95_ms={:.3} max_ms={:.3}\n",
            hits.len(),
            samples[25],
            samples[47],
            samples[49]
        ));
        for (rank, hit) in hits.iter().enumerate() {
            snapshot.push_str(&format!(
                "{query}\t{rank}\t{}\t{}\t{}\n",
                path_key(&hit.path),
                hit.title,
                hit.score.to_bits()
            ));
        }
    }
    if let Some(expected) = expected {
        ensure!(
            fs::read_to_string(expected)? == snapshot,
            "ranked search results or scores differ from baseline"
        );
        metrics.push_str("baseline_comparison=identical_paths_titles_scores_and_order\n");
    }
    ensure!(
        before == sources(&vault)?,
        "benchmark altered Markdown files"
    );
    metrics.push_str("source_bytes=unchanged\n");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, snapshot)?;
    fs::write(output.with_extension("metrics.txt"), &metrics)?;
    print!("{metrics}");
    ensure!(
        slowest_p95 < 100.0,
        "search p95 target missed: {slowest_p95:.3} ms"
    );
    Ok(())
}
