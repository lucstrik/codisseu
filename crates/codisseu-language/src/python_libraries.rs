//! Read-only Python sources, without running the interpreter or importing code.
use std::path::{Path, PathBuf};
fn append(roots: &mut Vec<PathBuf>, path: PathBuf, base: &Path) {
    if path.is_dir()
        && let Ok(canonical) = path.canonicalize()
        && canonical.starts_with(base)
        && !roots.contains(&canonical)
    {
        roots.push(canonical);
    }
}
pub(crate) fn roots(interpreter: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = codisseu_runtime::python_library_roots().unwrap_or_default();
    if let Some(python) = interpreter {
        // Preserve the declared venv executable before resolving its usual symlink
        // to the base interpreter. Both may contribute explicitly scoped libraries.
        for executable in [Some(python.to_path_buf()), python.canonicalize().ok()]
            .into_iter()
            .flatten()
        {
            let Some(bin) = executable.parent() else {
                continue;
            };
            let Some(base) = bin.parent().and_then(|p| p.canonicalize().ok()) else {
                continue;
            };
            if !matches!(
                bin.file_name().and_then(|n| n.to_str()),
                Some("bin" | "Scripts")
            ) {
                continue;
            }
            for lib in ["lib", "lib64", "Lib"] {
                let folder = base.join(lib);
                if lib == "Lib" {
                    append(&mut roots, folder, &base);
                    continue;
                }
                if let Ok(entries) = std::fs::read_dir(folder) {
                    for entry in entries.take(32).flatten() {
                        if entry.file_name().to_str().is_some_and(|n| {
                            n.starts_with("python3.") && n[8..].bytes().all(|b| b.is_ascii_digit())
                        }) {
                            // The stdlib root includes its site-packages. External .pth
                            // entries and arbitrary interpreter sys.path are not trusted.
                            append(&mut roots, entry.path(), &base);
                        }
                    }
                }
            }
        }
    }
    roots.truncate(64);
    roots
}
pub fn python_library_root(path: &Path, interpreter: Option<&Path>) -> Option<PathBuf> {
    if !matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi" | "pyw")
    ) {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if canonical != path || !canonical.is_file() {
        return None;
    }
    roots(interpreter)
        .into_iter()
        .find(|root| canonical.starts_with(root))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn venv_roots_are_confined_and_source_only() {
        let dir =
            std::env::temp_dir().join(format!("codisseu-python-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("lib/python3.13/site-packages")).unwrap();
        std::fs::write(dir.join("bin/python"), "").unwrap();
        let path = dir.join("lib/python3.13/site-packages/dep.py");
        std::fs::write(&path, "value=42\n").unwrap();
        assert!(python_library_root(&path, Some(&dir.join("bin/python"))).is_some());
        let other = dir.join("outside.py");
        std::fs::write(&other, "value=42\n").unwrap();
        assert!(python_library_root(&other, Some(&dir.join("bin/python"))).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
