use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail, ensure};
use selenite_core::{LinkResolver, validate_page_path};
use selenite_index::{Index, path_key};
mod services;

const HELP: &str = "Selenite — Markdown vault tools

Usage:
  selenite rebuild <vault>
  selenite search <vault> \"<query>\"
  selenite backlinks <vault> <page>
  selenite mcp <vault> [--allow-writes]
  selenite ai-index <vault>
  selenite ask <vault> \"<question>\"
  selenite views <vault>
  selenite query <vault> <view-id>
  selenite init-gtd <vault>
  selenite recover-renames <vault>
  selenite sync-plan <vault>
  selenite sync-run <vault> <reviewed-plan.json>
  selenite plugins <vault>
  selenite catalog <vault>
  selenite install-plugin <vault> <id>
  selenite run-plugin <vault> <id> <input-json>
  selenite pro-status <vault>
  selenite --help

Search returns score, vault-relative path, and title separated by tabs.
Backlinks returns source paths and optional fragments separated by tabs.
Fields escape backslashes, tabs, and newlines. No match produces no output.
Page paths are relative to the vault; bare page names must be unambiguous.
All commands derive their results from Markdown files on disk.
";

enum Command {
    Help,
    Rebuild(PathBuf),
    Search { vault: PathBuf, query: String },
    Backlinks { vault: PathBuf, page: String },
    Mcp { vault: PathBuf, allow_writes: bool },
    AiIndex(PathBuf),
    Ask { vault: PathBuf, question: String },
    Views(PathBuf),
    Query { vault: PathBuf, view: String },
    InitGtd(PathBuf),
    Service(services::Command),
}

fn main() -> ExitCode {
    let result = (|| {
        let command = parse_args(env::args_os().skip(1).collect())?;
        execute(command, &mut io::stdout().lock())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(arguments: Vec<OsString>) -> Result<Command> {
    if let Some(command) = services::parse(&arguments)? {
        return Ok(Command::Service(command));
    }
    match arguments.as_slice() {
        [help] if help == "--help" || help == "-h" => Ok(Command::Help),
        [command, vault] if command == "rebuild" => Ok(Command::Rebuild(vault.into())),
        [command, vault] if command == "mcp" => Ok(Command::Mcp {
            vault: vault.into(),
            allow_writes: false,
        }),
        [command, vault, flag] if command == "mcp" && flag == "--allow-writes" => {
            Ok(Command::Mcp {
                vault: vault.into(),
                allow_writes: true,
            })
        }
        [command, vault] if command == "ai-index" => Ok(Command::AiIndex(vault.into())),
        [command, vault] if command == "views" => Ok(Command::Views(vault.into())),
        [command, vault] if command == "init-gtd" => Ok(Command::InitGtd(vault.into())),
        [command, vault, question] if command == "ask" => Ok(Command::Ask {
            vault: vault.into(),
            question: question
                .to_str()
                .context("question must be Unicode")?
                .to_owned(),
        }),
        [command, vault, view] if command == "query" => Ok(Command::Query {
            vault: vault.into(),
            view: view.to_str().context("view ID must be Unicode")?.to_owned(),
        }),
        [command, vault, query] if command == "search" => Ok(Command::Search {
            vault: vault.into(),
            query: query
                .to_str()
                .context("search query must be valid Unicode")?
                .to_owned(),
        }),
        [command, vault, page] if command == "backlinks" => Ok(Command::Backlinks {
            vault: vault.into(),
            page: page
                .to_str()
                .context("page target must be valid Unicode")?
                .to_owned(),
        }),
        _ => bail!("invalid arguments\n\n{HELP}"),
    }
}

fn execute(command: Command, output: &mut impl Write) -> Result<()> {
    match command {
        Command::Help => output.write_all(HELP.as_bytes())?,
        Command::Service(command) => services::execute(command, output)?,
        Command::Mcp {
            vault,
            allow_writes,
        } => selenite_mcp::Server::new(vault, allow_writes)?.serve(io::stdin().lock(), output)?,
        Command::Views(vault) => json_output(
            output,
            &selenite_db::DatabaseStore::open(vault)?.list_views()?,
        )?,
        Command::Query { vault, view } => json_output(
            output,
            &selenite_db::DatabaseStore::open(vault)?.query_saved(&view)?,
        )?,
        Command::InitGtd(vault) => json_output(
            output,
            &selenite_db::DatabaseStore::open(vault)?.install_gtd_templates()?,
        )?,
        Command::AiIndex(root) => {
            let vault = selenite_core::vault::Vault::open(root)?;
            let config = selenite_ai::AiConfig::load(&vault)?;
            let provider = selenite_ai::HttpProvider::new(config.clone())?;
            json_output(
                output,
                &selenite_ai::rebuild_embeddings(&vault, &config, &provider)?,
            )?;
        }
        Command::Ask {
            vault: root,
            question,
        } => {
            let vault = selenite_core::vault::Vault::open(root)?;
            let config = selenite_ai::AiConfig::load(&vault)?;
            let provider = selenite_ai::HttpProvider::new(config.clone())?;
            json_output(
                output,
                &selenite_ai::ask(&vault, &config, &provider, &question)?,
            )?;
        }
        Command::Rebuild(vault) => {
            let (_, stats) = Index::rebuild_from_disk(&vault).context("rebuild vault")?;
            writeln!(
                output,
                "rebuilt\tpages={}\tlinks={}\tproperties={}\telapsed_ms={:.3}",
                stats.pages,
                stats.links,
                stats.properties,
                stats.elapsed.as_secs_f64() * 1000.0
            )?;
        }
        Command::Search { vault, query } => {
            let index = refresh_index(&vault)?;
            for hit in index.search(&query, usize::MAX)? {
                writeln!(
                    output,
                    "{:.6}\t{}\t{}",
                    hit.score,
                    tsv(&path_key(&hit.path)),
                    tsv(&hit.title)
                )?;
            }
        }
        Command::Backlinks { vault, page } => {
            let index = refresh_index(&vault)?;
            let paths: Vec<_> = index.pages()?.into_iter().map(|page| page.path).collect();
            let target = resolve_page_argument(&paths, &page)?;
            for link in index.backlinks(&target)? {
                let source = tsv(&path_key(&link.source));
                match link.heading {
                    Some(fragment) => writeln!(output, "{source}\t#{}", tsv(&fragment))?,
                    None => writeln!(output, "{source}")?,
                }
            }
        }
    }
    Ok(())
}

fn refresh_index(vault: &Path) -> Result<Index> {
    let mut index = match Index::open(vault) {
        Ok(index) => index,
        Err(_) => {
            Index::rebuild_from_disk(vault)
                .context("recover vault index")?
                .0
        }
    };
    index.refresh().context("refresh index from Markdown")?;
    Ok(index)
}

fn resolve_page_argument(paths: &[PathBuf], target: &str) -> Result<PathBuf> {
    ensure!(!target.trim().is_empty(), "page target must not be empty");
    let normalized = target.replace('\\', "/");
    let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
    let explicit = normalized.contains('/') || normalized.to_ascii_lowercase().ends_with(".md");
    let resolver = LinkResolver::new(paths)?;
    let source = paths.first().context("vault contains no Markdown pages")?;
    if explicit {
        // CLI page arguments are vault-relative, so source-relative link syntax
        // is normalized here before invoking the shared link resolver.
        let candidate = if normalized.to_ascii_lowercase().ends_with(".md") {
            normalized.to_owned()
        } else {
            format!("{normalized}.md")
        };
        validate_page_path(Path::new(&candidate))?;
        resolver
            .resolve(source, &format!("/{normalized}"))
            .map_err(Into::into)
    } else {
        // The source is a valid known page but is irrelevant for a bare target.
        resolver.resolve(source, normalized).map_err(Into::into)
    }
}

fn tsv(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn json_output(output: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer_pretty(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}
