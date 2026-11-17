//! Bounded, no-overwrite file moves for Rename. Call only on the I/O worker.
//!
//! Filesystem moves preserve saved bytes; dirty buffers are committed separately
//! by the application. Atomicity is per filesystem rename, with reverse rollback
//! for a batch. This is not a durable crash journal or a filesystem-wide lock.
use std::{
    collections::HashSet,
    fmt,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FILE: usize = 16 * 1024 * 1024;
const MAX_BATCH: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceMove {
    pub source: PathBuf,
    pub target: PathBuf,
    pub expected_disk: Arc<[u8]>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLocation {
    pub source: PathBuf,
    pub target: PathBuf,
}
#[derive(Clone, Debug)]
pub struct ResourceReceipt {
    pub moves: Vec<ResourceLocation>,
}
#[derive(Debug)]
pub struct ResourceError {
    pub message: String,
    /// Operations that could not be rolled back. Their target remains the
    /// recovery location; never retry the transaction as if nothing happened.
    pub recovery: Vec<ResourceLocation>,
}
impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        for entry in &self.recovery {
            write!(
                f,
                "; recuperação necessária: {} → {}",
                entry.source.display(),
                entry.target.display()
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for ResourceError {}
impl From<String> for ResourceError {
    fn from(message: String) -> Self {
        Self {
            message,
            recovery: vec![],
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    len: u64,
}
impl Identity {
    fn from(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            len: metadata.len(),
        }
    }
}
struct PlannedMove {
    change: ResourceMove,
    identity: Identity,
    parent_identity: Identity,
    directory: File,
}
pub struct ResourcePlan {
    root: PathBuf,
    entries: Vec<PlannedMove>,
}
fn contextual(path: &Path, error: impl fmt::Display) -> String {
    format!("{}: {error}", path.display())
}
fn directory(path: &Path) -> Result<(File, Identity), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| contextual(path, e))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(contextual(path, "Diretório regular obrigatório"));
    }
    let file = File::open(path).map_err(|e| contextual(path, e))?;
    let actual = file.metadata().map_err(|e| contextual(path, e))?;
    if Identity::from(&actual) != Identity::from(&metadata) {
        return Err(contextual(path, "Diretório mudou durante abertura"));
    }
    Ok((file, Identity::from(&actual)))
}
fn read_exact(
    path: &Path,
    expected: &[u8],
    identity: Option<&Identity>,
) -> Result<Identity, String> {
    let before = fs::symlink_metadata(path).map_err(|e| contextual(path, e))?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(contextual(path, "Somente arquivo regular sem symlink"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            return Err(contextual(
                path,
                "Arquivo com hard links não suportado em Rename",
            ));
        }
    }
    let before_id = Identity::from(&before);
    if before.len() != expected.len() as u64
        || before.len() > MAX_FILE as u64
        || identity.is_some_and(|id| id != &before_id)
    {
        return Err(contextual(path, "Arquivo mudou desde o snapshot de Rename"));
    }
    let file = File::open(path).map_err(|e| contextual(path, e))?;
    if Identity::from(&file.metadata().map_err(|e| contextual(path, e))?) != before_id {
        return Err(contextual(
            path,
            "Identidade do arquivo mudou durante abertura",
        ));
    }
    let mut bytes = Vec::with_capacity(expected.len());
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| contextual(path, e))?;
    if bytes != expected {
        return Err(contextual(
            path,
            "Bytes em disco divergem do snapshot salvo",
        ));
    }
    let after = fs::symlink_metadata(path).map_err(|e| contextual(path, e))?;
    if after.file_type().is_symlink() || Identity::from(&after) != before_id {
        return Err(contextual(path, "Arquivo mudou durante leitura"));
    }
    Ok(before_id)
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(contextual(path, e)),
        Ok(_) => Err(contextual(
            path,
            "Destino já existe; Rename nunca sobrescreve",
        )),
    }
}
fn same_directory(root: &Path, change: &ResourceMove) -> Result<PathBuf, String> {
    if !change.source.is_absolute()
        || !change.target.is_absolute()
        || change.source == change.target
    {
        return Err("Rename exige caminhos absolutos distintos".into());
    }
    let parent = change.source.parent().ok_or("Arquivo sem diretório")?;
    if Some(parent) != change.target.parent() {
        return Err("Mover entre diretórios ainda não está certificado".into());
    }
    if !parent.starts_with(root)
        || fs::canonicalize(parent).map_err(|e| contextual(parent, e))? != parent
    {
        return Err("Diretório fora do workspace ou com alias/symlink".into());
    }
    if fs::canonicalize(&change.source).map_err(|e| contextual(&change.source, e))? != change.source
    {
        return Err("Origem possui alias/symlink".into());
    }
    if change.source.file_name().is_none() || change.target.file_name().is_none() {
        return Err("Nome de arquivo ausente".into());
    }
    Ok(parent.to_owned())
}
impl ResourcePlan {
    pub fn prepare(workspace: &Path, moves: &[ResourceMove]) -> Result<Self, String> {
        #[cfg(not(target_os = "linux"))]
        return Err("Rename de arquivos ainda não certificado nesta plataforma".into());
        #[cfg(target_os = "linux")]
        {
            if moves.is_empty() || moves.len() > 32 {
                return Err("Rename exige 1..32 operações de arquivo".into());
            }
            if moves.iter().any(|m| m.expected_disk.len() > MAX_FILE)
                || moves.iter().map(|m| m.expected_disk.len()).sum::<usize>() > MAX_BATCH
            {
                return Err("Snapshots de arquivos excedem limites 16/32 MiB".into());
            }
            let root = fs::canonicalize(workspace).map_err(|e| contextual(workspace, e))?;
            if !root.is_dir() {
                return Err("Workspace não é diretório".into());
            }
            let mut sources = HashSet::new();
            let mut targets = HashSet::new();
            for change in moves {
                if !sources.insert(&change.source) || !targets.insert(&change.target) {
                    return Err("Origem/destino duplicado na transação".into());
                }
            }
            if targets.iter().any(|p| sources.contains(p)) {
                return Err("Ciclos/cadeias de destinos ocupados não são suportados".into());
            }
            let mut entries = Vec::new();
            for change in moves {
                let parent = same_directory(&root, change)?;
                absent(&change.target)?;
                let identity = read_exact(&change.source, &change.expected_disk, None)?;
                let (directory, parent_identity) = directory(&parent)?;
                entries.push(PlannedMove {
                    change: change.clone(),
                    identity,
                    parent_identity,
                    directory,
                });
            }
            Ok(Self { root, entries })
        }
    }
    pub fn apply(self) -> Result<ResourceReceipt, ResourceError> {
        self.apply_with(|_| Ok(()))
    }
    fn apply_with(
        self,
        mut checkpoint: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<ResourceReceipt, ResourceError> {
        let mut moved = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let result = (|| {
                checkpoint(index)?;
                self.verify_entry(entry)?;
                rename_no_replace(&entry.directory, &entry.change.source, &entry.change.target)?;
                Ok(())
            })();
            if let Err(message) = result {
                return Err(self.rollback(&moved, message));
            }
            moved.push(index);
            if let Err(message) = read_exact(
                &entry.change.target,
                &entry.change.expected_disk,
                Some(&entry.identity),
            ) {
                return Err(self.rollback(&moved, message));
            }
        }
        Ok(ResourceReceipt {
            moves: self
                .entries
                .iter()
                .map(|e| ResourceLocation {
                    source: e.change.source.clone(),
                    target: e.change.target.clone(),
                })
                .collect(),
        })
    }
    fn verify_entry(&self, entry: &PlannedMove) -> Result<(), String> {
        let parent = same_directory(&self.root, &entry.change)?;
        let now = fs::symlink_metadata(&parent).map_err(|e| contextual(&parent, e))?;
        let opened = entry
            .directory
            .metadata()
            .map_err(|e| contextual(&parent, e))?;
        // Directory sizes can change as unrelated entries are created. Device and
        // inode are the stable identity; never compare its length for freshness.
        if !same_object(&Identity::from(&now), &entry.parent_identity)
            || !same_object(&Identity::from(&opened), &entry.parent_identity)
        {
            return Err(contextual(&parent, "Identidade do diretório mudou"));
        }
        absent(&entry.change.target)?;
        read_exact(
            &entry.change.source,
            &entry.change.expected_disk,
            Some(&entry.identity),
        )?;
        Ok(())
    }
    fn rollback(&self, moved: &[usize], message: String) -> ResourceError {
        let mut error = ResourceError {
            message,
            recovery: vec![],
        };
        for &index in moved.iter().rev() {
            let entry = &self.entries[index];
            // An external write must be preserved. A changed file stays at its
            // current path and is reported for recovery, never overwritten.
            let result = read_exact(
                &entry.change.target,
                &entry.change.expected_disk,
                Some(&entry.identity),
            )
            .and_then(|_| absent(&entry.change.source))
            .and_then(|_| {
                rename_no_replace(&entry.directory, &entry.change.target, &entry.change.source)
            });
            if let Err(reason) = result {
                error.message.push_str(&format!("; rollback: {reason}"));
                error.recovery.push(ResourceLocation {
                    source: entry.change.source.clone(),
                    target: entry.change.target.clone(),
                });
            }
        }
        error
    }
}
fn same_object(a: &Identity, b: &Identity) -> bool {
    #[cfg(unix)]
    {
        a.device == b.device && a.inode == b.inode
    }
    #[cfg(not(unix))]
    {
        a == b
    }
}
#[cfg(target_os = "linux")]
fn rename_no_replace(directory: &File, source: &Path, target: &Path) -> Result<(), String> {
    use std::{
        ffi::{CString, c_char, c_int, c_uint},
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    unsafe extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    let old = CString::new(source.file_name().ok_or("Origem sem nome")?.as_bytes())
        .map_err(|_| "NUL no caminho")?;
    let new = CString::new(target.file_name().ok_or("Destino sem nome")?.as_bytes())
        .map_err(|_| "NUL no caminho")?;
    // Both names are relative to the held directory descriptor; RENAME_NOREPLACE
    // makes destination existence a kernel-checked precondition, not a TOCTOU.
    let result = unsafe {
        renameat2(
            directory.as_raw_fd(),
            old.as_ptr(),
            directory.as_raw_fd(),
            new.as_ptr(),
            1,
        )
    };
    if result != 0 {
        return Err(contextual(target, std::io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn rename_no_replace(_: &File, _: &Path, _: &Path) -> Result<(), String> {
    Err("Rename de arquivos ainda não certificado nesta plataforma".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    fn change(root: &Path, old: &str, new: &str, bytes: &[u8]) -> ResourceMove {
        fs::write(root.join(old), bytes).unwrap();
        ResourceMove {
            source: root.join(old),
            target: root.join(new),
            expected_disk: Arc::from(bytes),
        }
    }
    #[test]
    fn moves_preserve_saved_bytes_inode_permissions_and_support_inverse_after_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let m = change(dir.path(), "Weather.ts", "Forecast.ts", b"saved bytes\n");
        fs::set_permissions(&m.source, fs::Permissions::from_mode(0o640)).unwrap();
        let identity = Identity::from(&fs::metadata(&m.source).unwrap());
        let receipt = ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m))
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(
            receipt.moves,
            vec![ResourceLocation {
                source: m.source.clone(),
                target: m.target.clone()
            }]
        );
        assert!(!m.source.exists());
        assert_eq!(fs::read(&m.target).unwrap(), b"saved bytes\n");
        assert_eq!(Identity::from(&fs::metadata(&m.target).unwrap()), identity);
        assert_eq!(
            fs::metadata(&m.target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        // Save is a separate operation: Undo moves its latest saved bytes back;
        // it must not silently restore old content over the explicit Save.
        fs::write(&m.target, b"explicit Save after dirty-buffer rename\n").unwrap();
        let reverse = ResourceMove {
            source: m.target.clone(),
            target: m.source.clone(),
            expected_disk: Arc::from(&b"explicit Save after dirty-buffer rename\n"[..]),
        };
        ResourcePlan::prepare(dir.path(), std::slice::from_ref(&reverse))
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(fs::read(&m.source).unwrap(), reverse.expected_disk.as_ref());
        assert!(!m.target.exists());
    }
    #[test]
    fn occupied_or_late_destination_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let m = change(dir.path(), "A.ts", "B.ts", b"source");
        fs::write(&m.target, b"occupied").unwrap();
        assert!(ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).is_err());
        fs::remove_file(&m.target).unwrap();
        let plan = ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).unwrap();
        fs::write(&m.target, b"late external write").unwrap();
        let error = plan.apply().unwrap_err();
        assert!(error.recovery.is_empty());
        assert_eq!(fs::read(&m.source).unwrap(), b"source");
        assert_eq!(fs::read(&m.target).unwrap(), b"late external write");
    }
    #[test]
    fn batch_failure_rolls_back_every_completed_move_in_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let moves = vec![
            change(dir.path(), "A.ts", "X.ts", b"a"),
            change(dir.path(), "B.ts", "Y.ts", b"b"),
            change(dir.path(), "C.ts", "Z.ts", b"c"),
        ];
        let plan = ResourcePlan::prepare(dir.path(), &moves).unwrap();
        let error = plan
            .apply_with(|index| {
                if index == 2 {
                    Err("injected later failure".into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(error.recovery.is_empty(), "{error}");
        for m in &moves {
            assert_eq!(fs::read(&m.source).unwrap(), m.expected_disk.as_ref());
            assert!(!m.target.exists());
        }
    }
    #[test]
    fn rollback_reports_recovery_when_competing_writer_occupies_original_or_modifies_target() {
        for mutate_target in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let moves = vec![
                change(dir.path(), "A.ts", "X.ts", b"original"),
                change(dir.path(), "B.ts", "Y.ts", b"b"),
            ];
            let plan = ResourcePlan::prepare(dir.path(), &moves).unwrap();
            let error = plan
                .apply_with(|index| {
                    if index == 1 {
                        fs::write(
                            if mutate_target {
                                &moves[0].target
                            } else {
                                &moves[0].source
                            },
                            b"external content",
                        )
                        .unwrap();
                        Err("injected failure with concurrent writer".into())
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(
                error.recovery,
                vec![ResourceLocation {
                    source: moves[0].source.clone(),
                    target: moves[0].target.clone()
                }]
            );
            assert_eq!(fs::read(&moves[1].source).unwrap(), b"b");
            assert!(!moves[1].target.exists());
            if mutate_target {
                assert!(!moves[0].source.exists());
                assert_eq!(fs::read(&moves[0].target).unwrap(), b"external content");
            } else {
                assert_eq!(fs::read(&moves[0].source).unwrap(), b"external content");
                assert_eq!(fs::read(&moves[0].target).unwrap(), b"original");
            }
            assert!(error.to_string().contains("recuperação necessária"));
        }
    }
    #[test]
    fn stale_bytes_and_same_bytes_replaced_inode_cannot_move() {
        let dir = tempfile::tempdir().unwrap();
        let m = change(dir.path(), "A.ts", "B.ts", b"original");
        let plan = ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).unwrap();
        fs::write(&m.source, b"modified").unwrap();
        assert!(plan.apply().is_err());
        assert!(!m.target.exists());
        fs::write(&m.source, b"original").unwrap();
        let plan = ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).unwrap();
        fs::rename(&m.source, dir.path().join("kept-original.ts")).unwrap();
        fs::write(&m.source, b"original").unwrap();
        assert!(plan.apply().is_err());
        assert!(m.source.exists());
        assert!(!m.target.exists());
    }
    #[test]
    fn symlinks_hardlinks_external_paths_and_renamed_parent_are_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let m = change(dir.path(), "A.ts", "B.ts", b"original");
        fs::write(outside.path().join("outside.ts"), b"outside").unwrap();
        fs::remove_file(&m.source).unwrap();
        symlink(outside.path().join("outside.ts"), &m.source).unwrap();
        assert!(ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).is_err());
        fs::remove_file(&m.source).unwrap();
        fs::write(&m.source, b"original").unwrap();
        symlink(outside.path().join("outside.ts"), &m.target).unwrap();
        assert!(ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).is_err());
        fs::remove_file(&m.target).unwrap();
        fs::hard_link(&m.source, dir.path().join("alias.ts")).unwrap();
        assert!(ResourcePlan::prepare(dir.path(), std::slice::from_ref(&m)).is_err());
        fs::remove_file(dir.path().join("alias.ts")).unwrap();
        let external = ResourceMove {
            source: m.source.clone(),
            target: outside.path().join("B.ts"),
            expected_disk: m.expected_disk.clone(),
        };
        assert!(ResourcePlan::prepare(dir.path(), &[external]).is_err());
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let sub_move = change(&sub, "A.ts", "B.ts", b"inside");
        let plan = ResourcePlan::prepare(dir.path(), std::slice::from_ref(&sub_move)).unwrap();
        fs::rename(&sub, dir.path().join("moved-sub")).unwrap();
        fs::create_dir(&sub).unwrap();
        fs::write(&sub_move.source, b"inside").unwrap();
        assert!(plan.apply().is_err());
        assert!(!sub_move.target.exists());
        assert!(dir.path().join("moved-sub/A.ts").exists());
        assert_eq!(
            fs::read(outside.path().join("outside.ts")).unwrap(),
            b"outside"
        );
    }
    #[test]
    fn duplicate_cycles_missing_parent_and_limits_fail_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let a = change(dir.path(), "A.ts", "B.ts", b"a");
        assert!(ResourcePlan::prepare(dir.path(), &[]).is_err());
        assert!(ResourcePlan::prepare(dir.path(), &[a.clone(), a.clone()]).is_err());
        let b = change(dir.path(), "B.ts", "A.ts", b"b");
        assert!(ResourcePlan::prepare(dir.path(), &[a.clone(), b]).is_err());
        let mut huge = a.clone();
        huge.expected_disk = Arc::from(vec![0; MAX_FILE + 1]);
        assert!(ResourcePlan::prepare(dir.path(), &[huge]).is_err());
        assert_eq!(fs::read(&a.source).unwrap(), b"a");
        assert_eq!(fs::read(&a.target).unwrap(), b"b");
    }
}
