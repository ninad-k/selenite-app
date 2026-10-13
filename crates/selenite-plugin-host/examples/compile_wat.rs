use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result, ensure};

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let input = PathBuf::from(
        arguments
            .next()
            .context("usage: compile_wat <input.wat> <new-output.wasm>")?,
    );
    let output = PathBuf::from(arguments.next().context("missing output path")?);
    ensure!(arguments.next().is_none(), "unexpected extra argument");
    let wasm = wat::parse_file(input)?;
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create_new(&output).context("output must not already exist")?;
    file.write_all(&wasm)?;
    file.sync_all()?;
    println!("Wrote {} bytes to {}", wasm.len(), output.display());
    Ok(())
}
