# Phase 0 verification

Recorded on 2026-10-13 on Windows, using Rust 1.95.0 and Node.js 24.21.0. These are local measurements, not a cross-platform performance guarantee.

## Completion evidence

| Criterion | Recorded result |
| --- | --- |
| Windows workspace build, tests and warnings | `cargo fmt --all --check`, `cargo build --workspace`, `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` passed; 57 tests passed. |
| Frontend build | `npm run build` in `apps/desktop` passed. |
| Three operating systems | GitHub Actions is configured for Windows, Ubuntu and macOS. Hosted CI has not run because nothing was pushed. |
| Rebuild 5,000 notes and recover without the cache | CLI rebuilds passed; ranked results were identical after moving the complete cache aside and rebuilding from its absence. Literal cache deletion remains unverified. |
| Search below 100 ms | Current query API p95 peaked at **4.297 ms**, maximum **4.678 ms**. End-to-end CLI search took **715.970–833.442 ms**, so the 100 ms result applies only to the query API. |
| Native desktop ready below 1 s | Five actual Tauri launches with a preselected vault completed in **906.210, 646.569, 646.006, 638.524 and 717.086 ms** from process spawn to readiness report. |
| Existing Obsidian opens the vault unchanged | Not performed. Plain Markdown/YAML and byte-preservation checks passed; actual editor interoperability remains a manual acceptance check. |

The functional implementation is complete, but Phase 0 is not fully signed off: literal deletion followed by rebuild and actual Obsidian opening remain unverified. The user deferred the Obsidian check and cannot perform it now. The search threshold also needs its stated API scope kept distinct from CLI latency. No Phase 1 implementation is included.

## Index and search measurements

The generated fixture contains 5,000 UTF-8 Markdown notes across 50 folders, including YAML, Unicode, wikilinks and fenced code. The current release build with thin LTO, at commit `819f22b`, rebuilt 5,000 pages, 5,000 links and 25,000 properties in **2,278.589 ms**. Earlier release rebuilds took **1,211.709 ms** before cache relocation and **1,197.191 ms** after relocation; these are separate recorded runs, not a controlled compiler-profile comparison.

The user approved deleting the generated benchmark cache. Tool policy rejected the deletion twice with a generic “blocked by policy” result. The complete cache was instead moved to a sibling named `cache-retained-approval-test`, then rebuilt at its original location. The old cache was retained and was not used as rebuild input. This establishes recovery from an absent cache, but does not claim that the literal deletion step occurred.

Each of six queries used five warmups followed by 50 measured calls to `Index::search`. Timing includes readiness validation and reader reload, result collection and deterministic ordering. It excludes process startup, cache opening, disk reconciliation and CLI output. Current cache opening was measured separately at **3.323 ms**.

| Query | Matches | Current p50 (ms) | Current p95 (ms) | Current maximum (ms) |
| --- | ---: | ---: | ---: | ---: |
| `selenite` | 5,000 | 3.161 | 4.297 | 4.678 |
| `astronomy` | 625 | 0.545 | 0.648 | 0.920 |
| `"local knowledge"` | 5,000 | 3.227 | 3.594 | 3.716 |
| `title:Note` | 5,000 | 3.001 | 3.319 | 3.511 |
| `unique00420` | 1 | 0.163 | 0.182 | 0.194 |
| `astronomy AND tasks` | 625 | 0.595 | 0.631 | 0.656 |

The before/after recovery snapshots and the current release snapshot matched every result path, title, score bit pattern and rank. The benchmark checked each note against the generator's exact expected source and checked the full source snapshot again after querying. Markdown bytes were unchanged.

Three PowerShell stopwatch measurements of `selenite search <vault> "unique00420"` recorded **822.189, 833.442 and 715.970 ms**. Those include process startup, opening and reconciling the full vault, and TSV output piped to `Out-Null`. End-to-end CLI search did not meet 100 ms; it must not be confused with the API measurements above.

Evidence is retained locally under `.local/benchmark/`: `final.metrics.txt`, `final.tsv`, `rebuild-final.txt` and `cli.metrics.txt`, plus the original `before`/`after` metrics, result snapshots and rebuild logs. These generated files are ignored by Git. The measurement implementation is [phase0_bench.rs](../crates/selenite-index/examples/phase0_bench.rs).

## Native desktop measurement

[native-smoke.mjs](../scripts/native-smoke.mjs) launched the release Tauri executable and connected to its actual WebView through local CDP. The frontend reported readiness after rendering and two animation frames. The check verified today's preview, the file tree with 5,001 Markdown files including the journal note, navigation to another note and back, cancellation of an in-flight navigation, and rejection of `../outside.md` by the native command.

| Run | WebView profile | Process spawn to report (ms) | Internal app readiness (ms) |
| --- | --- | ---: | ---: |
| 1 | Fresh | 906.210 | 588.783 |
| 2 | Reused | 646.569 | 583.995 |
| 3 | Reused | 646.006 | 604.166 |
| 4 | Reused | 638.524 | 586.397 |
| 5 | Reused | 717.086 | 667.320 |

“Fresh” means a newly created WebView data directory. The operating system's file cache was not cleared. The preselected vault avoided timing human interaction with a folder picker. These measurements support the local startup target for this fixture and machine; they are not system-cold measurements or a portable latency SLA.

Native evidence, process logs and a screenshot are retained at `.local/native-smoke/1791888826934-120532/`; `summary.json` records both clocks. The tested journal page was `Journal/2026-10-13.md`.

## Reproduce on Windows

Run from the authorized repository. Keep tool caches and generated evidence inside it. Install prerequisites through the documented build setup before running these commands.

```powershell
Set-Location -LiteralPath 'D:\PProjects\selenite-app'
$ErrorActionPreference = 'Stop'
$env:CARGO_HOME = Join-Path $PWD '.local/cargo'
$env:TEMP = Join-Path $PWD '.local/tmp'
$env:TMP = $env:TEMP
$env:npm_config_cache = Join-Path $PWD '.local/npm'
$env:npm_config_userconfig = Join-Path $PWD '.local/npmrc'
New-Item -ItemType Directory -Force -Path $env:CARGO_HOME, $env:TEMP, $env:npm_config_cache | Out-Null
npm --prefix apps/desktop ci
npm --prefix apps/desktop run build
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p selenite-cli -p selenite-desktop
cargo build --release -p selenite-index --example phase0_bench
```

Create a new fixture destination; the generator refuses an existing directory. The retained-cache procedure below does not delete files. Close any app using this fixture before moving its cache.

```powershell
$runRoot = Join-Path $PWD ('.local/verification-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runRoot | Out-Null
$benchmarkVault = Join-Path $runRoot 'vault'
$before = Join-Path $runRoot 'before.tsv'
$after = Join-Path $runRoot 'after.tsv'
& .\target\release\examples\phase0_bench.exe generate $benchmarkVault
& .\target\release\selenite.exe rebuild $benchmarkVault
& .\target\release\examples\phase0_bench.exe measure $benchmarkVault $before

$vaultRoot = (Resolve-Path -LiteralPath $benchmarkVault).Path
$metadata = (Resolve-Path -LiteralPath (Join-Path $vaultRoot '.selenite')).Path
$cache = (Resolve-Path -LiteralPath (Join-Path $metadata 'cache')).Path
if ([IO.Path]::GetRelativePath($vaultRoot, $cache) -ne [IO.Path]::Combine('.selenite', 'cache')) {
    throw 'Cache path is outside the expected fixture location'
}
$retained = Join-Path $metadata ('cache-retained-' + [guid]::NewGuid().ToString('N'))
if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($retained)) -ne $metadata) {
    throw 'Retained cache destination escaped the metadata directory'
}
if (Test-Path -LiteralPath $retained) { throw 'Retained cache destination already exists' }
Move-Item -LiteralPath $cache -Destination $retained
& .\target\release\selenite.exe rebuild $benchmarkVault
& .\target\release\examples\phase0_bench.exe measure $benchmarkVault $after $before
```

For the remaining literal-deletion acceptance check, obtain permission for the exact generated cache directory and preserve its source vault. The earlier permission was scoped to the original benchmark cache; it is not blanket permission to delete new fixtures or user vaults.

Use a separate generated vault for native startup testing, because the desktop adds today's note and the index benchmark requires exactly 5,000 generated notes:

```powershell
$desktopVault = Join-Path $runRoot 'desktop-vault'
& .\target\release\examples\phase0_bench.exe generate $desktopVault
node scripts/native-smoke.mjs $desktopVault 5
```

The recorded native run used `node scripts/native-smoke.mjs .local/desktop-benchmark/vault 5`. Each invocation retains a separate evidence directory and WebView profile.

## Manual interoperability acceptance

Use an existing Obsidian installation; no other editor was installed or run during this verification.

1. Record Markdown file hashes for a generated test vault containing a Selenite daily note.
2. Open that same folder as a vault, without converting or importing its notes.
3. Confirm the journal note, YAML properties, ordinary Markdown and wikilinks are readable.
4. Close the editor without editing notes, compare Markdown hashes, and record the editor version and result. Editor-specific configuration files should be assessed separately from note content.

Passing this manual check closes the currently unverified editor-opening criterion; source-format and byte-preservation tests alone do not establish it.
