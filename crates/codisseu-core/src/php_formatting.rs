//! Single-snapshot PHP formatting. Only installation-owned helper/tool are executed.
use std::{
    fs,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub(super) fn format(source: &str) -> Result<String, String> {
    if source.len() > 256 * 1024 {
        return Err("Formatação PHP limitada a256KiB".into());
    }
    let (php, fixer, helper) = codisseu_runtime::php_formatter()?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let input = temp.path().join("input.php");
    let output = temp.path().join("output.php");
    let errors = temp.path().join("errors.log");
    fs::write(&input, source).map_err(|e| e.to_string())?;
    let stderr = fs::File::create(&errors).map_err(|e| e.to_string())?;
    let child = Command::new(php)
        .args([
            "-d",
            "auto_prepend_file=",
            "-d",
            "auto_append_file=",
            "-d",
            "xdebug.mode=off",
            "-d",
            "memory_limit=128M",
            "-d",
            "max_execution_time=5",
        ])
        .arg(helper)
        .arg(fixer)
        .arg(&input)
        .arg(&output)
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|_| "Não foi possível iniciar o motor PHP de formatação")?;
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut process = Process(child);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = process.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(
                    "Formatação PHP recusada: sintaxe inválida ou versão não suportada".into(),
                );
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err("Formatação PHP excedeu8segundos".into());
        }
        if fs::metadata(&errors).is_ok_and(|m| m.len() > 65536) {
            return Err("Motor PHP excedeu limite de diagnóstico".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut bytes = Vec::new();
    fs::File::open(&output)
        .map_err(|_| "Motor PHP não produziu resultado")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Formatação PHP excederia1MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Resultado PHP não UTF-8".into())
}
