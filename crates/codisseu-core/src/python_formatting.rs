//! Ruff operates only on a bounded in-memory snapshot. It never writes project sources.
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
static RUFF: Mutex<()> = Mutex::new(());
#[derive(Clone, Copy)]
pub(super) enum Operation {
    Format,
    SafeFixes,
    Imports,
}
pub(super) fn transform(
    source: &str,
    path: &Path,
    root: &Path,
    operation: Operation,
) -> Result<String, String> {
    if source.len() > 256 * 1024 {
        return Err("Operação Ruff limitada a256KiB por arquivo".into());
    }
    if !path.starts_with(root)
        || !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("py" | "pyi" | "pyw")
        )
    {
        return Err("Fonte Python fora do projeto ou extensão inválida".into());
    }
    let _guard = RUFF
        .try_lock()
        .map_err(|_| "Aguarde a operação Ruff anterior")?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let input = temp.path().join("source.py");
    let output = temp.path().join("output");
    let error = temp.path().join("errors");
    fs::write(&input, source).map_err(|e| e.to_string())?;
    let mut command = Command::new(codisseu_runtime::ruff()?);
    match operation {
        Operation::Format => {
            command.arg("format");
        }
        Operation::SafeFixes => {
            command.args(["check", "--fix-only", "--no-unsafe-fixes", "--exit-zero"]);
        }
        Operation::Imports => {
            command.args([
                "check",
                "--select",
                "I",
                "--fix-only",
                "--no-unsafe-fixes",
                "--exit-zero",
            ]);
        }
    }
    command
        .arg("--stdin-filename")
        .arg(path)
        .arg("-")
        .current_dir(root)
        .stdin(fs::File::open(input).map_err(|e| e.to_string())?)
        .stdout(Stdio::from(
            fs::File::create(&output).map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            fs::File::create(&error).map_err(|e| e.to_string())?,
        ))
        .env("NO_COLOR", "1");
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut process = Process(
        command
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar Ruff: {e}"))?,
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = process.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                let error = fs::read(&error).unwrap_or_default();
                return Err(format!(
                    "Ruff recusou a operação: {}",
                    crate::php_tests::process::inert(&error[..error.len().min(8192)])
                ));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err("Ruff excedeu8segundos; buffer preservado".into());
        }
        if fs::metadata(&output).is_ok_and(|m| m.len() > 1024 * 1024)
            || fs::metadata(&error).is_ok_and(|m| m.len() > 65536)
        {
            return Err("Ruff excedeu orçamento de saída".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut bytes = Vec::new();
    fs::File::open(output)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Resultado Ruff excede1MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Resultado Ruff não UTF-8".into())
}
