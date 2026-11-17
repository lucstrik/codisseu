//! Bounded Cargo discovery and explicit shell-free task plans. Discovery never runs Cargo.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CargoAction {
    Check,
    #[default]
    Build,
    Run,
    Test,
    Clippy,
    Doc,
}
impl CargoAction {
    pub const ALL: [Self; 6] = [
        Self::Check,
        Self::Build,
        Self::Run,
        Self::Test,
        Self::Clippy,
        Self::Doc,
    ];
    pub fn command(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Run => "run",
            Self::Test => "test",
            Self::Clippy => "clippy",
            Self::Doc => "doc",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Check => "Check",
            Self::Build => "Build",
            Self::Run => "Run",
            Self::Test => "Test",
            Self::Clippy => "Clippy",
            Self::Doc => "Documentation",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct CargoOptions {
    pub action: CargoAction,
    pub package: String,
    pub target: String,
    pub release: bool,
    pub offline: bool,
    pub locked: bool,
    pub all_features: bool,
    /// Arguments for the program/test harness, never reparsed by a shell.
    pub arguments: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct CargoPlan {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub args: Vec<String>,
}

/// The nearest manifest is a package; Cargo resolves its owning workspace itself.
/// Canonical paths and a strict ancestor bound prevent escaping the open project.
pub fn discover_manifest(root: &Path, active: Option<&Path>) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Projeto Rust: {e}"))?;
    let start = active
        .unwrap_or(&root)
        .canonicalize()
        .map_err(|e| format!("Arquivo Rust: {e}"))?;
    if !start.starts_with(&root) {
        return Err("Arquivo fora do projeto Rust".into());
    }
    let mut dir = if start.is_dir() {
        start
    } else {
        start.parent().ok_or("Arquivo sem pasta")?.to_path_buf()
    };
    for _ in 0..128 {
        let candidate = dir.join("Cargo.toml");
        if let Ok(meta) = std::fs::symlink_metadata(&candidate) {
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
                return Err(
                    "Cargo.toml deve ser arquivo regular de até1MiB, sem link simbólico".into(),
                );
            }
            return Ok(candidate);
        }
        if dir == root || !dir.pop() || !dir.starts_with(&root) {
            break;
        }
    }
    Err("Cargo.toml não encontrado entre o arquivo ativo e a raiz do projeto".into())
}

pub fn cargo_program() -> PathBuf {
    // Existing API remains infallible for explicit task planning; a missing
    // bundled engine is kept as a non-executable error path rather than falling
    // through to a project-owned tool.
    codisseu_runtime::cargo().unwrap_or_else(|_| PathBuf::from("/codisseu-missing-bundled-cargo"))
}

pub fn plan(
    root: &Path,
    manifest: &Path,
    program: PathBuf,
    options: &CargoOptions,
) -> Result<CargoPlan, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let resolved = manifest
        .canonicalize()
        .map_err(|e| format!("Cargo.toml: {e}"))?;
    let meta = std::fs::symlink_metadata(manifest).map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root)
        || resolved.file_name().is_none_or(|n| n != "Cargo.toml")
        || !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > 1024 * 1024
    {
        return Err("Manifesto Cargo fora do projeto ou inválido".into());
    }
    if program.as_os_str().is_empty() || program.as_os_str().len() > 4096 {
        return Err("Executável Cargo inválido".into());
    }
    for value in [&options.package, &options.target] {
        if value.len() > 256 || value.contains(['\0', '\n', '\r']) || value.starts_with('-') {
            return Err("Pacote ou alvo Cargo inválido".into());
        }
    }
    if options.arguments.len() > 64
        || options.arguments.iter().map(String::len).sum::<usize>() > 32768
        || options.arguments.iter().any(|a| a.contains('\0'))
    {
        return Err("Argumentos Cargo excedem os limites".into());
    }
    if !options.arguments.is_empty()
        && !matches!(options.action, CargoAction::Run | CargoAction::Test)
    {
        return Err("Argumentos de programa são permitidos apenas em Run e Test".into());
    }
    let mut args = vec![
        options.action.command().into(),
        "--manifest-path".into(),
        resolved.display().to_string(),
    ];
    if !options.package.is_empty() {
        args.extend(["--package".into(), options.package.clone()]);
    }
    if !options.target.is_empty() {
        args.extend(["--bin".into(), options.target.clone()]);
    }
    if options.release {
        args.push("--release".into());
    }
    if options.offline {
        args.push("--offline".into());
    }
    if options.locked {
        args.push("--locked".into());
    }
    if options.all_features {
        args.push("--all-features".into());
    }
    if !options.arguments.is_empty() {
        args.push("--".into());
        args.extend(options.arguments.clone());
    }
    Ok(CargoPlan {
        program,
        cwd: resolved.parent().unwrap().to_path_buf(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_manifest_and_literal_arguments_stay_in_project() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("space ; project");
        std::fs::create_dir_all(root.join("member/src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=['member']\n").unwrap();
        std::fs::write(
            root.join("member/Cargo.toml"),
            "[package]\nname='example'\n",
        )
        .unwrap();
        let file = root.join("member/src/main.rs");
        std::fs::write(&file, "fn main(){}\n").unwrap();
        let manifest = discover_manifest(&root, Some(&file)).unwrap();
        assert_eq!(manifest, root.join("member/Cargo.toml"));
        let options = CargoOptions {
            action: CargoAction::Run,
            package: "example".into(),
            target: "example".into(),
            offline: true,
            arguments: vec!["literal ; $(nothing) ' spaces".into()],
            ..Default::default()
        };
        let result = plan(&root, &manifest, "cargo".into(), &options).unwrap();
        assert_eq!(result.args.last().unwrap(), "literal ; $(nothing) ' spaces");
        assert!(result.args.contains(&"--offline".into()));
        assert_eq!(result.cwd, root.join("member"));
        assert!(discover_manifest(&root, Some(temp.path())).is_err());
        assert!(
            plan(
                &root.join("member"),
                &root.join("Cargo.toml"),
                "cargo".into(),
                &options
            )
            .is_err()
        );
        let mut invalid = options.clone();
        invalid.package = "--config=bad".into();
        assert!(plan(&root, &manifest, "cargo".into(), &invalid).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_manifest_is_not_an_execution_authority() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other"), "[package]").unwrap();
        std::os::unix::fs::symlink("other", dir.path().join("Cargo.toml")).unwrap();
        assert!(discover_manifest(dir.path(), None).is_err());
        assert!(
            plan(
                dir.path(),
                &dir.path().join("Cargo.toml"),
                "cargo".into(),
                &CargoOptions::default()
            )
            .is_err()
        );
    }
}
