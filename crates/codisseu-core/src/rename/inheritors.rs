//! Optional PHP class/file choices and sequential semantic proof of the batch.
use super::*;
use codisseu_syntax::{RenameEdit, RenameSnapshot, ValidationLimits};
use std::path::Path;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenameInheritorKind {
    Class,
    File,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameInheritorChoice {
    pub kind: RenameInheritorKind,
    pub path: PathBuf,
    pub range: Option<TextRange>,
    pub display_name: String,
    pub old_name: String,
    pub new_name: String,
    pub selected: bool,
}
#[derive(Clone, Default)]
pub(crate) struct Choices {
    pub rows: Vec<RenameInheritorChoice>,
    pub confirmed: bool,
}
pub(crate) struct Plan {
    pub snapshots: Vec<RenameSnapshot>,
    pub resources: Vec<codisseu_syntax::RenameResourceProposal>,
    pub reports: Vec<RenameValidation>,
    pub issues: Vec<ValidationIssue>,
    pub choices: Arc<Choices>,
}
fn file_row(
    proposal: codisseu_syntax::RenameResourceProposal,
) -> Result<RenameInheritorChoice, String> {
    let old_name = proposal
        .source
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Arquivo sem nome UTF-8")?
        .to_owned();
    let new_name = proposal
        .target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Destino sem nome UTF-8")?
        .to_owned();
    Ok(RenameInheritorChoice {
        kind: RenameInheritorKind::File,
        path: proposal.source,
        range: None,
        display_name: old_name.clone(),
        old_name,
        new_name,
        selected: true,
    })
}
pub(crate) fn valid(row: &RenameInheritorChoice) -> bool {
    if !row.selected {
        return true;
    }
    if row.new_name.is_empty() || row.new_name.len() > 256 {
        return false;
    }
    match row.kind {
        RenameInheritorKind::Class => {
            codisseu_syntax::validate_rename_candidate(&row.new_name).is_ok()
        }
        RenameInheritorKind::File => {
            !row.new_name.contains(['/', '\\'])
                && Path::new(&row.new_name).extension() == row.path.extension()
                && Path::new(&row.new_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| codisseu_syntax::validate_rename_candidate(s).is_ok())
        }
    }
}
fn offset(text: &str, line: u32, col: u32) -> Result<usize, String> {
    codisseu_language::utf16_byte_offset(text, line, col)
        .ok_or_else(|| "Range UTF-16 de composição inválido".into())
}
fn range_bytes(text: &str, range: &TextRange) -> Result<std::ops::Range<usize>, String> {
    Ok(offset(text, range.start_line, range.start_character)?
        ..offset(text, range.end_line, range.end_character)?)
}
fn text_range(text: &str, range: std::ops::Range<usize>) -> TextRange {
    let point = |byte| {
        let prefix = &text[..byte];
        (
            prefix.bytes().filter(|b| *b == b'\n').count() as u32,
            prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
        )
    };
    let (a, b) = point(range.start);
    let (c, d) = point(range.end);
    TextRange {
        start_line: a,
        start_character: b,
        end_line: c,
        end_character: d,
    }
}
fn edits_bytes(snapshot: &RenameSnapshot) -> Result<Vec<(std::ops::Range<usize>, usize)>, String> {
    let mut edits = snapshot
        .edits
        .iter()
        .map(|e| Ok((range_bytes(&snapshot.before, &e.range)?, e.new_text.len())))
        .collect::<Result<Vec<_>, String>>()?;
    edits.sort_by_key(|e| e.0.start);
    Ok(edits)
}
fn forward(snapshot: &RenameSnapshot, range: &TextRange) -> Result<TextRange, String> {
    let range = range_bytes(&snapshot.before, range)?;
    let mut delta = 0isize;
    for (old, len) in edits_bytes(snapshot)? {
        if old.end <= range.start {
            delta += len as isize - old.len() as isize;
        } else if old.start < range.end {
            return Err("Origem opcional foi modificada por outro rename".into());
        }
    }
    Ok(text_range(
        &snapshot.after,
        (range.start as isize + delta) as usize..(range.end as isize + delta) as usize,
    ))
}
fn inverse(
    snapshot: &RenameSnapshot,
    range: &TextRange,
    strict: bool,
) -> Result<TextRange, String> {
    let range = range_bytes(&snapshot.after, range)?;
    let edits = edits_bytes(snapshot)?;
    let convert = |point: usize, end: bool| -> Result<usize, String> {
        let mut delta = 0isize;
        for (old, len) in &edits {
            let start = (old.start as isize + delta) as usize;
            let finish = start + len;
            if point < start || (!end && point == start) {
                break;
            }
            if point >= finish {
                delta += *len as isize - old.len() as isize;
                continue;
            }
            if strict {
                return Err("Subrename altera token já transformado".into());
            }
            return Ok(if end { old.end } else { old.start });
        }
        usize::try_from(point as isize - delta).map_err(|_| "Mapeamento inverso inválido".into())
    };
    Ok(text_range(
        &snapshot.before,
        convert(range.start, false)?..convert(range.end, true)?,
    ))
}
fn compose(combined: &mut [RenameSnapshot], next: &[RenameSnapshot]) -> Result<(), String> {
    for step in next {
        let base = combined
            .iter_mut()
            .find(|s| s.path == step.path)
            .ok_or("Subrename fora do corpus")?;
        if base.after != step.before {
            return Err("Subrename não usa o estado sequencial anterior".into());
        }
        if step.edits.is_empty() {
            if step.after != step.before {
                return Err("Snapshot sem edições mudou durante a composição".into());
            }
            // Context files remain in the proof. No edit builder is needed when
            // this sequential step leaves their already-validated bytes intact.
            continue;
        }
        let additions = step
            .edits
            .iter()
            .map(|e| {
                Ok(RenameEdit {
                    range: inverse(base, &e.range, true)?,
                    new_text: e.new_text.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        base.edits.extend(additions);
        base.edits
            .sort_by_key(|e| (e.range.start_line, e.range.start_character));
        let edits = base
            .edits
            .iter()
            .map(|e| RenameTextEdit {
                start_line: e.range.start_line,
                start_character: e.range.start_character,
                end_line: e.range.end_line,
                end_character: e.range.end_character,
                new_text: e.new_text.clone(),
            })
            .collect::<Vec<_>>();
        let after: Arc<str> = Arc::from(super::build_after(&base.before, &edits)?);
        if after != step.after {
            return Err("Prova sequencial diverge da composição original".into());
        }
        base.after = after;
    }
    Ok(())
}
fn original_report(report: &mut RenameValidation, combined: &[RenameSnapshot]) {
    for conflict in &mut report.conflicts {
        if let Some(s) = combined.iter().find(|s| s.path == conflict.path)
            && let Ok(range) = inverse(s, &conflict.range, false)
        {
            conflict.range = range;
        }
        if let Some(related) = &mut conflict.related
            && let Some(s) = combined.iter().find(|s| s.path == related.path)
            && let Ok(range) = inverse(s, &related.range, false)
        {
            related.range = range;
        }
    }
    for issue in &mut report.issues {
        if let Some(range) = &issue.range
            && let Some(s) = combined.iter().find(|s| s.path == issue.path)
            && let Ok(range) = inverse(s, range, false)
        {
            issue.range = Some(range);
        }
    }
}
pub(crate) fn plan(
    primary: &[RenameSnapshot],
    new_name: &str,
    choices: &Arc<Choices>,
    cancel: &AtomicBool,
) -> Result<Plan, String> {
    let limits = ValidationLimits::default();
    let descendants =
        codisseu_syntax::plan_class_inheritor_renames(primary, new_name, cancel, limits)?;
    let mut rows = vec![];
    if let Some(resource) =
        codisseu_syntax::class_file_rename_candidate(primary, new_name, cancel, limits)?
    {
        rows.push(file_row(resource)?);
    }
    for child in descendants {
        rows.push(RenameInheritorChoice {
            kind: RenameInheritorKind::Class,
            path: child.path.clone(),
            range: Some(child.range),
            display_name: child.display_name,
            old_name: child.old_name,
            new_name: child.new_name.clone(),
            selected: true,
        });
        if let Some(resource) = codisseu_syntax::class_file_rename_candidate(
            &child.snapshots,
            &child.new_name,
            cancel,
            limits,
        )? {
            rows.push(file_row(resource)?);
        }
    }
    rows.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then((a.kind as u8).cmp(&(b.kind as u8)))
    });
    if !choices.rows.is_empty() {
        if rows.len() != choices.rows.len()
            || rows.iter().zip(&choices.rows).any(|(a, b)| {
                a.path != b.path
                    || a.kind != b.kind
                    || a.range != b.range
                    || a.old_name != b.old_name
            })
        {
            return Err("Classes/arquivos relacionados mudaram; refaça Rename".into());
        }
        rows = choices.rows.clone();
    }
    if rows.iter().any(|r| !valid(r)) {
        return Err("Corrija os nomes relacionados antes de confirmar".into());
    }
    let mut combined = primary.to_vec();
    let mut reports = vec![];
    let mut issues = vec![];
    let mut resources = vec![];
    for row in rows
        .iter()
        .filter(|r| r.selected && r.kind == RenameInheritorKind::Class && r.new_name != r.old_name)
    {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Rename cancelado".into());
        }
        let current = combined
            .iter()
            .map(|s| RenameSnapshot {
                path: s.path.clone(),
                before: s.after.clone(),
                after: s.after.clone(),
                edits: vec![],
            })
            .collect::<Vec<_>>();
        let source = combined
            .iter()
            .find(|s| s.path == row.path)
            .ok_or("Origem de herdeiro ausente")?;
        let origin = forward(source, row.range.as_ref().ok_or("Classe sem range")?)?;
        let proposed = codisseu_syntax::propose_global_rename_with_limits(
            &current,
            &row.path,
            &origin,
            &row.new_name,
            cancel,
            limits,
        )?
        .ok_or("Herdeiro deixou de ser classe")?;
        let mut report =
            codisseu_syntax::validate_rename_with_limits(&proposed, &row.new_name, cancel, limits);
        original_report(&mut report, &combined);
        for conflict in &mut report.conflicts {
            conflict.message = format!(
                "Herdeiro {} → {}: {}",
                row.old_name, row.new_name, conflict.message
            );
        }
        for issue in &mut report.issues {
            issue.message = format!(
                "Herdeiro {} → {}: {}",
                row.old_name, row.new_name, issue.message
            );
        }
        if report.coverage == Coverage::Verified {
            compose(&mut combined, &proposed)?;
        }
        reports.push(report);
    }
    // File paths are independently selected. AST module edits remain original
    // coordinates and are applied only after the sequential symbol proof.
    for row in rows
        .iter()
        .filter(|r| r.selected && r.kind == RenameInheritorKind::File && r.new_name != r.old_name)
    {
        match codisseu_syntax::plan_named_file_rename(
            primary,
            &row.path,
            &row.path.with_file_name(&row.new_name),
            cancel,
            limits,
        ) {
            Ok(Some(proposal)) => resources.push(proposal),
            Ok(None) => {}
            Err(message) => issues.push(ValidationIssue {
                path: row.path.clone(),
                range: None,
                kind: IssueKind::UnsupportedEdit,
                message: format!("Arquivo relacionado: {message}"),
            }),
        }
    }
    Ok(Plan {
        snapshots: combined,
        resources,
        reports,
        issues,
        choices: Arc::new(Choices {
            rows,
            confirmed: choices.confirmed,
        }),
    })
}
impl RenameState {
    pub fn choosing_inheritors(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::ChoosingInheritors)
    }
    pub fn inheritor_choices(&self) -> &[RenameInheritorChoice] {
        self.context
            .as_ref()
            .map_or(&[], |c| &c.search.inheritors.rows)
    }
    pub fn can_confirm_inheritors(&self) -> bool {
        self.choosing_inheritors() && self.inheritor_choices().iter().all(valid)
    }
}
impl AppService {
    pub(crate) fn edit_rename_inheritors(&mut self, token: u64) -> Result<(), String> {
        if !self.rename_current() || self.rename.context_token() != Some(token) {
            return Err("Seleção de herdeiros obsoleta".into());
        }
        let c = self.rename.context.as_mut().ok_or("Rename ausente")?;
        if c.phase != Phase::Preview || c.search.inheritors.rows.is_empty() {
            return Err("Não há seleção de herdeiros para editar".into());
        }
        Arc::make_mut(&mut c.search.inheritors).confirmed = false;
        c.phase = Phase::ChoosingInheritors;
        Ok(())
    }

    pub(crate) fn update_rename_inheritor(
        &mut self,
        token: u64,
        index: usize,
        selected: bool,
        new_name: String,
    ) -> Result<(), String> {
        if !self.rename_current()
            || self.rename.context_token() != Some(token)
            || !self.rename.choosing_inheritors()
        {
            return Err("Seleção de herdeiros obsoleta".into());
        }
        if new_name.len() > 256 {
            return Err("Nome relacionado excede 256 bytes".into());
        }
        let c = self.rename.context.as_mut().unwrap();
        let state = Arc::make_mut(&mut c.search.inheritors);
        let row = state
            .rows
            .get_mut(index)
            .ok_or("Item relacionado inválido")?;
        row.selected = selected;
        row.new_name = new_name;
        Ok(())
    }
    pub(crate) fn confirm_rename_inheritors(&mut self, token: u64) -> Result<(), String> {
        if !self.rename_current()
            || self.rename.context_token() != Some(token)
            || !self.rename.can_confirm_inheritors()
        {
            return Err("Seleção de herdeiros inválida/obsoleta".into());
        }
        let c = self.rename.context.as_mut().unwrap();
        Arc::make_mut(&mut c.search.inheritors).confirmed = true;
        let prepared = PreparedRename {
            search: c.search.clone(),
            files: c.proposal.clone(),
            snapshots: c.snapshots.clone(),
            validation: c.validation.clone().ok_or("Preview ausente")?,
            corpus: c.corpus.clone(),
            resources: c.resources.clone(),
        };
        let work = Work::RenameChoices {
            token,
            workspace: self.workspace.clone().ok_or("Workspace ausente")?,
            prepared: Box::new(prepared),
            new_name: self.rename.new_name.clone(),
            cancel: c.cancel.clone(),
        };
        c.phase = Phase::Building;
        self.enqueue(work, Pending::Rename(token))?;
        Ok(())
    }
}

#[cfg(test)]
mod composition_tests {
    use super::*;
    #[test]
    fn unchanged_context_is_preserved_but_unlisted_byte_changes_are_rejected() {
        let original = RenameSnapshot {
            path: "/w/Other.php".into(),
            before: Arc::from("<?php namespace Other; class Weather {}"),
            after: Arc::from("<?php namespace Other; class Weather {}"),
            edits: vec![],
        };
        let mut combined = vec![original.clone()];
        compose(&mut combined, std::slice::from_ref(&original)).unwrap();
        assert_eq!(combined[0].before, combined[0].after);
        assert!(combined[0].edits.is_empty());
        let mut forged = original;
        forged.after = Arc::from("<?php namespace Other; class Forged {}");
        assert!(compose(&mut combined, &[forged]).is_err());
        assert_eq!(combined[0].before, combined[0].after);
        assert!(combined[0].edits.is_empty());
    }
}
