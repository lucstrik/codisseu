use crate::{
    BlameLine, BranchInfo, CommitChange, CommitDetails, CommitEntry, FileVersion, GitError,
    GitErrorKind, HistoryPage, Repository, TagInfo,
    repository::{args, path_from_bytes, trim_line, valid_oid},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn utf8(bytes: &[u8]) -> Result<String, GitError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        GitError::unsupported(
            "Metadado Git não Unicode não pode ser usado como referência na interface",
        )
    })
}
impl Repository {
    pub fn history(
        &self,
        skip: usize,
        limit: usize,
        path: Option<&Path>,
        cancel: &AtomicBool,
    ) -> Result<HistoryPage, GitError> {
        self.history_reference(skip, limit, path, None, cancel)
    }
    /// Read one immutable commit topology. Reuse `resolved_reference` on subsequent
    /// pages to keep pagination stable when the named branch moves externally.
    pub fn history_reference(
        &self,
        skip: usize,
        limit: usize,
        path: Option<&Path>,
        reference: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<HistoryPage, GitError> {
        if skip > 5000 || limit == 0 || limit > 200 {
            return Err(GitError::invalid(
                "Histórico limitado a200 commits por página/5000 de deslocamento",
            ));
        }
        let resolved_reference = reference.map(|r| self.resolve(r, cancel)).transpose()?;
        if resolved_reference.is_none()
            && !self
                .run(&args(&["rev-parse", "--verify", "HEAD"]), None, cancel)?
                .status
                .success()
        {
            return Ok(HistoryPage {
                resolved_reference,
                skip,
                commits: vec![],
                has_more: false,
                next_lanes: vec![],
            });
        }
        let count = skip + limit + 1;
        let mut arguments = args(&[
            "log",
            "--topo-order",
            "--date-order",
            "--encoding=UTF-8",
            "-z",
            "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x00%D",
            &format!("--max-count={count}"),
        ]);
        if let Some(reference) = &resolved_reference {
            arguments.push(reference.into());
        } else {
            // Stash snapshots belong to their own tool window, not the branch DAG.
            arguments.extend(args(&["--exclude=refs/stash", "--all"]));
        }
        if path.is_some() {
            arguments.push("--follow".into());
        }
        arguments.push("--".into());
        if let Some(path) = path {
            arguments.push(self.relative_path(path)?.into_os_string());
        }
        let output = self.run_checked(&arguments, None, cancel)?;
        let fields = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
        let fields = if fields.last() == Some(&&b""[..]) {
            &fields[..fields.len() - 1]
        } else {
            &fields[..]
        };
        if fields.len() % 7 != 0 {
            return Err(GitError::invalid("Formato log Git inesperado"));
        }
        let mut commits = Vec::new();
        let mut lanes = Vec::<String>::new();
        let mut page_end_lanes = None;
        for fields in fields.chunks_exact(7) {
            let oid = utf8(fields[0])?;
            if !valid_oid(&oid) {
                return Err(GitError::invalid("OID no histórico inválido"));
            }
            let parents = utf8(fields[1])?
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if parents.iter().any(|id| !valid_oid(id)) {
                return Err(GitError::invalid("Parent OID inválido"));
            }
            let (lane, edges) = graph_row(&mut lanes, &oid, &parents)?;
            let timestamp = utf8(fields[4])?
                .parse()
                .map_err(|_| GitError::invalid("Timestamp Git inválido"))?;
            let refs = utf8(fields[6])?
                .split(", ")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            commits.push(CommitEntry {
                oid,
                parents,
                author: String::from_utf8_lossy(fields[2]).into_owned(),
                email: String::from_utf8_lossy(fields[3]).into_owned(),
                timestamp,
                subject: String::from_utf8_lossy(fields[5]).into_owned(),
                refs,
                lane,
                edges,
            });
            if commits.len() == skip + limit {
                page_end_lanes = Some(lanes.clone());
            }
        }
        let has_more = commits.len() > skip + limit;
        let commits = commits.into_iter().skip(skip).take(limit).collect();
        Ok(HistoryPage {
            resolved_reference,
            skip,
            commits,
            has_more,
            next_lanes: page_end_lanes.unwrap_or(lanes),
        })
    }
    pub fn branches(&self, cancel: &AtomicBool) -> Result<Vec<BranchInfo>, GitError> {
        let output=self.run_checked(&args(&["for-each-ref","--count=2049","--sort=refname","--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track)%00%(worktreepath)%00","refs/heads","refs/remotes"]),None,cancel)?;
        let fields = output.stdout.split(|b| *b == 0).collect::<Vec<_>>();
        let mut branches = Vec::new();
        let mut offset = 0;
        while offset + 6 <= fields.len() {
            let name = utf8(fields[offset].strip_prefix(b"\n").unwrap_or(fields[offset]))?;
            if name.is_empty() {
                break;
            }
            if branches.len() >= 2048 {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Mais de2048 branches; filtre via Git CLI",
                ));
            }
            let remote = name.starts_with("refs/remotes/");
            let name = name
                .strip_prefix(if remote {
                    "refs/remotes/"
                } else {
                    "refs/heads/"
                })
                .ok_or_else(|| GitError::invalid("Nome branch inesperado"))?
                .to_owned();
            branches.push(BranchInfo {
                name,
                oid: utf8(fields[offset + 1])?,
                current: fields[offset + 2] == b"*",
                remote,
                upstream: (!fields[offset + 3].is_empty())
                    .then(|| utf8(fields[offset + 3]))
                    .transpose()?,
                tracking: utf8(fields[offset + 4])?,
                worktree: (!fields[offset + 5].is_empty())
                    .then(|| path_from_bytes(fields[offset + 5]))
                    .transpose()?,
            });
            offset += 6;
        }
        Ok(branches)
    }
    pub fn tags(&self, cancel: &AtomicBool) -> Result<Vec<TagInfo>, GitError> {
        let output=self.run_checked(&args(&["for-each-ref","--count=2049","--sort=-creatordate","--format=%(refname)%00%(objectname)%00%(*objectname)%00%(objecttype)%00%(subject)%00","refs/tags"]),None,cancel)?;
        let fields = output.stdout.split(|b| *b == 0).collect::<Vec<_>>();
        let mut tags = Vec::new();
        for fields in fields.chunks_exact(5) {
            if tags.len() >= 2048 {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Mais de2048 tags; consulta limitada",
                ));
            }
            let name = utf8(fields[0].strip_prefix(b"\n").unwrap_or(fields[0]))?;
            let name = name
                .strip_prefix("refs/tags/")
                .ok_or_else(|| GitError::invalid("Nome tag inesperado"))?
                .to_owned();
            let oid = utf8(fields[1])?;
            tags.push(TagInfo {
                name,
                target: if fields[2].is_empty() {
                    oid.clone()
                } else {
                    utf8(fields[2])?
                },
                oid,
                annotated: fields[3] == b"tag",
                message: String::from_utf8_lossy(fields[4]).into_owned(),
            });
        }
        Ok(tags)
    }
    pub fn file_at(
        &self,
        revision: &str,
        path: &Path,
        cancel: &AtomicBool,
    ) -> Result<FileVersion, GitError> {
        let commit = self.resolve(revision, cancel)?;
        let path = self.relative_path(path)?;
        let mut spec = std::ffi::OsString::from(format!("{commit}:"));
        spec.push(path.as_os_str());
        let output = self.run_checked(
            &[
                "show".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                spec,
            ],
            None,
            cancel,
        )?;
        Ok(FileVersion {
            commit,
            path,
            bytes: output.stdout,
        })
    }
    pub fn commit_details(
        &self,
        revision: &str,
        cancel: &AtomicBool,
    ) -> Result<CommitDetails, GitError> {
        let commit = self.resolve(revision, cancel)?;
        let parents = self
            .run_checked(
                &args(&["show", "--no-patch", "--format=%P", &commit]),
                None,
                cancel,
            )?
            .stdout;
        let parents = utf8(trim_line(&parents))?
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut names = if let Some(parent) = parents.first() {
            args(&["diff", "--name-status", "-z", "-M", parent, &commit])
        } else {
            args(&[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "-r",
                "--name-status",
                "-z",
                "-M",
                &commit,
            ])
        };
        names.push("--".into());
        let raw = self.run_checked(&names, None, cancel)?.stdout;
        let mut fields = raw.split(|b| *b == 0);
        let mut changes = Vec::new();
        while let Some(status) = fields.next() {
            if status.is_empty() {
                continue;
            }
            let status = char::from(status[0]);
            let first = fields
                .next()
                .ok_or_else(|| GitError::invalid("Diff-tree sem path"))?;
            let (path, original_path) = if matches!(status, 'R' | 'C') {
                (
                    path_from_bytes(
                        fields
                            .next()
                            .ok_or_else(|| GitError::invalid("Rename sem destino"))?,
                    )?,
                    Some(path_from_bytes(first)?),
                )
            } else {
                (path_from_bytes(first)?, None)
            };
            if changes.len() >= 10000 {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Commit altera mais de10000 caminhos",
                ));
            }
            changes.push(CommitChange {
                path,
                original_path,
                status,
            });
        }
        let patch = if let Some(parent) = parents.first() {
            self.run_checked(
                &args(&[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--binary",
                    "--full-index",
                    parent,
                    &commit,
                    "--",
                ]),
                None,
                cancel,
            )?
            .stdout
        } else {
            self.run_checked(
                &args(&[
                    "show",
                    "--format=",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--binary",
                    "--full-index",
                    &commit,
                    "--",
                ]),
                None,
                cancel,
            )?
            .stdout
        };
        let binary = patch.windows(16).any(|w| w == b"GIT binary patch");
        let message = self
            .run_checked(
                &args(&["show", "--no-patch", "--format=%B", &commit]),
                None,
                cancel,
            )?
            .stdout;
        let message = String::from_utf8_lossy(trim_line(&message)).into_owned();
        Ok(CommitDetails {
            commit,
            message,
            parents,
            changes,
            patch,
            binary,
        })
    }
    pub fn blame(
        &self,
        path: &Path,
        start: usize,
        lines: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<BlameLine>, GitError> {
        if start == 0 || lines == 0 || lines > 500 {
            return Err(GitError::invalid(
                "Blame requer linha inicial>=1 e no máximo500 linhas",
            ));
        }
        let path = self.relative_path(path)?;
        let output = self.run_checked(
            &[
                "blame".into(),
                "--line-porcelain".into(),
                "--root".into(),
                "--encoding=UTF-8".into(),
                "-L".into(),
                format!("{start},+{lines}").into(),
                "--".into(),
                path.clone().into_os_string(),
            ],
            None,
            cancel,
        )?;
        let mut result = Vec::new();
        let (mut oid, mut author, mut email, mut filename) =
            (String::new(), String::new(), String::new(), path);
        let (mut original_line, mut line, mut timestamp) = (0usize, 0usize, 0i64);
        for bytes in output.stdout.split(|b| *b == b'\n') {
            if bytes.first() == Some(&b'\t') {
                result.push(BlameLine {
                    line,
                    original_line,
                    oid: oid.clone(),
                    author: author.clone(),
                    email: email.clone(),
                    timestamp,
                    path: filename.clone(),
                    text: bytes[1..].to_vec(),
                });
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            let mut parts = text.split_whitespace();
            if let Some(candidate) = parts.next()
                && valid_oid(candidate)
            {
                oid = candidate.into();
                original_line = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| GitError::invalid("Blame original line inválida"))?;
                line = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| GitError::invalid("Blame line inválida"))?;
            } else if let Some(value) = text.strip_prefix("author ") {
                author = value.into();
            } else if let Some(value) = text.strip_prefix("author-mail ") {
                email = value.trim_matches(['<', '>']).into();
            } else if let Some(value) = text.strip_prefix("author-time ") {
                timestamp = value
                    .parse()
                    .map_err(|_| GitError::invalid("Blame timestamp inválido"))?;
            } else if bytes.starts_with(b"filename ") {
                filename = path_from_bytes(&unquote(&bytes[9..])?)?;
            }
        }
        Ok(result)
    }
}
fn graph_row(
    lanes: &mut Vec<String>,
    oid: &str,
    parents: &[String],
) -> Result<(usize, Vec<(usize, usize)>), GitError> {
    let lane = if let Some(index) = lanes.iter().position(|id| id == oid) {
        index
    } else {
        lanes.push(oid.into());
        lanes.len() - 1
    };
    let before = lanes.clone();
    lanes.remove(lane);
    for (index, parent) in parents.iter().enumerate() {
        if !lanes.contains(parent) {
            lanes.insert((lane + index).min(lanes.len()), parent.clone());
        }
    }
    if lanes.len() > 128 {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Grafo excede128 trilhas simultâneas",
        ));
    }
    let mut edges = before
        .iter()
        .enumerate()
        .filter_map(|(from, id)| {
            lanes
                .iter()
                .position(|next| next == id)
                .map(|to| (from, to))
        })
        .collect::<Vec<_>>();
    for parent in parents {
        if let Some(to) = lanes.iter().position(|next| next == parent) {
            edges.push((lane, to));
        }
    }
    Ok((lane, edges))
}
fn unquote(bytes: &[u8]) -> Result<Vec<u8>, GitError> {
    if !bytes.starts_with(b"\"") {
        return Ok(bytes.to_vec());
    }
    if !bytes.ends_with(b"\"") {
        return Err(GitError::invalid("Caminho Git quoted inválido"));
    }
    let mut result = Vec::new();
    let mut i = 1;
    while i + 1 < bytes.len() {
        if bytes[i] != b'\\' {
            result.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() - 1 {
            return Err(GitError::invalid("Escape Git incompleto"));
        }
        let value = match bytes[i] {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            b'"' => b'"',
            b'\\' => b'\\',
            b'0'..=b'7' => {
                if i + 2 >= bytes.len() - 1
                    || !bytes[i..i + 3].iter().all(|b| matches!(b, b'0'..=b'7'))
                {
                    return Err(GitError::invalid("Escape octal Git inválido"));
                }
                let value = u16::from(bytes[i] - b'0') * 64
                    + u16::from(bytes[i + 1] - b'0') * 8
                    + u16::from(bytes[i + 2] - b'0');
                i += 2;
                u8::try_from(value)
                    .map_err(|_| GitError::invalid("Escape octal Git fora de faixa"))?
            }
            _ => return Err(GitError::invalid("Escape Git desconhecido")),
        };
        result.push(value);
        i += 1;
    }
    Ok(result)
}

impl Repository {
    pub fn stash_details(
        &self,
        reference: &str,
        cancel: &AtomicBool,
    ) -> Result<crate::StashDetails, GitError> {
        let entry = crate::workflows::list_stashes(self, cancel)?
            .into_iter()
            .find(|s| s.reference == reference)
            .ok_or_else(|| GitError::invalid("Stash não existe mais; atualize a lista"))?;
        let mut detail = self.commit_details(&entry.oid, cancel)?;
        if let Some(untracked) = detail.parents.get(2) {
            let extra = self.commit_details(untracked, cancel)?;
            if detail.patch.len() + extra.patch.len() > 8 * 1024 * 1024
                || detail.changes.len() + extra.changes.len() > 10000
            {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Detalhes do stash excedem o orçamento",
                ));
            }
            detail.patch.extend(extra.patch);
            detail.changes.extend(extra.changes);
        }
        Ok(crate::StashDetails {
            reference: entry.reference,
            oid: entry.oid,
            changes: detail.changes,
            patch: detail.patch,
        })
    }
}

impl Repository {
    pub fn revision_diff(
        &self,
        base: Option<&str>,
        head: &str,
        path: Option<&Path>,
        cancel: &AtomicBool,
    ) -> Result<crate::RevisionDiff, GitError> {
        let query = crate::RevisionDiffQuery {
            base: base.map(str::to_owned),
            head: head.to_owned(),
            path: path.map(Path::to_path_buf),
        };
        let head = self.resolve(head, cancel)?;
        let base = base.map(|b| self.resolve(b, cancel)).transpose()?;
        let empty = if base.is_none() {
            let raw = self
                .run_checked(
                    &args(&["hash-object", "-t", "tree", "--stdin"]),
                    Some(b""),
                    cancel,
                )?
                .stdout;
            String::from_utf8_lossy(trim_line(&raw)).into_owned()
        } else {
            String::new()
        };
        let path = path.map(|p| self.relative_path(p)).transpose()?;
        let mut a = args(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--binary",
            "--full-index",
            base.as_deref().unwrap_or(&empty),
            &head,
            "--",
        ]);
        if let Some(path) = &path {
            a.push(path.as_os_str().into());
        }
        let bytes = self.run_checked(&a, None, cancel)?.stdout;
        Ok(crate::RevisionDiff {
            query,
            base,
            head,
            path,
            bytes,
        })
    }
}
