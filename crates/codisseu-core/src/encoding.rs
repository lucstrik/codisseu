//! Strict, reversible disk codecs. The editor always holds Unicode.
use super::*;
use std::{fs, io::Read, path::Path};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileEncoding {
    #[default]
    Utf8,
    Latin1,
    Windows1252,
}
impl FileEncoding {
    pub const ALL: [Self; 3] = [Self::Utf8, Self::Latin1, Self::Windows1252];
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Latin1 => "ISO-8859-1",
            Self::Windows1252 => "Windows-1252",
        }
    }
    pub fn decode(self, bytes: &[u8]) -> Result<(String, Self), String> {
        let encoding = if bytes.starts_with(b"\xef\xbb\xbf") {
            Self::Utf8
        } else {
            self
        };
        let text = match encoding {
            Self::Utf8 => String::from_utf8(bytes.to_vec()).map_err(|_| "UTF-8 inválido: selecione a codificação do projeto no rodapé; arquivo preservado".to_owned())?,
            Self::Latin1 => bytes.iter().map(|&b| char::from(b)).collect(),
            Self::Windows1252 => bytes.iter().map(|&b| if (0x80..=0x9f).contains(&b) { CP1252[(b - 0x80) as usize] } else { char::from(b) }).collect(),
        };
        if encoding.encode(&text)? != bytes {
            return Err("Codificação não permite leitura reversível; arquivo preservado".into());
        }
        Ok((text, encoding))
    }
    pub fn encode(self, text: &str) -> Result<Vec<u8>, String> {
        if self == Self::Utf8 {
            return Ok(text.as_bytes().to_vec());
        }
        text.chars().map(|c| {
            let byte = if self == Self::Windows1252 {
                CP1252.iter().position(|&mapped| mapped == c).map(|i| i as u8 + 0x80)
                    .or_else(|| ((c as u32 <= 0xff) && !(0x80..=0x9f).contains(&(c as u32))).then_some(c as u8))
            } else { (c as u32 <= 0xff).then_some(c as u8) };
            byte.ok_or_else(|| format!("Não foi possível salvar em {}: caractere {c:?} (U+{:04X}) não representável. Arquivo original preservado", self.label(), c as u32))
        }).collect()
    }
}
// Undefined CP1252 slots remain their C1 scalar, preserving every byte.
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    schema_version: u32,
    projects: BTreeMap<PathBuf, FileEncoding>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            projects: BTreeMap::new(),
        }
    }
}
fn load(path: &Path) -> Result<Profile, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err("Perfil de encoding deve ser arquivo regular sem symlink".into());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Profile::default()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let bytes = match fs::File::open(path) {
        Ok(f) => {
            let mut bytes = Vec::new();
            f.take(128 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            bytes
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Profile::default()),
        Err(e) => return Err(e.to_string()),
    };
    if bytes.len() > 128 * 1024 {
        return Err("Perfil de encoding excede 128 KiB".into());
    }
    let profile: Profile = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Perfil de encoding inválido; preservado: {e}"))?;
    if profile.schema_version != 1
        || profile.projects.len() > 512
        || profile.projects.keys().any(|p| {
            !p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        })
    {
        return Err("Perfil de encoding inválido; preservado".into());
    }
    Ok(profile)
}
pub(super) struct Resolver {
    settings: Profile,
}

/// A known PHP provider ambiguity mixes saved legacy byte offsets and the
/// Unicode workspace after didOpen. Never apply those uncertain coordinates.
pub(super) fn validate_rename_ranges(
    path: &Path,
    encoding: FileEncoding,
    text: &str,
    edits: &[RenameTextEdit],
) -> Result<(), String> {
    if Language::for_path(path) == Some(Language::Php)
        && encoding != FileEncoding::Utf8
        && edits.iter().any(|edit| {
            utf16_byte_offset(text, edit.end_line, edit.end_character)
                .is_none_or(|end| !text[..end].is_ascii())
        })
    {
        return Err("Rename PHP indisponível para posições legadas precedidas por acentos: o provider não certifica os intervalos; arquivos preservados".into());
    }
    Ok(())
}
impl Resolver {
    pub(super) fn new(profile: Option<&Path>) -> Result<Self, String> {
        Ok(Self {
            settings: profile.map(load).transpose()?.unwrap_or_default(),
        })
    }
    pub(super) fn resolve(
        &self,
        path: &Path,
        workspace: Option<&Path>,
    ) -> Result<FileEncoding, String> {
        context_with(&self.settings, path, workspace).map(|(_, e)| e)
    }
}
pub(super) fn resolve(profile: Option<&Path>, path: &Path) -> Result<FileEncoding, String> {
    resolve_for(profile, path, None)
}
pub(super) fn resolve_for(
    profile: Option<&Path>,
    path: &Path,
    workspace: Option<&Path>,
) -> Result<FileEncoding, String> {
    context(profile, path, workspace).map(|(_, encoding)| encoding)
}
pub(super) fn context(
    profile: Option<&Path>,
    path: &Path,
    workspace: Option<&Path>,
) -> Result<(PathBuf, FileEncoding), String> {
    let settings = profile.map(load).transpose()?.unwrap_or_default();
    context_with(&settings, path, workspace)
}
fn context_with(
    settings: &Profile,
    path: &Path,
    workspace: Option<&Path>,
) -> Result<(PathBuf, FileEncoding), String> {
    let mut root = projects::resolve_root(workspace, path);
    // Explicit folder choices create a project boundary even without a manifest.
    if let Some(configured) = settings
        .projects
        .keys()
        .filter(|r| path.starts_with(r) && r.starts_with(&root))
        .max_by_key(|r| r.components().count())
    {
        root = configured.clone();
    }
    if let Some(encoding) = settings.projects.get(&root) {
        return Ok((root, *encoding));
    }
    let boundary = if workspace.is_some_and(super::workspace_file::is_workspace_file) {
        Some(root.as_path())
    } else {
        workspace
    };
    let encoding = super::editorconfig::resolve(path, boundary)?.unwrap_or_default();
    Ok((root, encoding))
}
pub(super) fn project_contexts(
    profile: Option<&Path>,
    root: &Path,
    entries: &[TreeEntry],
) -> Result<BTreeMap<PathBuf, FileEncoding>, String> {
    let settings = profile.map(load).transpose()?.unwrap_or_default();
    let mut result = BTreeMap::from([(
        root.to_path_buf(),
        context_with(&settings, root, Some(root))?.1,
    )]);
    for entry in entries.iter().filter(|e| {
        e.depth == 0
            && e.is_dir
            && fs::symlink_metadata(&e.path).is_ok_and(|m| !m.file_type().is_symlink())
    }) {
        if result.len() >= 512 {
            break;
        }
        // Directories already visible/loaded only. Never recursively index here.
        if let Ok((project, encoding)) = context_with(&settings, &entry.path, Some(root)) {
            result.entry(project).or_insert(encoding);
        } else {
            result
                .entry(projects::resolve_root(Some(root), &entry.path))
                .or_default();
        }
    }
    for (project, encoding) in settings
        .projects
        .iter()
        .filter(|(p, _)| p.starts_with(root))
    {
        if result.len() >= 512 {
            break;
        }
        result.insert(project.clone(), *encoding);
    }
    Ok(result)
}
pub(super) fn persist(path: &Path, root: PathBuf, encoding: FileEncoding) -> Result<(), String> {
    use std::io::Write;
    let mut profile = load(path)?;
    profile.projects.insert(root, encoding);
    if profile.projects.len() > 512 {
        return Err("Limite de 512 preferências de encoding".into());
    }
    let bytes = serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("Perfil de encoding excede 128 KiB".into());
    }
    if let Ok(meta) = fs::symlink_metadata(path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("Perfil de encoding deve ser arquivo regular sem symlink".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&bytes)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn settle(app: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < end, "{}", app.status);
            std::thread::sleep(Duration::from_millis(2));
        }
        app.poll();
    }
    fn run(app: &mut AppService, cmd: Command) {
        app.dispatch(cmd).unwrap();
        settle(app);
        assert_eq!(app.last_error, None);
    }
    fn open(app: &mut AppService, root: &Path, path: &Path) {
        run(app, Command::OpenWorkspace(root.into()));
        run(app, Command::OpenFile(path.into()));
    }
    #[test]
    fn php_legacy_rename_requires_unambiguous_ranges_and_preserves_utf8_support() {
        let edit = RenameTextEdit {
            start_line: 0,
            start_character: 5,
            end_line: 0,
            end_character: 9,
            new_text: "Next".into(),
        };
        assert!(
            validate_rename_ranges(
                Path::new("a.php"),
                FileEncoding::Latin1,
                "é // Name",
                std::slice::from_ref(&edit)
            )
            .is_err()
        );
        assert!(
            validate_rename_ranges(
                Path::new("a.php"),
                FileEncoding::Latin1,
                "// Name // é",
                std::slice::from_ref(&RenameTextEdit {
                    end_character: 7,
                    ..edit.clone()
                })
            )
            .is_ok()
        );
        assert!(
            validate_rename_ranges(
                Path::new("a.php"),
                FileEncoding::Utf8,
                "é // Name",
                std::slice::from_ref(&edit)
            )
            .is_ok()
        );
        assert!(
            validate_rename_ranges(
                Path::new("a.js"),
                FileEncoding::Windows1252,
                "é // Name",
                std::slice::from_ref(&edit)
            )
            .is_ok()
        );
    }
    #[test]
    fn codecs_round_trip_all_256_bytes_without_conflating_latin1_and_cp1252() {
        let bytes: Vec<u8> = (0..=255).collect();
        for codec in [FileEncoding::Latin1, FileEncoding::Windows1252] {
            let (text, actual) = codec.decode(&bytes).unwrap();
            assert_eq!(actual, codec);
            assert_eq!(codec.encode(&text).unwrap(), bytes);
            assert!(
                codec
                    .encode("😃")
                    .unwrap_err()
                    .contains("original preservado")
            );
        }
        assert_eq!(FileEncoding::Latin1.decode(&[0x80]).unwrap().0, "\u{80}");
        assert_eq!(FileEncoding::Windows1252.decode(&[0x80]).unwrap().0, "€");
        assert!(FileEncoding::Utf8.decode(&[0xe7]).is_err());
        for codec in FileEncoding::ALL {
            assert_eq!(
                codec.decode(b"\xef\xbb\xbfa\r\n").unwrap(),
                ("\u{feff}a\r\n".into(), FileEncoding::Utf8)
            );
        }
    }
    #[test]
    fn latin1_project_edits_save_preserve_original_bytes_and_reject_unrepresentable_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        let path = root.join("a.php");
        let original = b"<?php // a\xe7\xe3o\r\n";
        fs::write(&path, original).unwrap();
        let mut app = AppService::new(dir.path().join("profile/session.json"));
        open(&mut app, &root, &path);
        let d = app.active_document().unwrap();
        assert_eq!(d.text(), "<?php // ação\r\n");
        assert_eq!(d.encoding(), FileEncoding::Latin1);
        let id = d.id;
        run(&mut app, Command::Save { id });
        assert_eq!(fs::read(&path).unwrap(), original);
        let version = app.active_document().unwrap().version;
        run(
            &mut app,
            Command::Edit {
                id,
                expected_version: version,
                text: "<?php // ação e café\r\n".into(),
            },
        );
        run(&mut app, Command::Save { id });
        let baseline = fs::read(&path).unwrap();
        assert_eq!(baseline, b"<?php // a\xe7\xe3o e caf\xe9\r\n");
        let version = app.active_document().unwrap().version;
        run(
            &mut app,
            Command::Edit {
                id,
                expected_version: version,
                text: "<?php // 😃".into(),
            },
        );
        app.dispatch(Command::Save { id }).unwrap();
        settle(&mut app);
        assert!(
            app.last_error
                .as_ref()
                .unwrap()
                .contains("não representável")
        );
        assert_eq!(fs::read(&path).unwrap(), baseline);
        assert!(app.active_document().unwrap().is_dirty());
        assert!(
            app.dispatch(Command::SetProjectEncoding(FileEncoding::Utf8))
                .is_err()
        );
        assert_eq!(app.active_document().unwrap().text(), "<?php // 😃");
    }
    #[test]
    fn project_preference_survives_restart_workspace_switch_and_session_restore() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let path = a.join("accent.php");
        fs::write(&path, b"<?php // pre\xe7o \x80\r\n").unwrap();
        let session = dir.path().join("profile/session.json");
        let mut app = AppService::new(session.clone());
        run(&mut app, Command::OpenWorkspace(a.clone()));
        run(
            &mut app,
            Command::SetProjectEncoding(FileEncoding::Windows1252),
        );
        run(&mut app, Command::OpenFile(path.clone()));
        let d = app.active_document().unwrap();
        assert_eq!(d.text(), "<?php // preço €\r\n");
        let id = d.id;
        run(&mut app, Command::Save { id });
        assert_eq!(fs::read(&path).unwrap(), b"<?php // pre\xe7o \x80\r\n");
        run(&mut app, Command::OpenWorkspace(b.clone()));
        assert_eq!(app.project_encoding(), FileEncoding::Utf8);
        run(&mut app, Command::OpenWorkspace(a.clone()));
        assert_eq!(app.project_encoding(), FileEncoding::Windows1252);
        run(&mut app, Command::OpenFile(path.clone()));
        run(&mut app, Command::PersistSession);
        drop(app);
        let mut app = AppService::new(session.clone());
        run(&mut app, Command::RestoreSession);
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Windows1252
        );
        assert_eq!(
            app.active_document().unwrap().text(),
            "<?php // preço €\r\n"
        );
        drop(app);
        let mut app = AppService::new(session);
        open(&mut app, &a, &path);
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Windows1252
        );
    }
    #[test]
    fn reload_is_atomic_on_invalid_utf8_and_profile_failure_and_blocks_interleaved_commands() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let session = dir.path().join("session.json");
        let path = root.join("a.php");
        fs::write(&path, b"<?php // a\xe7\xe3o").unwrap();
        let mut app = AppService::new(session.clone());
        run(&mut app, Command::OpenWorkspace(root.clone()));
        run(&mut app, Command::SetProjectEncoding(FileEncoding::Latin1));
        run(&mut app, Command::OpenFile(path.clone()));
        let d = app.active_document().unwrap();
        let id = d.id;
        let version = d.version;
        let profile = fs::read(session.with_extension("encodings.json")).unwrap();
        app.dispatch(Command::SetProjectEncoding(FileEncoding::Utf8))
            .unwrap();
        assert!(
            app.dispatch(Command::Edit {
                id,
                expected_version: version,
                text: "WRONG".into()
            })
            .is_err()
        );
        assert!(app.dispatch(Command::CancelAll).is_err());
        settle(&mut app);
        assert_eq!(app.active_document().unwrap().version, version);
        assert_eq!(app.active_document().unwrap().text(), "<?php // ação");
        assert_eq!(
            fs::read(session.with_extension("encodings.json")).unwrap(),
            profile
        );
        assert!(app.last_error.as_ref().unwrap().contains("UTF-8 inválido"));
        fs::write(session.with_extension("encodings.json"), b"invalid profile").unwrap();
        app.dispatch(Command::SetProjectEncoding(FileEncoding::Windows1252))
            .unwrap();
        settle(&mut app);
        assert!(
            app.last_error
                .as_ref()
                .unwrap()
                .contains("Perfil de encoding inválido")
        );
        assert_eq!(app.active_document().unwrap().version, version);
        assert_eq!(app.project_encoding(), FileEncoding::Latin1);
        assert_eq!(
            fs::read(session.with_extension("encodings.json")).unwrap(),
            b"invalid profile"
        );
    }
    #[test]
    fn legacy_save_conflict_and_external_reload_are_byte_safe() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        let path = dir.path().join("a.php");
        fs::write(&path, b"<?php // a\xe7\xe3o").unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        open(&mut app, dir.path(), &path);
        let d = app.active_document().unwrap();
        let id = d.id;
        let version = d.version;
        run(
            &mut app,
            Command::Edit {
                id,
                expected_version: version,
                text: "local".into(),
            },
        );
        fs::write(&path, b"<?php // caf\xe9").unwrap();
        app.dispatch(Command::Save { id }).unwrap();
        settle(&mut app);
        assert!(app.last_error.as_ref().unwrap().contains("Conflito"));
        assert_eq!(fs::read(&path).unwrap(), b"<?php // caf\xe9");
        assert_eq!(app.active_document().unwrap().text(), "local");
        let file = crate::worker::external_document(&path, FileEncoding::Latin1).unwrap();
        assert_eq!(file.text, "<?php // café");
        assert_eq!(file.encoding, FileEncoding::Latin1);
    }
    #[test]
    fn bom_and_clean_external_update_keep_encoding_and_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        let path = root.join("a.php");
        fs::write(&path, b"\xef\xbb\xbf<?php // caf\xc3\xa9\r\n").unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        open(&mut app, &root, &path);
        let id = app.active_document().unwrap().id;
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Utf8
        );
        run(&mut app, Command::Save { id });
        assert_eq!(
            fs::read(&path).unwrap(),
            b"\xef\xbb\xbf<?php // caf\xc3\xa9\r\n"
        );
        fs::write(&path, b"<?php // a\xe7\xe3o\r\n").unwrap();
        // A BOM previously fixed this document as UTF-8. Invalid external UTF-8
        // must preserve that buffer until explicit project reload.
        run(&mut app, Command::SetProjectEncoding(FileEncoding::Latin1));
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Latin1
        );
        fs::write(&path, b"<?php // caf\xe9\r\n").unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while app.active_document().unwrap().text() != "<?php // café\r\n" {
            app.poll();
            assert!(Instant::now() < end, "{}", app.status);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!app.active_document().unwrap().is_dirty());
        run(&mut app, Command::Save { id });
        assert_eq!(fs::read(&path).unwrap(), b"<?php // caf\xe9\r\n");
    }
    #[test]
    fn nested_projects_keep_independent_overrides_under_aggregate_root() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        let nested = parent.join("child");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("a.php");
        fs::write(&path, b"<?php // \x80").unwrap();
        let session = dir.path().join("session.json");
        let mut app = AppService::new(session.clone());
        run(&mut app, Command::OpenWorkspace(nested.clone()));
        run(
            &mut app,
            Command::SetProjectEncoding(FileEncoding::Windows1252),
        );
        run(&mut app, Command::OpenWorkspace(parent.clone()));
        run(&mut app, Command::SetProjectEncoding(FileEncoding::Latin1));
        run(&mut app, Command::OpenFile(path.clone()));
        assert_eq!(app.active_document().unwrap().text(), "<?php // €");
        run(&mut app, Command::PersistSession);
        drop(app);
        let mut app = AppService::new(session);
        run(&mut app, Command::RestoreSession);
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Windows1252
        );
        run(&mut app, Command::OpenWorkspace(nested));
        run(&mut app, Command::OpenFile(path));
        assert_eq!(
            app.active_document().unwrap().encoding(),
            FileEncoding::Windows1252
        );
        assert_eq!(app.active_document().unwrap().text(), "<?php // €");
    }
    #[test]
    #[cfg(unix)]
    fn encoding_reload_rejects_replaced_symlink_without_changing_baseline_or_preference() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let path = root.join("a.php");
        let outside = dir.path().join("outside.php");
        fs::write(&path, "<?php // original").unwrap();
        fs::write(&outside, b"<?php // caf\xe9").unwrap();
        let session = dir.path().join("session.json");
        let mut app = AppService::new(session.clone());
        open(&mut app, &root, &path);
        let version = app.active_document().unwrap().version;
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        app.dispatch(Command::SetProjectEncoding(FileEncoding::Latin1))
            .unwrap();
        settle(&mut app);
        assert!(app.last_error.as_ref().unwrap().contains("link simbólico"));
        assert_eq!(app.active_document().unwrap().text(), "<?php // original");
        assert_eq!(app.active_document().unwrap().version, version);
        assert_eq!(app.project_encoding(), FileEncoding::Utf8);
        assert!(!session.with_extension("encodings.json").exists());
        assert_eq!(fs::read(&outside).unwrap(), b"<?php // caf\xe9");
    }
}
