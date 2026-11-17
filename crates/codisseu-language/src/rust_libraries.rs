//! Read-only source roots for Rust navigation, independently revalidated by the
//! language and document workers. No Cargo/rustc execution or arbitrary home scan.
use std::path::{Path, PathBuf};

fn declared_home(variable: &str, fallback: Option<PathBuf>) -> Option<PathBuf> {
    let path = std::env::var_os(variable).map(PathBuf::from).or(fallback)?;
    path.is_absolute().then_some(())?;
    path.canonicalize().ok()
}

fn append_root(roots: &mut Vec<PathBuf>, path: PathBuf) {
    // A source root or one of its ancestors must not redirect the source scope
    // to another tree. Declared CARGO_HOME/RUSTUP_HOME are canonicalized first.
    if path.is_dir() && path.canonicalize().ok().as_ref() == Some(&path) {
        roots.push(path);
    }
}

fn roots_from_homes(cargo: Option<&Path>, rustup: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(cargo) = cargo {
        append_root(&mut roots, cargo.join("registry/src"));
    }
    if let Some(rustup) = rustup {
        let toolchains = rustup.join("toolchains");
        if toolchains.canonicalize().ok().as_ref() == Some(&toolchains)
            && let Ok(entries) = std::fs::read_dir(toolchains)
        {
            for entry in entries.take(64).flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    append_root(
                        &mut roots,
                        entry.path().join("lib/rustlib/src/rust/library"),
                    );
                }
            }
        }
    }
    roots
}

pub(crate) fn roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .and_then(|p| p.canonicalize().ok());
    let cargo = declared_home("CARGO_HOME", home.as_ref().map(|p| p.join(".cargo")));
    let rustup = declared_home("RUSTUP_HOME", home.as_ref().map(|p| p.join(".rustup")));
    let mut roots = roots_from_homes(cargo.as_deref(), rustup.as_deref());
    if let Ok(Some(resources)) = codisseu_runtime::discover()
        && let Ok(Some(sysroot)) = resources.rust_sysroot()
    {
        append_root(&mut roots, sysroot.join("lib/rustlib/src/rust/library"));
    }
    roots
}

pub(crate) fn root_in(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    if path.extension().is_none_or(|ext| ext != "rs") || !path.is_file() {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if canonical.extension().is_none_or(|ext| ext != "rs") {
        return None;
    }
    roots
        .iter()
        .find(|root| canonical.starts_with(root))
        .cloned()
}

/// Return the permitted source root for an existing `.rs` library file.
/// This performs filesystem reads and must only run on a worker, never per frame.
/// Callers must additionally prove a Rust navigation origin and open read-only.
pub fn rust_library_root(path: &Path) -> Option<PathBuf> {
    root_in(path, &roots())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_roots_are_bounded_and_do_not_include_cargo_credentials_or_arbitrary_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let cargo = home.join("cargo");
        let rustup = home.join("rustup");
        let registry = cargo.join("registry/src/index/package-1/src");
        let library = rustup.join("toolchains/stable/lib/rustlib/src/rust/library/core/src");
        for dir in [&registry, &library] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let source = registry.join("lib.rs");
        let core = library.join("lib.rs");
        let private = home.join("private.rs");
        let credentials = cargo.join("credentials.toml");
        for file in [&source, &core, &private, &credentials] {
            std::fs::write(file, "test").unwrap();
        }
        let roots = roots_from_homes(Some(&cargo), Some(&rustup));
        assert_eq!(roots.len(), 2);
        assert_eq!(root_in(&source, &roots), Some(cargo.join("registry/src")));
        assert!(root_in(&core, &roots).is_some());
        assert!(root_in(&private, &roots).is_none());
        assert!(root_in(&credentials, &roots).is_none());
        assert!(root_in(&registry, &roots).is_none());
        assert!(root_in(&registry.join("missing.rs"), &roots).is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&private, registry.join("escape.rs")).unwrap();
            assert!(root_in(&registry.join("escape.rs"), &roots).is_none());
            let alias = home.join("alias");
            std::fs::create_dir_all(&alias).unwrap();
            std::os::unix::fs::symlink(&home, alias.join("registry")).unwrap();
            assert!(roots_from_homes(Some(&alias), None).is_empty());
            std::os::unix::fs::symlink(home.join("private"), rustup.join("toolchains/linked"))
                .unwrap();
            assert_eq!(roots_from_homes(None, Some(&rustup)).len(), 1);
        }
    }
}
