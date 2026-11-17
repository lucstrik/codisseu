//! Snapshot-bound outgoing inspection and explicit non-forced refspec push.
use crate::{
    repository::{args, checked, path_from_bytes, valid_oid},
    workflows, *,
};
use std::{collections::BTreeMap, ffi::OsString, sync::atomic::AtomicBool, time::Duration};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PushTagsMode {
    #[default]
    None,
    CurrentBranch,
    All,
}
#[derive(Clone, Debug)]
pub struct PushReview {
    pub remote: String,
    pub source: String,
    pub target: String,
    pub local_oid: String,
    pub remote_oid: Option<String>,
    pub diff_base: Option<String>,
    pub commits: Vec<CommitEntry>,
    pub changes: Vec<CommitChange>,
    pub tags: Vec<TagInfo>,
    pub all_tags: Vec<TagInfo>,
    pub warnings: Vec<String>,
    pub can_push: bool,
}
#[derive(Clone, Debug)]
pub struct PushInspection {
    review: PushReview,
    snapshot: RepoSnapshot,
    url: String,
    urls: Vec<u8>,
    remote_tags: Vec<u8>,
    tag_conflicts: bool,
    all_tag_conflicts: bool,
    heads: Option<Vec<u8>>,
}
impl PushInspection {
    pub fn review(&self) -> &PushReview {
        &self.review
    }
}
#[derive(Clone, Debug)]
pub struct PreparedPush {
    inspection: PushInspection,
    mode: PushTagsMode,
}
fn branch(repo: &Repository, name: &str, cancel: &AtomicBool) -> Result<(), GitError> {
    if name.is_empty()
        || name.starts_with('-')
        || name.len() > 1024
        || name.contains(['\0', '\n', '\r'])
    {
        return Err(GitError::invalid("Nome de branch inválido"));
    }
    repo.run_checked(
        &args(&["check-ref-format", &format!("refs/heads/{name}")]),
        None,
        cancel,
    )?;
    Ok(())
}
fn remote_head(
    repo: &Repository,
    url: &str,
    target: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>, GitError> {
    let raw = checked(repo.run_timeout(
        &args(&[
            "ls-remote",
            "--heads",
            "--",
            url,
            &format!("refs/heads/{target}"),
        ]),
        None,
        cancel,
        Duration::from_secs(120),
    )?)?
    .stdout;
    let mut result = None;
    for line in raw.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        let text = std::str::from_utf8(line)
            .map_err(|_| GitError::invalid("Referência remota inválida"))?;
        let (oid, name) = text
            .split_once('\t')
            .ok_or_else(|| GitError::invalid("Resposta ls-remote inválida"))?;
        if name != format!("refs/heads/{target}") || !valid_oid(oid) || result.is_some() {
            return Err(GitError::invalid("Destino remoto ambíguo"));
        }
        result = Some(oid.into());
    }
    Ok(result)
}
fn remote_heads(repo: &Repository, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>, GitError> {
    Ok(checked(repo.run_timeout(
        &args(&["ls-remote", "--heads", "--", url]),
        None,
        cancel,
        Duration::from_secs(120),
    )?)?
    .stdout)
}
fn new_target_base(
    repo: &Repository,
    local: &str,
    heads: &[u8],
    cancel: &AtomicBool,
) -> Result<(Vec<String>, Option<String>), GitError> {
    let mut candidates = std::collections::BTreeSet::new();
    for line in heads.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let text = std::str::from_utf8(line)
            .map_err(|_| GitError::invalid("Referências remotas inválidas"))?;
        let (oid, name) = text
            .split_once('\t')
            .ok_or_else(|| GitError::invalid("Anúncio remoto inválido"))?;
        if !valid_oid(oid) || !name.starts_with("refs/heads/") {
            return Err(GitError::invalid("Anúncio remoto inválido"));
        }
        candidates.insert(oid.to_owned());
        if candidates.len() > 10000 {
            return Err(GitError::new(
                GitErrorKind::OutputLimit,
                "Mais de10000heads remotos",
            ));
        }
    }
    if candidates.is_empty() {
        return Ok((vec![], None));
    }
    let input = candidates
        .iter()
        .map(|o| format!("{o}\n"))
        .collect::<String>();
    let raw = repo
        .run_checked(
            &args(&["cat-file", "--batch-check=%(objectname) %(objecttype)"]),
            Some(input.as_bytes()),
            cancel,
        )?
        .stdout;
    let known = String::from_utf8_lossy(&raw)
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(o, t)| *t == "commit" && valid_oid(o))
        .map(|(o, _)| o.to_owned())
        .collect::<Vec<_>>();
    if known.is_empty() {
        return Err(GitError::invalid(
            "Histórico remoto desconhecido localmente. Execute Fetch antes de revisar a nova branch.",
        ));
    }
    let mut a = args(&["merge-base", local]);
    a.extend(known.iter().map(OsString::from));
    let result = repo.run(&a, None, cancel)?;
    let base = if result.status.success() {
        let oid = String::from_utf8_lossy(&result.stdout).trim().to_owned();
        if !valid_oid(&oid) {
            return Err(GitError::invalid("Ancestral remoto inválido"));
        }
        Some(oid)
    } else if result.status.code() == Some(1) {
        None
    } else {
        return Err(GitError::new(
            GitErrorKind::Git,
            crate::redact(&result.stderr),
        ));
    };
    Ok((known, base))
}
fn remote_tags(repo: &Repository, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>, GitError> {
    Ok(checked(repo.run_timeout(
        &args(&["ls-remote", "--refs", "--tags", "--", url]),
        None,
        cancel,
        Duration::from_secs(120),
    )?)?
    .stdout)
}
pub(crate) fn names(raw: &[u8]) -> Result<Vec<CommitChange>, GitError> {
    let mut fields = raw.split(|b| *b == 0);
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        if changes.len() >= 10000 {
            return Err(GitError::new(
                GitErrorKind::OutputLimit,
                "Mais de10000arquivos de saída",
            ));
        }
        let first = fields
            .next()
            .ok_or_else(|| GitError::invalid("Arquivo de saída ausente"))?;
        let kind = char::from(status[0]);
        let (path, original_path) = if matches!(kind, 'R' | 'C') {
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
        changes.push(CommitChange {
            path,
            original_path,
            status: kind,
        });
    }
    Ok(changes)
}
fn metadata(
    repo: &Repository,
    ids: &[String],
    cancel: &AtomicBool,
) -> Result<Vec<CommitEntry>, GitError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut a = args(&[
        "log",
        "--no-walk=unsorted",
        "-z",
        "--encoding=UTF-8",
        "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x00%D",
    ]);
    a.extend(ids.iter().map(OsString::from));
    a.push("--".into());
    let raw = repo.run_checked(&a, None, cancel)?.stdout;
    let mut fields = raw.split(|b| *b == 0).collect::<Vec<_>>();
    if fields.last() == Some(&&b""[..]) {
        fields.pop();
    }
    if fields.len() % 7 != 0 {
        return Err(GitError::invalid("Metadados de saída inválidos"));
    }
    fields
        .chunks_exact(7)
        .map(|f| {
            let text = |i| String::from_utf8_lossy(f[i]).into_owned();
            let oid = text(0);
            if !valid_oid(&oid) {
                return Err(GitError::invalid("Commit de saída inválido"));
            }
            Ok(CommitEntry {
                oid,
                parents: text(1).split_whitespace().map(str::to_owned).collect(),
                author: text(2),
                email: text(3),
                timestamp: text(4)
                    .parse()
                    .map_err(|_| GitError::invalid("Timestamp inválido"))?,
                subject: text(5),
                refs: text(6)
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                lane: 0,
                edges: vec![],
            })
        })
        .collect()
}
pub fn inspect_push(
    repo: &Repository,
    remote: &str,
    source: &str,
    target: &str,
    cancel: &AtomicBool,
) -> Result<PushInspection, GitError> {
    branch(repo, source, cancel)?;
    branch(repo, target, cancel)?;
    let snapshot = repo.snapshot(cancel)?;
    let local_oid = repo.resolve(&format!("refs/heads/{source}"), cancel)?;
    let urls = workflows::remote_urls(repo, remote, true, cancel)?;
    let candidates = urls
        .split(|b| *b == b'\n')
        .filter(|r| !r.is_empty())
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(GitError::unsupported(
            "Revisão de Push exige um único endereço de envio",
        ));
    }
    let url = std::str::from_utf8(candidates[0])
        .map_err(|_| GitError::invalid("Endereço remoto sem UTF-8"))?
        .to_owned();
    if url.starts_with('-') || url.starts_with("ext::") {
        return Err(GitError::invalid("Transporte remoto não permitido"));
    }
    let remote_oid = remote_head(repo, &url, target, cancel)?;
    if let Some(oid) = &remote_oid {
        let exists = repo.run(
            &args(&["cat-file", "-e", &format!("{oid}^{{commit}}")]),
            None,
            cancel,
        )?;
        if !exists.status.success() {
            return Err(GitError::invalid(
                "O commit do destino remoto não está disponível localmente. Execute Fetch antes de revisar o Push.",
            ));
        }
    }
    let heads = if remote_oid.is_none() {
        Some(remote_heads(repo, &url, cancel)?)
    } else {
        None
    };
    let (excluded, diff_base) = if let Some(heads) = &heads {
        new_target_base(repo, &local_oid, heads, cancel)?
    } else {
        (remote_oid.iter().cloned().collect(), remote_oid.clone())
    };
    let mut list = args(&["rev-list", "--topo-order", "--max-count=201", &local_oid]);
    for oid in &excluded {
        list.push(format!("^{oid}").into());
    }
    list.push("--".into());
    let raw = repo.run_checked(&list, None, cancel)?.stdout;
    let ids = String::from_utf8_lossy(&raw)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if ids.len() > 200 {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Mais de200commits de saída; revisão limitada a200por envio",
        ));
    }
    let commits = metadata(repo, &ids, cancel)?;
    let mut diff = if let Some(oid) = &diff_base {
        args(&["diff", "--name-status", "-z", "-M", oid, &local_oid, "--"])
    } else {
        let empty = repo.run_checked(
            &args(&["hash-object", "-t", "tree", "--stdin"]),
            Some(b""),
            cancel,
        )?;
        let empty = String::from_utf8_lossy(&empty.stdout).trim().to_owned();
        args(&[
            "diff",
            "--name-status",
            "-z",
            "-M",
            &empty,
            &local_oid,
            "--",
        ])
    };
    diff.insert(1, "--no-ext-diff".into());
    diff.insert(2, "--no-textconv".into());
    let changes = names(&repo.run_checked(&diff, None, cancel)?.stdout)?;
    let can_push = if let Some(oid) = &remote_oid {
        repo.run(
            &args(&["merge-base", "--is-ancestor", oid, &local_oid]),
            None,
            cancel,
        )?
        .status
        .success()
    } else {
        true
    };
    let raw_tags = remote_tags(repo, &url, cancel)?;
    let remote_map = String::from_utf8_lossy(&raw_tags)
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(o, n)| (n.to_owned(), o.to_owned()))
        .collect::<BTreeMap<_, _>>();
    let mut tags = Vec::new();
    let mut all_tags = Vec::new();
    let mut tag_conflicts = false;
    let mut all_tag_conflicts = false;
    for tag in repo.tags(cancel)? {
        let reference = format!("refs/tags/{}", tag.name);
        let reachable = tag.annotated
            && repo.resolve(&reference, cancel).is_ok_and(|id| {
                repo.run(
                    &args(&["merge-base", "--is-ancestor", &id, &local_oid]),
                    None,
                    cancel,
                )
                .is_ok_and(|o| o.status.success())
            });
        match remote_map.get(&reference) {
            None => {
                if reachable {
                    tags.push(tag.clone());
                }
                all_tags.push(tag);
            }
            Some(oid) if oid == &tag.oid => {}
            Some(_) => {
                all_tag_conflicts = true;
                if reachable {
                    tag_conflicts = true;
                }
            }
        }
    }
    let mut warnings = vec![];
    if !can_push {
        warnings.push("O destino não é ancestral da origem. Integre os commits remotos antes de enviar; Push forçado não está disponível.".into());
    }
    if tag_conflicts {
        warnings.push("Uma tag anotada alcançável tem outro conteúdo no remoto; desmarque tags ou resolva o conflito externamente.".into());
    }
    if repo.snapshot(cancel)? != snapshot
        || remote_head(repo, &url, target, cancel)? != remote_oid
        || (heads.is_some() && remote_heads(repo, &url, cancel)? != *heads.as_ref().unwrap())
    {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Repositório ou destino remoto mudou durante a revisão",
        ));
    }
    Ok(PushInspection {
        review: PushReview {
            remote: remote.into(),
            source: source.into(),
            target: target.into(),
            local_oid,
            remote_oid,
            diff_base,
            commits,
            changes,
            tags,
            all_tags,
            warnings,
            can_push,
        },
        snapshot,
        url,
        urls,
        remote_tags: raw_tags,
        tag_conflicts,
        all_tag_conflicts,
        heads,
    })
}
pub fn prepare_push(
    repo: &Repository,
    inspection: &PushInspection,
    tags: bool,
    cancel: &AtomicBool,
) -> Result<PreparedPush, GitError> {
    prepare_push_mode(
        repo,
        inspection,
        if tags {
            PushTagsMode::CurrentBranch
        } else {
            PushTagsMode::None
        },
        cancel,
    )
}
pub fn prepare_push_mode(
    repo: &Repository,
    inspection: &PushInspection,
    mode: PushTagsMode,
    cancel: &AtomicBool,
) -> Result<PreparedPush, GitError> {
    let r = &inspection.review;
    if !r.can_push {
        return Err(GitError::invalid(
            "Push não fast-forward recusado; atualize e integre primeiro",
        ));
    }
    if (mode == PushTagsMode::CurrentBranch && inspection.tag_conflicts)
        || (mode == PushTagsMode::All && inspection.all_tag_conflicts)
    {
        return Err(GitError::invalid(
            "Tags remotas conflitantes; não serão sobrescritas",
        ));
    }
    validate(repo, inspection, mode != PushTagsMode::None, cancel)?;
    Ok(PreparedPush {
        inspection: inspection.clone(),
        mode,
    })
}
fn validate(
    repo: &Repository,
    p: &PushInspection,
    tags: bool,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    let r = &p.review;
    if let Some(heads) = &p.heads
        && remote_heads(repo, &p.url, cancel)? != *heads
    {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Branches remotas mudaram após a revisão; atualize",
        ));
    }
    if repo.snapshot(cancel)? != p.snapshot
        || workflows::remote_urls(repo, &r.remote, true, cancel)? != p.urls
        || remote_head(repo, &p.url, &r.target, cancel)? != r.remote_oid
        || (tags && remote_tags(repo, &p.url, cancel)? != p.remote_tags)
    {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "A revisão de Push mudou. Atualize antes de enviar.",
        ));
    }
    Ok(())
}
pub fn apply_push(
    repo: &Repository,
    plan: &PreparedPush,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    let p = &plan.inspection;
    validate(repo, p, plan.mode != PushTagsMode::None, cancel)?;
    let r = &p.review;
    let mut a = args(&[
        "-c",
        "push.followTags=false",
        "push",
        "--porcelain",
        "--atomic",
        "--no-follow-tags",
        "--recurse-submodules=no",
        "--",
        &p.url,
        &format!("{}:refs/heads/{}", r.local_oid, r.target),
    ]);
    if plan.mode != PushTagsMode::None {
        for tag in if plan.mode == PushTagsMode::All {
            &r.all_tags
        } else {
            &r.tags
        } {
            a.push(format!("{}:refs/tags/{}", tag.oid, tag.name).into());
        }
    }
    Ok(workflows::report(
        repo,
        repo.run_timeout(&a, None, cancel, Duration::from_secs(120)),
        cancel,
    ))
}
