//! Installation-owned tools, never discovered in the open project.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
const MANIFEST: &str = "runtime.json";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub language: String,
    pub program: String,
    pub args: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    tools: BTreeMap<String, String>,
    providers: Vec<Provider>,
}
#[derive(Clone, Debug)]
pub struct Resources {
    root: PathBuf,
    manifest: Manifest,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedProvider {
    pub language: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}
impl Resources {
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|_| "Recursos do Codisseu não encontrados; reinstale o pacote")?;
        let path = root.join(MANIFEST);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "Manifesto dos motores ausente; reinstale o pacote")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 * 1024 {
            return Err("Manifesto dos motores inválido ou grande demais".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|_| "Não foi possível ler os motores")?
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Não foi possível ler os motores")?;
        if bytes.len() > 128 * 1024 {
            return Err("Manifesto dos motores grande demais".into());
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| "Manifesto dos motores inválido")?;
        // The public core package deliberately ships no private interpreters.
        // A present empty manifest is authoritative, not a request to search
        // the developer checkout or guess a project-supplied replacement.
        let core_only = manifest.tools.is_empty() && manifest.providers.is_empty();
        if manifest.schema_version != 1
            || manifest.tools.len() > 32
            || (!core_only && !(2..=4).contains(&manifest.providers.len()))
        {
            return Err("Versão/limites do manifesto dos motores inválidos".into());
        }
        let result = Self { root, manifest };
        if core_only {
            return Ok(result);
        }
        for role in ["php", "node", "php-debug", "js-debug"] {
            result.tool(role)?;
        }
        let resolved = result.providers()?;
        if resolved.iter().enumerate().any(|(i, p)| {
            resolved[..i]
                .iter()
                .any(|other| other.language == p.language)
        }) || !["Php", "TypeScript"]
            .iter()
            .all(|language| resolved.iter().any(|p| &p.language == language))
            || resolved.iter().any(|p| {
                !matches!(
                    p.language.as_str(),
                    "Php" | "TypeScript" | "Rust" | "Python"
                )
            })
        {
            return Err("Motores de linguagem inválidos".into());
        }
        Ok(result)
    }
    fn path(&self, relative: &str) -> Result<PathBuf, String> {
        let path = Path::new(relative);
        if relative.is_empty()
            || relative.len() > 4096
            || relative.contains(['\0', '\n', '\r', '\\'])
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("Caminho de motor fora da instalação".into());
        }
        let target = self
            .root
            .join(path)
            .canonicalize()
            .map_err(|_| "Motor ausente; reinstale o pacote completo do Codisseu")?;
        if !target.starts_with(&self.root) || !target.is_file() {
            return Err("Motor fora da instalação ou arquivo inválido".into());
        }
        Ok(target)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn tool(&self, name: &str) -> Result<PathBuf, String> {
        self.path(
            self.manifest
                .tools
                .get(name)
                .ok_or("Motor não incluído neste pacote")?,
        )
    }
    /// Private compiler tree, resolved only on workers. The source directory
    /// is a permitted read-only library root, never a workspace write grant.
    pub fn rust_sysroot(&self) -> Result<Option<PathBuf>, String> {
        if !self.manifest.tools.contains_key("rustc") {
            return Ok(None);
        }
        self.tool("rustc")?;
        let declared = self.root.join("runtime/rust/toolchain");
        let path = declared
            .canonicalize()
            .map_err(|_| "Sysroot Rust ausente; reinstale o pacote")?;
        if path != declared || !path.starts_with(&self.root) || !path.is_dir() {
            return Err("Sysroot Rust fora da instalação".into());
        }
        Ok(Some(path))
    }
    pub fn python_library_roots(&self) -> Result<Vec<PathBuf>, String> {
        if !self.manifest.tools.contains_key("python") {
            return Ok(vec![]);
        }
        self.tool("python")?;
        let base = self.root.join("runtime/python");
        if base
            .canonicalize()
            .map_err(|_| "Bibliotecas Python privadas ausentes")?
            != base
        {
            return Err("Biblioteca Python fora da instalação".into());
        }
        checked_python_roots(&base)
    }
    pub fn providers(&self) -> Result<Vec<ResolvedProvider>, String> {
        self.manifest
            .providers
            .iter()
            .map(|command| {
                if command.args.len() > 32 {
                    return Err("Argumentos do motor excedem o limite".into());
                }
                let program = self.path(&command.program)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if fs::metadata(&program)
                        .map_err(|_| "Motor indisponível")?
                        .permissions()
                        .mode()
                        & 0o111
                        == 0
                    {
                        return Err("Motor não executável; reinstale o pacote".into());
                    }
                }
                let args = command
                    .args
                    .iter()
                    .map(|arg| {
                        if arg.len() > 16384 || arg.contains('\0') {
                            return Err("Argumento do motor inválido".into());
                        }
                        if let Some(relative) = arg.strip_prefix("@/") {
                            Ok(self.path(relative)?.to_string_lossy().into_owned())
                        } else {
                            Ok(arg.clone())
                        }
                    })
                    .collect::<Result<_, String>>()?;
                Ok(ResolvedProvider {
                    language: command.language.clone(),
                    program,
                    args,
                })
            })
            .collect()
    }
    pub fn report_json(&self) -> Result<String, String> {
        let tools = self
            .manifest
            .tools
            .keys()
            .map(|key| Ok((key.clone(), self.tool(key)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        serde_json::to_string_pretty(
            &serde_json::json!({"resources":self.root,"tools":tools,"providers":self.providers()?}),
        )
        .map_err(|e| e.to_string())
    }
}
pub fn discover() -> Result<Option<Resources>, String> {
    if let Some(root) = std::env::var_os("CODISSEU_RESOURCE_DIR") {
        return Resources::open(Path::new(&root)).map(Some);
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = executable
        .parent()
        .ok_or("Diretório da instalação ausente")?;
    for root in [
        directory.join("../share/codisseu"),
        directory.join("resources"),
    ] {
        if root.join(MANIFEST).exists() {
            return Resources::open(&root).map(Some);
        }
    }
    Ok(None)
}
/// Development fallback is anchored to this build, never current_dir/workspace.
pub fn node() -> Result<PathBuf, String> {
    if let Some(resources) = discover()? {
        return resources.tool("node");
    }
    let local =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/node-v24.22.0-linux-x64/bin/node");
    Ok(if local.is_file() {
        local
    } else {
        PathBuf::from("node")
    })
}
/// Browser helper and CEF directory, resolved only by the browser worker.
/// The desktop application never links libcef or searches the open project.
pub fn browser_engine() -> Result<(PathBuf, PathBuf), String> {
    if let Some(resources) = discover()? {
        let program = resources.tool("browser-engine")?;
        let library = resources.tool("browser-runtime")?;
        let runtime = library.parent().ok_or("Motor do navegador inválido")?;
        return Ok((program, runtime.to_path_buf()));
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/browser");
    let program = base.join("bin/codisseu-browser-engine");
    let runtime = base.join("cef");
    if !program.is_file() || !runtime.join("libcef.so").is_file() {
        return Err("Motor do navegador ausente; prepare tools/install_browser.py ou reinstale o pacote completo".into());
    }
    let program = program
        .canonicalize()
        .map_err(|_| "Motor do navegador inválido")?;
    let canonical_base = base
        .canonicalize()
        .map_err(|_| "Motor do navegador inválido")?;
    let runtime = runtime.canonicalize().map_err(|_| "CEF privado inválido")?;
    let library = runtime
        .join("libcef.so")
        .canonicalize()
        .map_err(|_| "CEF privado inválido")?;
    if !program.starts_with(&canonical_base)
        || !runtime.starts_with(&canonical_base)
        || !library.starts_with(&canonical_base)
    {
        return Err("Motor do navegador fora da instalação".into());
    }
    Ok((program, runtime))
}
/// Installation-owned default. Explicit project interpreters bypass this API.
/// Resolve only on workers; never look inside the current project or global pip.
pub fn python() -> Result<PathBuf, String> {
    private_python_tool("python", "cpython/bin/python3")
}
pub fn ruff() -> Result<PathBuf, String> {
    private_python_tool("ruff", "cpython/bin/ruff")
}
pub fn python_test_runner() -> Result<PathBuf, String> {
    if let Some(resources) = discover()? {
        return resources.tool("python-test-runner");
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/python_test_runner.py");
    if !path.is_file() {
        return Err("Runner Python privado ausente; reinstale o pacote".into());
    }
    Ok(path)
}
fn private_python_tool(role: &str, relative: &str) -> Result<PathBuf, String> {
    if let Some(resources) = discover()? {
        return resources.tool(role);
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tools/python")
        .join(relative);
    if !path.is_file() {
        return Err(
            "Ferramentas Python privadas ausentes; prepare tools/install_python_tools.py".into(),
        );
    }
    Ok(path)
}
/// Adapter interpreter and debugpy package root. The debug target's interpreter
/// remains separate, including virtual environments without debugpy installed.
pub fn python_debug_adapter() -> Result<(PathBuf, PathBuf), String> {
    let adapter = private_python_tool(
        "python-debug",
        "cpython/lib/python3.13/site-packages/debugpy/adapter/__main__.py",
    )?;
    let root = adapter
        .parent()
        .and_then(Path::parent)
        .ok_or("debugpy privado inválido")?;
    Ok((python()?, root.into()))
}
/// The standalone interpreter derives its prefix from its executable. Private
/// adapters use -I explicitly; avoid injecting Python flags here because their
/// child targets must preserve a selected interpreter's normal site behavior.
pub fn python_environment() -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, String> {
    Ok(vec![])
}
/// Read-only stdlib/typeshed roots for language navigation. These paths never
/// grant permission to write dependencies or installation-owned sources.
pub fn python_library_roots() -> Result<Vec<PathBuf>, String> {
    if let Some(resources) = discover()? {
        return resources.python_library_roots();
    }
    checked_python_roots(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/python"))
}
fn checked_python_roots(base: &Path) -> Result<Vec<PathBuf>, String> {
    let declared_base = base.to_path_buf();
    let base = base
        .canonicalize()
        .map_err(|_| "Bibliotecas Python privadas ausentes")?;
    // Canonicalize the parent's lexical path first, but reject a symlinked
    // installation-owned Python tree itself.
    if fs::symlink_metadata(&declared_base)
        .map_err(|_| "Bibliotecas Python privadas ausentes")?
        .file_type()
        .is_symlink()
    {
        return Err("Biblioteca Python fora da instalação".into());
    }
    let mut roots = vec![];
    for relative in [
        "cpython/lib/python3.13",
        "node_modules/basedpyright/dist/typeshed-fallback",
    ] {
        let declared = base.join(relative);
        let path = declared
            .canonicalize()
            .map_err(|_| "Biblioteca Python privada ausente")?;
        if path != declared || !path.starts_with(&base) || !path.is_dir() {
            return Err("Biblioteca Python fora da instalação".into());
        }
        roots.push(path);
    }
    Ok(roots)
}
/// Default Cargo comes from the installation; explicit user commands remain
/// untouched. This performs filesystem reads and belongs on a worker.
pub fn cargo() -> Result<PathBuf, String> {
    if let Some(resources) = discover()? {
        return resources.tool("cargo");
    }
    let executable = if cfg!(windows) { "cargo.exe" } else { "cargo" };
    if let Some(home) = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cargo")))
    {
        let path = home.join("bin").join(executable);
        if path.is_file() {
            return Ok(path);
        }
    }
    Ok(executable.into())
}
/// Child-only defaults for bundled Rust. PATH supplies the private tools while
/// RUSTC/RUSTFLAGS/Cargo configuration remain authoritative when explicit.
/// Never call on the UI thread or mutate the process-global environment.
pub fn rust_environment() -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, String> {
    let Some(resources) = discover()? else {
        return Ok(vec![]);
    };
    if resources.rust_sysroot()?.is_none() {
        return Ok(vec![]);
    }
    let cargo = resources.tool("cargo")?;
    let parent = cargo.parent().ok_or("Cargo privado sem pasta")?;
    let current = std::env::var_os("PATH").unwrap_or_else(|| {
        if cfg!(unix) {
            "/usr/bin:/bin".into()
        } else {
            std::ffi::OsString::new()
        }
    });
    let paths = std::iter::once(parent.to_path_buf()).chain(std::env::split_paths(&current));
    let path = std::env::join_paths(paths).map_err(|_| "PATH dos motores Rust inválido")?;
    Ok(vec![("PATH".into(), path)])
}
/// Formatters are resolved from the installation, never vendor/bin in a project.
pub fn php_formatter() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    if let Some(resources) = discover()? {
        return Ok((
            resources.tool("php")?,
            resources.tool("php-formatter")?,
            resources.tool("php-formatter-helper")?,
        ));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixer = root.join(".tools/php-formatting/php-cs-fixer.phar");
    let helper = root.join("tools/php_format.php");
    if !fixer.is_file() || !helper.is_file() {
        return Err(
            "Motor PHP de formatação ausente; execute tools/install_php_formatter.py".into(),
        );
    }
    Ok((PathBuf::from("php"), fixer, helper))
}
pub fn debug_adapter(php: bool) -> Result<PathBuf, String> {
    if let Some(resources) = discover()? {
        return resources.tool(if php { "php-debug" } else { "js-debug" });
    }
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tools/debug")
        .join(if php {
            "php/extension/out/phpDebug.js"
        } else {
            "js/js-debug/src/dapDebugServer.js"
        }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        for file in ["node", "php", "php-debug", "js-debug", "phpactor", "ts"] {
            fs::write(temp.path().join(file), "fixture").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(temp.path().join(file), fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }
        let manifest = Manifest {
            schema_version: 1,
            tools: ["node", "php", "php-debug", "js-debug"]
                .into_iter()
                .map(|n| (n.into(), n.into()))
                .collect(),
            providers: vec![
                Provider {
                    language: "Php".into(),
                    program: "php".into(),
                    args: vec!["@/phpactor".into()],
                },
                Provider {
                    language: "TypeScript".into(),
                    program: "node".into(),
                    args: vec!["@/ts".into(), "--stdio".into()],
                },
            ],
        };
        fs::write(
            temp.path().join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        temp
    }
    #[test]
    fn core_manifest_is_valid_without_tools_and_never_invents_private_runtimes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(MANIFEST);
        let mut manifest = Manifest {
            schema_version: 1,
            tools: BTreeMap::new(),
            providers: vec![],
        };
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let resources = Resources::open(temp.path()).unwrap();
        assert!(resources.providers().unwrap().is_empty());
        assert!(resources.rust_sysroot().unwrap().is_none());
        assert!(resources.python_library_roots().unwrap().is_empty());
        for role in [
            "php",
            "node",
            "python",
            "rustc",
            "php-debug",
            "browser-engine",
        ] {
            assert!(resources.tool(role).unwrap_err().contains("não incluído"));
        }
        let report: serde_json::Value =
            serde_json::from_str(&resources.report_json().unwrap()).unwrap();
        assert_eq!(report["tools"], serde_json::json!({}));
        assert_eq!(report["providers"], serde_json::json!([]));
        // Do not interpret a partial or unsupported manifest as the core package.
        manifest.schema_version = 2;
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Resources::open(temp.path()).is_err());
        manifest.schema_version = 1;
        manifest.tools.insert("php".into(), "php".into());
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Resources::open(temp.path()).is_err());
    }
    #[test]
    fn python_fourth_provider_is_optional_confined_and_bounded() {
        let temp = setup();
        assert!(
            Resources::open(temp.path())
                .unwrap()
                .python_library_roots()
                .unwrap()
                .is_empty()
        );
        let manifest_path = temp.path().join(MANIFEST);
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        for language in ["Rust", "Python"] {
            manifest.providers.push(Provider {
                language: language.into(),
                program: "node".into(),
                args: vec![],
            });
        }
        manifest.tools.insert("python".into(), "node".into());
        while manifest.tools.len() < 32 {
            manifest
                .tools
                .insert(format!("optional-{}", manifest.tools.len()), "node".into());
        }
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let resources = Resources::open(temp.path()).unwrap();
        assert_eq!(resources.providers().unwrap().len(), 4);
        assert!(resources.python_library_roots().is_err());
        for relative in [
            "cpython/lib/python3.13",
            "node_modules/basedpyright/dist/typeshed-fallback",
        ] {
            fs::create_dir_all(temp.path().join("runtime/python").join(relative)).unwrap();
        }
        assert_eq!(resources.python_library_roots().unwrap().len(), 2);
        #[cfg(unix)]
        {
            let path = temp
                .path()
                .join("runtime/python/node_modules/basedpyright/dist/typeshed-fallback");
            fs::remove_dir(&path).unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), path).unwrap();
            assert!(resources.python_library_roots().is_err());
        }
        manifest.tools.insert("one-too-many".into(), "node".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Resources::open(temp.path()).is_err());
        manifest.tools.remove("one-too-many");
        manifest.providers[3].language = "Rust".into();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Resources::open(temp.path()).is_err());
    }
    #[test]
    fn moved_prefix_with_spaces_resolves_only_installation_paths() {
        let temp = setup();
        let root = temp.path().join("New installation with spaces");
        fs::create_dir(&root).unwrap();
        for file in fs::read_dir(temp.path()).unwrap() {
            let file = file.unwrap();
            if file.file_type().unwrap().is_file() {
                fs::rename(file.path(), root.join(file.file_name())).unwrap();
            }
        }
        let resources = Resources::open(&root).unwrap();
        for command in resources.providers().unwrap() {
            assert!(command.program.starts_with(&root));
            assert!(command.args[0].starts_with(root.to_str().unwrap()));
        }
        assert!(resources.path("../outside").is_err());
        assert!(resources.path("/usr/bin/php").is_err());
    }
    #[test]
    fn formatter_roles_resolve_inside_installation_and_old_manifests_still_open() {
        let temp = setup();
        let resources = Resources::open(temp.path()).unwrap();
        assert!(resources.tool("php-formatter").is_err());
        for file in ["formatter.phar", "helper.php"] {
            fs::write(temp.path().join(file), "fixture").unwrap();
        }
        let manifest_path = temp.path().join(MANIFEST);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["tools"]["php-formatter"] = "formatter.phar".into();
        manifest["tools"]["php-formatter-helper"] = "helper.php".into();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let resources = Resources::open(temp.path()).unwrap();
        for name in ["php-formatter", "php-formatter-helper"] {
            assert!(resources.tool(name).unwrap().starts_with(temp.path()));
        }
    }
    #[test]
    fn optional_rust_provider_and_debugger_preserve_old_manifests_and_reject_duplicates() {
        let temp = setup();
        let path = temp.path().join(MANIFEST);
        let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.providers.push(Provider {
            language: "Rust".into(),
            program: "node".into(),
            args: vec![],
        });
        manifest
            .tools
            .insert("rust-debug".into(), "js-debug".into());
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let resources = Resources::open(temp.path()).unwrap();
        assert_eq!(resources.providers().unwrap().len(), 3);
        assert!(
            resources
                .tool("rust-debug")
                .unwrap()
                .starts_with(temp.path())
        );
        manifest.providers[2].language = "TypeScript".into();
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Resources::open(temp.path()).is_err());
    }
    #[test]
    fn bundled_rust_sysroot_is_optional_and_confined_after_relocation() {
        let temp = setup();
        assert!(
            Resources::open(temp.path())
                .unwrap()
                .rust_sysroot()
                .unwrap()
                .is_none()
        );
        let manifest_path = temp.path().join(MANIFEST);
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.tools.insert("rustc".into(), "node".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let resources = Resources::open(temp.path()).unwrap();
        assert!(resources.rust_sysroot().is_err());
        let sysroot = temp.path().join("runtime/rust/toolchain");
        fs::create_dir_all(&sysroot).unwrap();
        assert_eq!(
            resources.rust_sysroot().unwrap(),
            Some(sysroot.canonicalize().unwrap())
        );
        #[cfg(unix)]
        {
            fs::remove_dir(&sysroot).unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), &sysroot).unwrap();
            assert!(resources.rust_sysroot().is_err());
        }
    }
    #[test]
    fn malformed_missing_external_and_non_executable_tools_are_rejected() {
        let temp = setup();
        let original = fs::read(temp.path().join(MANIFEST)).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
        manifest["providers"][0]["program"] = "../php".into();
        fs::write(
            temp.path().join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            Resources::open(temp.path())
                .unwrap_err()
                .contains("fora da instalação")
        );
        fs::write(temp.path().join(MANIFEST), &original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::{fs::PermissionsExt, fs::symlink};
            fs::set_permissions(temp.path().join("php"), fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(
                Resources::open(temp.path())
                    .unwrap_err()
                    .contains("não executável")
            );
            fs::set_permissions(temp.path().join("php"), fs::Permissions::from_mode(0o755))
                .unwrap();
            fs::remove_file(temp.path().join("node")).unwrap();
            symlink("/bin/sh", temp.path().join("node")).unwrap();
            assert!(
                Resources::open(temp.path())
                    .unwrap_err()
                    .contains("fora da instalação")
            );
        }
        fs::write(temp.path().join(MANIFEST), vec![b'x'; 128 * 1024 + 1]).unwrap();
        assert!(
            Resources::open(temp.path())
                .unwrap_err()
                .contains("grande demais")
        );
    }
}
