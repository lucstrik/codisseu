//! Local file history uses its own profile worker and the existing editor transaction.
use super::*;
use codisseu_core::DocumentId;

#[derive(Default)]
pub(super) struct HistoryUi {
    document: Option<DocumentId>,
    path: PathBuf,
    filter: String,
    label: Option<(u64, String)>,
    label_focus: bool,
    cache: Option<Comparison>,
}
struct Comparison {
    revision: u64,
    version: u64,
    left: Vec<String>,
    right: Vec<String>,
    limited: bool,
    rows: Vec<(Option<usize>, Option<usize>)>,
}
fn align(left: &[String], right: &[String]) -> Vec<(Option<usize>, Option<usize>)> {
    let prefix = left.iter().zip(right).take_while(|(a, b)| a == b).count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (lend, rend) = (left.len() - suffix, right.len() - suffix);
    let mut rows: Vec<_> = (0..prefix).map(|i| (Some(i), Some(i))).collect();
    rows.extend((0..(lend - prefix).max(rend - prefix)).map(|i| {
        (
            ((prefix + i) < lend).then_some(prefix + i),
            ((prefix + i) < rend).then_some(prefix + i),
        )
    }));
    rows.extend((0..suffix).map(|i| (Some(lend + i), Some(rend + i))));
    rows
}
impl HistoryUi {
    pub(super) fn is_open(&self) -> bool {
        self.document.is_some()
    }
}
fn lines(text: &str) -> (Vec<String>, bool) {
    let mut end = text.len().min(256 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    let mut rows: Vec<_> = prefix
        .split('\n')
        .take(2000)
        .map(|line| line.trim_end_matches('\r').chars().take(512).collect())
        .collect();
    if rows.is_empty() {
        rows.push(String::new());
    }
    let limited = end < text.len()
        || prefix.split('\n').count() > 2000
        || prefix
            .lines()
            .take(2000)
            .any(|line| line.chars().count() > 512);
    (rows, limited)
}
impl CodisseuApp {
    pub(super) fn open_file_history(&mut self, label: bool) {
        let Some(doc) = self.service.active_document().filter(|d| !d.is_untitled()) else {
            return;
        };
        let (id, path, version) = (doc.id, doc.path.clone(), doc.version);
        self.send(Command::ShowLocalHistory { id });
        self.history_ui = HistoryUi {
            document: Some(id),
            path,
            label: label.then_some((version, String::new())),
            label_focus: label,
            ..Default::default()
        };
    }
    fn close_file_history(&mut self) {
        self.history_ui = Default::default();
        self.service.local_history.close();
        self.focus_editor = true;
    }
    pub(super) fn file_history_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.history_ui.document else {
            return;
        };
        let Some(doc) = self
            .service
            .documents
            .iter()
            .find(|d| d.id == id && d.path == self.history_ui.path)
        else {
            self.close_file_history();
            return;
        };
        let version = doc.version;
        let path = doc.path.clone();
        if self.history_ui.label.is_some() {
            let mut apply = false;
            let mut cancel = false;
            let result = egui::Modal::new(egui::Id::unique("local-history.label"))
                .frame(
                    egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                        .fill(color(self.tokens.bar)),
                )
                .show(ctx, |ui| {
                    ui.set_width(380_f32.min(ctx.content_rect().width() - 48.));
                    self.tokens.form_style(ui);
                    ui.strong("Put Label");
                    let (_, label) = self.history_ui.label.as_mut().unwrap();
                    let field = ui.add(
                        egui::TextEdit::singleline(label)
                            .desired_width(f32::INFINITY)
                            .hint_text("Label")
                            .char_limit(128)
                            .align(egui::Align2::LEFT_CENTER),
                    );
                    field.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            true,
                            "History label",
                        )
                    });
                    if self.history_ui.label_focus {
                        field.request_focus();
                        self.history_ui.label_focus = false;
                    }
                    ui.horizontal(|ui| {
                        apply = ui
                            .add_enabled(
                                !label.trim().is_empty(),
                                self.tokens.dialog_button("Create Label", true),
                            )
                            .clicked();
                        cancel = ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                    });
                    apply |=
                        !label.trim().is_empty() && consume_exact(ctx, Modifiers::NONE, Key::Enter);
                });
            if apply {
                let (expected_version, label) = self.history_ui.label.take().unwrap();
                self.send(Command::LabelLocalHistory {
                    id,
                    expected_version,
                    label,
                });
            } else if cancel || result.should_close() {
                self.history_ui.label = None;
            }
            return;
        }
        let busy = self.service.local_history.is_busy();
        let preview = self
            .service
            .local_history
            .view
            .as_ref()
            .filter(|v| v.path == path)
            .and_then(|v| v.preview.as_ref())
            .cloned();
        if let Some(preview) = &preview
            && self
                .history_ui
                .cache
                .as_ref()
                .is_none_or(|c| c.revision != preview.id || c.version != version)
        {
            let (left, a) = lines(&preview.text);
            let (right, b) = lines(doc.text());
            let rows = align(&left, &right);
            self.history_ui.cache = Some(Comparison {
                revision: preview.id,
                version,
                left,
                right,
                limited: a || b,
                rows,
            });
        }
        if preview.is_none() {
            self.history_ui.cache = None;
        }
        let revisions = self
            .service
            .local_history
            .view
            .as_ref()
            .filter(|v| v.path == path)
            .map(|v| v.revisions.clone())
            .unwrap_or_default();
        let mut select = None;
        let mut close = false;
        let mut restore = false;
        let mut label = false;
        let mut refresh = false;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.bar))
            .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)));
        let result=egui::Modal::new(egui::Id::unique("local-history.window")).frame(frame).show(ctx,|ui|{
            ui.set_width(1100_f32.min(ctx.content_rect().width()-48.));
            ui.horizontal(|ui|{
                ui.strong("Local History");ui.add_sized([(ui.available_width()-38.).max(30.),26.],egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(path.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=components::database_icon_button(ui,Icon::Close,"Close Local History",true).clicked();});
            });
            ui.separator();
            ui.horizontal(|ui|{
                restore=ui.add_enabled(!busy&&preview.is_some(),self.tokens.dialog_button("Restore to Editor",true)).clicked();
                label=ui.add(self.tokens.dialog_button("Put Label…",false)).clicked();
                refresh=components::database_icon_button(ui,Icon::Refresh,"Refresh Local History",!busy).clicked();
                if busy{ui.spinner();ui.label("Loading history…");}
            });
            let height=(ctx.content_rect().height()-235.).clamp(130.,470.);
            let left_width=(ui.available_width()*0.26).clamp(150.,260.);
            ui.horizontal_top(|ui|{
                ui.allocate_ui_with_layout(egui::vec2(left_width,height),egui::Layout::top_down(egui::Align::Min),|ui|{
                    ui.set_width(left_width);ui.set_min_height(height);
                    let field=ui.add(egui::TextEdit::singleline(&mut self.history_ui.filter).hint_text("Filter revisions").char_limit(256).desired_width(f32::INFINITY).align(egui::Align2::LEFT_CENTER));
                    field.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput,true,"Filter revisions"));
                    let filter=self.history_ui.filter.to_lowercase();
                    egui::ScrollArea::vertical().id_salt("local-history.revisions").max_height(height-38.).codisseu().show(ui,|ui|{
                        for revision in &revisions {
                            let title=format!("{}  {}",revision.date,revision.label.as_deref().unwrap_or(&revision.reason));
                            if !title.to_lowercase().contains(&filter){continue;}
                            let selected=preview.as_ref().is_some_and(|p|p.id==revision.id);
                            let (rect,response)=ui.allocate_exact_size(egui::vec2(left_width-8.,self.tokens.row_height*2.),egui::Sense::click());
                            if selected||response.hovered(){ui.painter().rect_filled(rect,self.tokens.control_corner_radius,color(if selected{self.tokens.selection}else{self.tokens.border}));}
                            let painter=ui.painter().with_clip_rect(rect.shrink2(egui::vec2(6.,0.)));
                            painter.text(rect.left_top()+egui::vec2(6.,5.),egui::Align2::LEFT_TOP,&revision.date,egui::FontId::proportional(self.tokens.ui_font_size-2.),color(if selected{self.tokens.text}else{self.tokens.muted}));
                            let caption=revision.label.as_deref().map(str::to_owned).unwrap_or_else(||path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                            painter.text(rect.left_top()+egui::vec2(6.,24.),egui::Align2::LEFT_TOP,caption,egui::FontId::proportional(self.tokens.ui_font_size),color(self.tokens.text));
                            response.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::ListBoxOption,true,format!("History revision {}: {}",revision.id,revision.label.as_deref().unwrap_or(&revision.reason))));
                            ui.ctx().accesskit_node_builder(response.id,|n|n.set_selected(selected));
                            if response.clicked(){select=Some(revision.id);}
                            response.on_hover_text(format!("{title}\n{} bytes",revision.bytes));
                        }
                        if revisions.is_empty()&&!busy{ui.label("No recorded revisions for this file.");}
                    });
                });
                let (divider,_)=ui.allocate_exact_size(egui::vec2(1.,height),egui::Sense::hover());ui.painter().rect_filled(divider,0.,color(self.tokens.border));
                let width=(ui.available_width()-8.).max(150.);
                ui.allocate_ui_with_layout(egui::vec2(width,height),egui::Layout::top_down(egui::Align::Min),|ui|{
                    ui.set_min_height(height);ui.painter().rect_filled(egui::Rect::from_min_size(ui.cursor().min,egui::vec2(width,height)),0.,color(self.tokens.editor));
                    let caption=preview.as_ref().and_then(|p|revisions.iter().find(|r|r.id==p.id)).map(|r|format!("{} · {}",r.date,path.file_name().unwrap_or_default().to_string_lossy())).unwrap_or_else(||"Selected Revision".into());
                    ui.horizontal(|ui|{ui.add_sized([width/2.-8.,24.],egui::Label::new(&caption).truncate()).on_hover_text(&caption);ui.label("Current");});
                    if let Some(cache)=&self.history_ui.cache {
                        let row_height=self.tokens.editor_line_height;
                        egui::ScrollArea::both().id_salt("local-history.compare").max_height(height-30.).codisseu().show_rows(ui,row_height,cache.rows.len(),|ui,range|{
                            let column=(width/2.-7.).max(60.);
                            for row in range {ui.horizontal(|ui|{
                                let pair=cache.rows[row];let old=pair.0.and_then(|i|cache.left.get(i)).map(String::as_str);let new=pair.1.and_then(|i|cache.right.get(i)).map(String::as_str);let changed=old!=new;
                                for (text,removed,line) in [(old.unwrap_or(""),true,pair.0),(new.unwrap_or(""),false,pair.1)] {
                                    let (rect,response)=ui.allocate_exact_size(egui::vec2(column,row_height),egui::Sense::hover());
                                    ui.painter().rect_filled(rect,0.,color(if changed&&line.is_some(){if removed{self.tokens.git_diff_deleted}else{self.tokens.git_diff_added}}else{self.tokens.editor}));
                                    ui.painter().with_clip_rect(rect.shrink2(egui::vec2(4.,0.))).text(rect.left_center()+egui::vec2(4.,0.),egui::Align2::LEFT_CENTER,line.map(|line|format!("{}  {text}",line+1)).unwrap_or_default(),egui::FontId::monospace(self.tokens.editor_font_size),color(self.tokens.text));
                                    response.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::Label,true,format!("{} line {}: {text}",if removed{"Revision"}else{"Current"},line.map_or(row+1,|i|i+1))));
                                }
                            });}
                        });
                        if cache.limited{ui.label(egui::RichText::new("Preview limited · restoring uses the complete revision").small().color(color(self.tokens.muted)));}
                    }
                });
            });
            ui.separator();
            if let Some(error)=&self.service.local_history.error {ui.colored_label(color(self.tokens.error),error);}
            ui.label(egui::RichText::new("Restore changes the editor. Ctrl+Z undoes; Ctrl+S saves to disk.").small().color(color(self.tokens.muted)));
        });
        if close || result.should_close() {
            self.close_file_history();
            return;
        }
        if let Some(revision) = select {
            self.send(Command::SelectLocalRevision { revision });
        }
        if refresh {
            self.send(Command::ShowLocalHistory { id });
        }
        if label {
            self.history_ui.label = Some((version, String::new()));
            self.history_ui.label_focus = true;
        }
        if restore && let Some(preview) = preview {
            self.send(Command::RestoreLocalRevision {
                id,
                expected_version: version,
                revision: preview.id,
            });
            self.close_file_history();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{fs, thread, time::Instant};
    fn settle(h: &mut Harness<CodisseuApp>) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            h.step();
            if !h.state().service.is_busy() {
                break;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(2));
        }
        h.run();
    }
    fn capture(h: &mut Harness<CodisseuApp>, name: &str, scale: f32) {
        if let Some(root) = std::env::var_os("CODISSEU_HISTORY_CAPTURES") {
            let root = PathBuf::from(root);
            fs::create_dir_all(&root).unwrap();
            h.ctx.request_repaint();
            h.run_steps(2);
            h.run();
            h.render()
                .unwrap()
                .save(root.join(format!("{name}-{scale}.png")))
                .unwrap();
        }
    }
    #[test]
    fn file_history_gui_labels_restore_cancel_undo_save_and_geometry() {
        for language in ["php", "ts"] {
            for scale in [1., 1.25] {
                let temp = tempfile::tempdir().unwrap();
                let source = temp.path().join(format!("sample.{language}"));
                let original = if language == "php" {
                    "<?php\r\n$value = 'ação 😀';\r\n"
                } else {
                    "export const value = 'ação 😀';\r\n"
                };
                let saved = original.replace("ação 😀", "saved B");
                let dirty = original.replace("ação 😀", "dirty C");
                fs::write(&source, original).unwrap();
                let mut service = AppService::new(temp.path().join("profile/session.json"));
                // Exercise explicit restore/Undo without the independent save timer.
                service
                    .dispatch(Command::SetAutoSave {
                        enabled: false,
                        delay_ms: 1000,
                    })
                    .unwrap();
                service.dispatch(Command::OpenFile(source.clone())).unwrap();
                let mut app = CodisseuApp::new(service);
                app.show_project = false;
                app.show_database = false;
                let mut h = Harness::builder()
                    .with_size([1440., 900.])
                    .with_pixels_per_point(scale)
                    .with_max_steps(200)
                    .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
                settle(&mut h);
                let id = h.state().service.active.unwrap();
                let version = h.state().service.active_document().unwrap().version;
                h.state_mut()
                    .service
                    .dispatch(Command::Edit {
                        id,
                        expected_version: version,
                        text: saved.clone(),
                    })
                    .unwrap();
                h.state_mut()
                    .service
                    .dispatch(Command::Save { id })
                    .unwrap();
                settle(&mut h);
                let version = h.state().service.active_document().unwrap().version;
                h.state_mut()
                    .service
                    .dispatch(Command::Edit {
                        id,
                        expected_version: version,
                        text: dirty.clone(),
                    })
                    .unwrap();
                // The normal File menu, not a direct view mutation.
                h.get_by_label("Menu principal").click();
                h.run();
                h.get_by_label("Arquivo").click();
                h.run();
                h.get_by_label("Local History").click();
                h.run();
                h.get_by_label("Show History").click();
                settle(&mut h);
                assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
                let revision = h
                    .state()
                    .service
                    .local_history
                    .view
                    .as_ref()
                    .unwrap()
                    .revisions
                    .last()
                    .unwrap()
                    .id;
                h.get_by_label(&format!("History revision {revision}: Opened"))
                    .click();
                settle(&mut h);
                assert!(h.get_by_label("Close Local History").rect().right() < 1440.);
                assert!(
                    h.query_by_label("Revision line 2: $value = 'ação 😀';")
                        .is_some()
                        || language == "ts"
                );
                capture(&mut h, &format!("history-{language}"), scale);
                h.get_by_label("Put Label…").click();
                settle(&mut h);
                assert!(h.get_by_label("History label").is_focused());
                h.event(egui::Event::Text("Before experiment".into()));
                h.run();
                h.get_by_label("Create Label").click();
                settle(&mut h);
                let checkpoint = h
                    .state()
                    .service
                    .local_history
                    .view
                    .as_ref()
                    .unwrap()
                    .revisions
                    .iter()
                    .find(|r| r.label.as_deref() == Some("Before experiment"))
                    .unwrap()
                    .id;
                assert_eq!(fs::read_to_string(&source).unwrap(), saved);
                h.get_by_label(&format!("History revision {revision}: Opened"))
                    .click();
                settle(&mut h);
                h.set_size(egui::vec2(980., 760.));
                h.run();
                assert!(h.get_by_label("Close Local History").rect().right() <= 980.);
                capture(&mut h, &format!("narrow-{language}"), scale);
                h.key_press(Key::Escape);
                settle(&mut h);
                assert!(!h.state().history_ui.is_open());
                assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
                h.state_mut().open_file_history(false);
                settle(&mut h);
                h.get_by_label(&format!("History revision {checkpoint}: Before experiment"))
                    .click();
                settle(&mut h);
                h.get_by_label("Restore to Editor").click();
                settle(&mut h);
                assert!(!h.state().history_ui.is_open());
                assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
                h.state_mut().open_file_history(false);
                settle(&mut h);
                h.get_by_label(&format!("History revision {revision}: Opened"))
                    .click();
                settle(&mut h);
                h.get_by_label("Restore to Editor").click();
                settle(&mut h);
                assert_eq!(
                    h.state().service.active_document().unwrap().text(),
                    original
                );
                assert!(h.state().service.active_document().unwrap().is_dirty());
                assert_eq!(fs::read_to_string(&source).unwrap(), saved);
                h.state_mut()
                    .service
                    .dispatch(Command::Undo { id })
                    .unwrap();
                settle(&mut h);
                assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
                h.state_mut()
                    .service
                    .dispatch(Command::Redo { id })
                    .unwrap();
                h.state_mut()
                    .service
                    .dispatch(Command::Save { id })
                    .unwrap();
                settle(&mut h);
                assert_eq!(fs::read_to_string(&source).unwrap(), original);
            }
        }
    }
    #[test]
    fn comparison_preview_has_explicit_byte_line_and_scalar_limits() {
        let (a, _) = lines("first\nlast\n");
        let (b, _) = lines("first\ninsert\nlast\n");
        assert_eq!(
            align(&a, &b),
            vec![
                (Some(0), Some(0)),
                (None, Some(1)),
                (Some(1), Some(2)),
                (Some(2), Some(3))
            ]
        );
        let (result, limited) = lines(&format!("{}\n{}", "😀".repeat(2000), "line\n".repeat(5000)));
        assert!(limited);
        assert!(result.len() <= 2000);
        assert!(result.iter().all(|l| l.chars().count() <= 512));
        assert_eq!(lines("a\r\nb"), (vec!["a".into(), "b".into()], false));
    }
}
