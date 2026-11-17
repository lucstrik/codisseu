use super::*;
use codisseu_core::git::DiffSide;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiffRow {
    pub left: Option<(usize, String, bool)>,
    pub right: Option<(usize, String, bool)>,
    pub header: bool,
}
/// Align adjacent deletions/additions into a comparison without rewriting bytes.
pub(super) fn diff_rows(patch: &str) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let (mut old, mut new) = (1, 1);
    let mut in_hunk = false;
    let mut removed = Vec::new();
    let mut added = Vec::new();
    fn flush(
        rows: &mut Vec<DiffRow>,
        removed: &mut Vec<(usize, String, bool)>,
        added: &mut Vec<(usize, String, bool)>,
    ) {
        let mut l = removed.drain(..);
        let mut r = added.drain(..);
        loop {
            let left = l.next();
            let right = r.next();
            if left.is_none() && right.is_none() {
                break;
            }
            rows.push(DiffRow {
                left,
                right,
                header: false,
            });
        }
    }
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            flush(&mut rows, &mut removed, &mut added);
            in_hunk = false;
            continue;
        }
        if line.starts_with("@@") {
            in_hunk = true;
            flush(&mut rows, &mut removed, &mut added);
            let parts = line.split_whitespace().collect::<Vec<_>>();
            old = parts
                .get(1)
                .and_then(|p| p.trim_start_matches('-').split(',').next())
                .and_then(|p| p.parse().ok())
                .unwrap_or(1);
            new = parts
                .get(2)
                .and_then(|p| p.trim_start_matches('+').split(',').next())
                .and_then(|p| p.parse().ok())
                .unwrap_or(1);
            rows.push(DiffRow {
                left: Some((0, line.into(), false)),
                right: None,
                header: true,
            });
        } else if !in_hunk || line.starts_with("\\ No newline") {
            flush(&mut rows, &mut removed, &mut added);
        } else if let Some(text) = line.strip_prefix('-') {
            removed.push((old, text.into(), true));
            old += 1;
        } else if let Some(text) = line.strip_prefix('+') {
            added.push((new, text.into(), true));
            new += 1;
        } else if let Some(text) = line.strip_prefix(' ') {
            flush(&mut rows, &mut removed, &mut added);
            rows.push(DiffRow {
                left: Some((old, text.into(), false)),
                right: Some((new, text.into(), false)),
                header: false,
            });
            old += 1;
            new += 1;
        }
    }
    flush(&mut rows, &mut removed, &mut added);
    rows
}
pub(super) struct DiffData {
    revision: u64,
    key: String,
    pub text: String,
    meta: Option<DiffMeta>,
    pub content: super::git_diff_view::DiffContent,
}
impl DiffData {
    pub(super) fn comparison(text: String, path: &std::path::Path) -> Self {
        let mut data = Self::inspection(text, path);
        data.content.hide_patch_headers();
        data
    }

    pub(super) fn inspection(text: String, path: &std::path::Path) -> Self {
        Self {
            revision: 0,
            key: String::new(),
            content: super::git_diff_view::DiffContent::new(&text, Some(path), false),
            text,
            meta: None,
        }
    }
}
struct DiffMeta {
    path: PathBuf,
    side: DiffSide,
    binary: bool,
    hunks: Vec<String>,
    bytes: Vec<u8>,
}
impl CodisseuApp {
    pub(super) fn git_diff_window(&mut self, ctx: &egui::Context) {
        if !self.git_ui.diff_open || self.git_ui.dialog.is_some() {
            return;
        }
        let size = ctx.content_rect().size();
        let width = (size.x - 48.).clamp(240., 1400.).min(size.x - 24.);
        let height = (size.y - 48.).clamp(220., 1000.).min(size.y - 24.);
        let revision = self.git_ui.commit.is_some();
        let file_mode = self.git_ui.file_version_open;
        let key = format!(
            "{:?}:{:?}:{file_mode}:{:?}:{:?}",
            self.git_ui.file,
            self.git_ui.commit,
            self.git_ui.diff_range,
            self.service.git.repo.as_ref().map(|r| &r.root)
        );
        if self
            .git_ui
            .diff_data
            .as_ref()
            .is_none_or(|d| d.revision != self.service.git.revision || d.key != key)
        {
            let diff = self
                .service
                .git
                .diff
                .as_ref()
                .filter(|d| Some(&d.path) == self.git_ui.file.as_ref());
            let bytes = if file_mode {
                self.service
                    .git
                    .file
                    .as_ref()
                    .filter(|f| {
                        Some(&f.path) == self.git_ui.file.as_ref()
                            && Some(&f.commit) == self.git_ui.commit.as_ref()
                    })
                    .map(|f| f.bytes.as_slice())
            } else if let Some((base, head, path)) = &self.git_ui.diff_range {
                self.service
                    .git
                    .revision_diff
                    .as_ref()
                    .filter(|d| {
                        &d.query.base == base && &d.query.head == head && &d.query.path == path
                    })
                    .map(|d| d.bytes.as_slice())
            } else if revision {
                self.service
                    .git
                    .details
                    .as_ref()
                    .filter(|d| Some(&d.commit) == self.git_ui.commit.as_ref())
                    .map(|d| d.patch.as_slice())
            } else {
                diff.map(|d| d.bytes.as_slice())
            };
            let text = bytes
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let meta = diff.map(|d| DiffMeta {
                path: d.path.clone(),
                side: d.side,
                binary: d.binary,
                hunks: d.hunks.iter().map(|h| h.header.clone()).collect(),
                bytes: d.bytes.clone(),
            });
            if self
                .git_ui
                .diff_data
                .as_ref()
                .and_then(|d| d.meta.as_ref())
                .is_some_and(|old| {
                    meta.as_ref().is_none_or(|new| {
                        old.path != new.path || old.side != new.side || old.bytes != new.bytes
                    })
                })
            {
                self.git_ui.selected_hunks.clear();
            }
            self.git_ui.diff_data = Some(std::sync::Arc::new(DiffData {
                revision: self.service.git.revision,
                key,
                content: super::git_diff_view::DiffContent::new(
                    &text,
                    self.git_ui.file.as_deref(),
                    file_mode,
                ),
                text,
                meta,
            }));
        }
        let data = self.git_ui.diff_data.as_ref().unwrap().clone();
        let diff = data.meta.as_ref();
        let mut close = false;
        let mut action = None;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background));
        egui::Modal::new(egui::Id::unique("git.diff.window"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                self.tokens.form_style(ui);
                ui.horizontal(|ui| {
                    ui.strong(if file_mode {
                        "File at Revision"
                    } else if revision {
                        "Commit Diff"
                    } else {
                        "Diff"
                    });
                    if let Some(path) = &self.git_ui.file {
                        ui.add_sized(
                            [(ui.available_width() - 36.).max(24.), 24.],
                            egui::Label::new(path.display().to_string()).truncate(),
                        )
                        .on_hover_text(path.display().to_string());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if components::database_icon_button(ui, Icon::Close, "Close Diff", true)
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                if let Some((base, head, _)) = &self.git_ui.diff_range {
                    let label = format!("{} → {}", base.as_deref().unwrap_or("Árvore vazia"), head);
                    let response = ui.add(
                        egui::Label::new(
                            RichText::new(label).small().color(color(self.tokens.muted)),
                        )
                        .truncate(),
                    );
                    if let Some(result) = self.service.git.revision_diff.as_ref().filter(|d| {
                        Some((&d.query.base, &d.query.head, &d.query.path))
                            == self.git_ui.diff_range.as_ref().map(|(b, h, p)| (b, h, p))
                    }) {
                        response.on_hover_text(format!(
                            "{} → {}",
                            result.base.as_deref().unwrap_or("Árvore vazia"),
                            result.head
                        ));
                    }
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let mut wrap = !self.git_ui.diff_no_wrap;
                    if ui.checkbox(&mut wrap, "Quebrar linhas").changed() {
                        self.git_ui.diff_no_wrap = !wrap;
                    }
                    egui::ComboBox::from_id_salt("git.diff.layout")
                        .selected_text(if self.git_ui.diff_unified {
                            "Unified"
                        } else {
                            "Side-by-side"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.git_ui.diff_unified,
                                false,
                                "Side-by-side",
                            );
                            ui.selectable_value(&mut self.git_ui.diff_unified, true, "Unified");
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::ComboBox,
                                true,
                                "Diff layout",
                            )
                        });
                    if ui
                        .button(if file_mode {
                            "Copy Contents"
                        } else {
                            "Copy Patch"
                        })
                        .clicked()
                    {
                        ctx.copy_text(data.text.clone());
                    }
                    if !revision
                        && !file_mode
                        && let Some(diff) = &diff
                    {
                        let indexed = diff.side == DiffSide::Index;
                        if ui
                            .add_enabled(
                                !self.git_ui.selected_hunks.is_empty(),
                                self.tokens.dialog_button(
                                    if indexed {
                                        "Unstage Selected Hunks"
                                    } else {
                                        "Stage Selected Hunks"
                                    },
                                    false,
                                ),
                            )
                            .clicked()
                        {
                            action = Some((
                                diff.path.clone(),
                                self.git_ui
                                    .selected_hunks
                                    .iter()
                                    .copied()
                                    .collect::<Vec<_>>(),
                                indexed,
                                diff.bytes.clone(),
                            ));
                        }
                    }
                });
                if !revision
                    && !file_mode
                    && let Some(diff) = &diff
                {
                    if diff.binary {
                        ui.label("Binary file · text comparison unavailable");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("git.diff.hunks")
                        .max_height(66.)
                        .codisseu()
                        .show_rows(ui, 22., diff.hunks.len(), |ui, range| {
                            for n in range {
                                let hunk = &diff.hunks[n];
                                let mut checked = self.git_ui.selected_hunks.contains(&n);
                                if ui
                                    .checkbox(&mut checked, format!("Hunk {}", n + 1))
                                    .on_hover_text(hunk)
                                    .changed()
                                {
                                    if checked {
                                        self.git_ui.selected_hunks.insert(n);
                                    } else {
                                        self.git_ui.selected_hunks.remove(&n);
                                    }
                                }
                            }
                        });
                }
                let unified = self.git_ui.diff_unified || file_mode;
                if !unified {
                    ui.columns(2, |columns| {
                        columns[0].label(if revision {
                            "Parent Revision"
                        } else if diff.as_ref().is_some_and(|d| d.side == DiffSide::Index) {
                            "HEAD"
                        } else {
                            "Index"
                        });
                        columns[1].label(if revision {
                            "Selected Revision"
                        } else if diff.as_ref().is_some_and(|d| d.side == DiffSide::Index) {
                            "Index"
                        } else {
                            "Your Version"
                        });
                    });
                }
                let used = ui.cursor().top() - ui.min_rect().top();
                let content_height = (height - used - 72.).max(60.);
                self.git_ui.diff_view.show(
                    ui,
                    data.clone(),
                    &self.tokens,
                    unified,
                    !self.git_ui.diff_no_wrap,
                    content_height,
                );
                self.git_status_line(ui);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if close {
            self.git_ui.diff_open = false;
            self.git_ui.file_version_open = false;
            self.git_ui.commit = None;
            self.git_ui.diff_range = None;
            if let Some(dialog) = self.git_ui.diff_return.take() {
                self.git_ui.dialog = Some(*dialog);
            }
        }
        if let Some((path, hunks, unstage, expected_patch)) = action
            && self.git_begin_intent(super::git::GitIntent::Other)
        {
            if !self.service.git.trusted {
                self.git_send(codisseu_core::git::GitCommand::Trust(true));
            }
            self.git_send(codisseu_core::git::GitCommand::HunksPrepare {
                path,
                hunks,
                unstage,
                expected_patch,
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn side_by_side_preserves_offsets_unicode_and_alignment() {
        let rows = diff_rows("@@ -10,3 +10,4 @@\n antes\n-antigo 😀\n+novo 😀\n+extra\n depois\n");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[2].left, Some((11, "antigo 😀".into(), true)));
        assert_eq!(rows[2].right, Some((11, "novo 😀".into(), true)));
        assert_eq!(rows[3].left, None);
        assert_eq!(rows[4].right, Some((13, "depois".into(), false)));
    }
}
