use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use selenite_core::{
    Page, validate_page_path,
    vault::{Vault, is_markdown},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasmtime::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
};

use crate::{Capability, PluginConfig, PluginManifest, SandboxLimits};

pub struct PluginHost {
    engine: Engine,
    limits: SandboxLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRun {
    pub output: Value,
    pub fuel_consumed: u64,
    pub host_calls: u32,
}

struct HostState {
    vault: Vault,
    grants: BTreeSet<Capability>,
    limits: StoreLimits,
    policy: SandboxLimits,
    calls: u32,
}

impl PluginHost {
    pub fn new(config: &PluginConfig) -> Result<Self> {
        config.validate()?;
        ensure!(config.enabled, "plugins are disabled");
        let mut engine = Config::new();
        engine
            .consume_fuel(true)
            .max_wasm_stack(512 * 1024)
            .wasm_memory64(false)
            .wasm_multi_memory(false);
        Ok(Self {
            engine: Engine::new(&engine)?,
            limits: config.limits.clone(),
        })
    }

    pub fn run(
        &self,
        vault: &Vault,
        manifest: &PluginManifest,
        wasm: &[u8],
        grants: &BTreeSet<Capability>,
        input: Value,
    ) -> Result<PluginRun> {
        manifest.validate()?;
        ensure!(
            wasm.len() <= self.limits.max_module_bytes && wasm.starts_with(b"\0asm"),
            "plugin must be a bounded binary WebAssembly module"
        );
        let input = serde_json::to_vec(&input)?;
        ensure!(
            input.len() <= self.limits.max_message_bytes,
            "plugin input exceeds its size limit"
        );
        let module = Module::new(&self.engine, wasm).context("compile plugin WebAssembly")?;
        for import in module.imports() {
            ensure!(
                import.module() == "selenite" && import.name() == "call",
                "unsupported import {}.{}; no WASI or ambient imports are available",
                import.module(),
                import.name()
            );
        }
        let state = HostState {
            vault: vault.clone(),
            grants: grants
                .intersection(&manifest.capabilities)
                .copied()
                .collect(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.limits.memory_bytes)
                .instances(1)
                .memories(1)
                .tables(1)
                .table_elements(1024)
                .trap_on_grow_failure(true)
                .build(),
            policy: self.limits.clone(),
            calls: 0,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.limits.fuel)?;
        let mut linker = Linker::new(&self.engine);
        linker.func_wrap("selenite", "call", host_call)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .context("instantiate plugin")?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("plugin must export memory")?;
        let allocate = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .context("plugin must export alloc(i32)->i32")?;
        let run = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "run")
            .context("plugin must export run(i32,i32)->i64")?;
        let input_pointer = allocate
            .call(&mut store, input.len() as i32)
            .context("plugin allocation failed")?;
        ensure!(
            input_pointer >= 0,
            "plugin returned an invalid allocation pointer"
        );
        memory
            .write(&mut store, input_pointer as usize, &input)
            .context("plugin allocation is outside memory")?;
        let packed = run
            .call(&mut store, (input_pointer, input.len() as i32))
            .context("plugin execution failed")? as u64;
        let pointer = (packed >> 32) as usize;
        let length = (packed & 0xffff_ffff) as usize;
        ensure!(
            length <= self.limits.max_message_bytes,
            "plugin output exceeds its size limit"
        );
        let mut output = vec![0; length];
        memory
            .read(&store, pointer, &mut output)
            .context("plugin output is outside memory")?;
        let output = serde_json::from_slice(&output).context("plugin output is not valid JSON")?;
        Ok(PluginRun {
            output,
            fuel_consumed: self.limits.fuel - store.get_fuel()?,
            host_calls: store.data().calls,
        })
    }
}

fn host_call(
    mut caller: Caller<'_, HostState>,
    request_pointer: i32,
    request_length: i32,
    output_pointer: i32,
    output_capacity: i32,
) -> Result<i32> {
    let policy = caller.data().policy.clone();
    ensure!(
        caller.data().calls < policy.max_host_calls,
        "plugin exceeded its host-call limit"
    );
    caller.data_mut().calls += 1;
    ensure!(
        request_pointer >= 0 && request_length >= 0 && output_pointer >= 0 && output_capacity >= 0,
        "negative ABI pointer or size"
    );
    ensure!(
        request_length as usize <= policy.max_message_bytes,
        "host request exceeds its size limit"
    );
    // Reserve the full response budget before dispatch. A write must not commit
    // and then fail merely because the guest supplied a tiny output buffer.
    ensure!(
        output_capacity as usize == policy.max_message_bytes,
        "host output capacity must equal max_message_bytes"
    );
    let memory = caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .context("plugin must export memory")?;
    check_range(
        &memory,
        &caller,
        output_pointer as usize,
        output_capacity as usize,
    )?;
    let mut request = vec![0; request_length as usize];
    memory
        .read(&caller, request_pointer as usize, &mut request)
        .context("host request is outside guest memory")?;
    let fuel = caller.get_fuel()?;
    ensure!(fuel >= 1000, "insufficient fuel for a host call");
    caller.set_fuel(fuel - 1000)?;
    let result = serde_json::from_slice::<Request>(&request)
        .context("invalid host request JSON")
        .and_then(|request| dispatch(caller.data(), request));
    let response = match result {
        Ok(value) => json!({"ok":true,"data":value}),
        Err(error) => {
            json!({"ok":false,"error":format!("{error:#}").chars().take(1024).collect::<String>()})
        }
    };
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() > policy.max_message_bytes {
        bytes = br#"{"ok":false,"error":"host response exceeds its size limit"}"#.to_vec();
    }
    memory.write(&mut caller, output_pointer as usize, &bytes)?;
    Ok(bytes.len() as i32)
}

fn check_range(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    pointer: usize,
    length: usize,
) -> Result<()> {
    let end = pointer.checked_add(length).context("ABI range overflow")?;
    ensure!(
        end <= memory.data_size(caller),
        "host output range is outside guest memory"
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    List {
        #[serde(default)]
        offset: usize,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
        expected_source: Option<String>,
        source: String,
    },
}

fn default_limit() -> usize {
    100
}

fn dispatch(state: &HostState, request: Request) -> Result<Value> {
    match request {
        Request::List { offset, limit } => {
            ensure!(
                state.grants.contains(&Capability::VaultList),
                "vault_list capability was not granted"
            );
            ensure!(
                (1..=1000).contains(&limit) && offset <= 20_000,
                "list limit must be 1–1000 and offset at most 20000"
            );
            let _guard = state.vault.lock_shared()?;
            let paths = bounded_pages(&state.vault, state.policy.max_message_bytes * 4)?;
            let end = offset.saturating_add(limit).min(paths.len());
            let selected: Vec<_> = paths
                .iter()
                .skip(offset)
                .take(limit)
                .map(|path| path_key(path))
                .collect();
            Ok(
                json!({"paths":selected,"next_offset":if end < paths.len() { Some(end) } else { None }}),
            )
        }
        Request::Read { path } => {
            ensure!(
                state.grants.contains(&Capability::VaultRead),
                "vault_read capability was not granted"
            );
            let path = page_path(&path)?;
            let _guard = state.vault.lock_shared()?;
            // JSON can expand a source byte into a six-byte escape sequence.
            let source = state
                .vault
                .read_source_bounded(&path, (state.policy.max_message_bytes - 512) / 6)?;
            Ok(json!({"path":path_key(&path),"source":source}))
        }
        Request::Write {
            path,
            expected_source,
            source,
        } => {
            ensure!(
                state.grants.contains(&Capability::VaultWrite),
                "vault_write capability was not granted"
            );
            let path = page_path(&path)?;
            Page::parse(&path, &source).context("plugin produced invalid Markdown/frontmatter")?;
            if let Some(expected) = expected_source {
                let result = state.vault.write_page(&path, &expected, &source)?;
                Ok(json!({"path":path_key(&path),"created":false,"revision":result.revision}))
            } else {
                ensure!(
                    state.vault.create_page(&path, &source)?,
                    "page already exists; provide expected_source to update"
                );
                Ok(json!({"path":path_key(&path),"created":true}))
            }
        }
    }
}

fn page_path(path: &str) -> Result<PathBuf> {
    ensure!(
        path.len() <= 256 && !path.contains('\\') && !path.chars().any(char::is_control),
        "page path must be a portable path of at most 256 bytes"
    );
    let path: PathBuf = path.split('/').collect();
    validate_page_path(&path)?;
    Ok(path)
}

pub(crate) fn path_key(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Cap enumeration itself rather than collecting an unbounded vault and then
/// paginating. No symbolic links or metadata directories are traversed.
fn bounded_pages(vault: &Vault, byte_budget: usize) -> Result<Vec<PathBuf>> {
    let mut pending = vec![PathBuf::new()];
    let mut pages = Vec::new();
    let mut bytes = 0;
    let mut entries = 0;
    while let Some(relative) = pending.pop() {
        let directory = if relative.as_os_str().is_empty() {
            vault.root().to_owned()
        } else {
            vault.resolve_existing(&relative)?
        };
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            entries += 1;
            ensure!(
                entries <= 20_000,
                "plugin listing exceeds 20000 vault entries"
            );
            let kind = entry.file_type()?;
            let name = entry.file_name();
            if name == ".git" || name == ".selenite" || kind.is_symlink() {
                continue;
            }
            let path = relative.join(name);
            let text = path.to_str().context("plugin paths must be Unicode")?;
            ensure!(
                text.len() <= 4096,
                "vault path exceeds the plugin listing limit"
            );
            bytes += text.len();
            ensure!(
                bytes <= byte_budget,
                "plugin listing exceeds its host memory budget"
            );
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && is_markdown(&path) {
                pages.push(path);
            }
        }
    }
    pages.sort();
    Ok(pages)
}
