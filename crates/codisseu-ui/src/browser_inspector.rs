//! Native, bounded views over the browser inspector worker. No CDP or I/O here.
use super::*;
use codisseu_core::browser::{BrowserId, Surface, inspector::*};
use std::collections::HashSet;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Network,
    Dom,
    Storage,
    Variables,
    Tokens,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Storage {
    #[default]
    Cookies,
    Local,
    Session,
}
#[derive(Default)]
pub(super) struct InspectorUi {
    theme: codisseu_core::ThemePreference,
    session: Option<(u64, u64, u64, String)>,
    target: Option<BrowserId>,
    section: Section,
    ratio: f32,
    focus: HashSet<egui::Id>,
    bounds: Option<egui::Rect>,
    filter: String,
    error: Option<String>,
    request: Option<String>,
    request_detail: usize,
    reveal: bool,
    storage: Storage,
    key: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: String,
    cookie_session: bool,
    cookie_expires: String,
    expression: String,
    watch: String,
    breakpoint_url: String,
    breakpoint_line: String,
    breakpoint_condition: String,
    jwt: String,
    decoded: Option<Result<(String, String), String>>,
}
impl InspectorUi {
    pub(super) fn started(&mut self, id: BrowserId) {
        self.target = Some(id);
    }
    pub(super) fn target(&self) -> Option<BrowserId> {
        self.target
    }
    pub(super) fn clear_stopped(&mut self) {
        if self.session.is_some() || self.target.is_some() {
            let ratio = self.ratio;
            *self = Self {
                ratio,
                ..Default::default()
            };
        }
    }
    pub(super) fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.focused()).is_some_and(|id| {
            self.focus.contains(&id)
                || self.bounds.is_some_and(|bounds| {
                    ctx.read_response(id)
                        .is_some_and(|response| bounds.contains(response.rect.center()))
                })
        })
    }
    fn edit(&mut self, ui: &mut egui::Ui, label: &str, field: Field, secret: bool) {
        let reveal = self.reveal;
        let text = match field {
            Field::Filter => &mut self.filter,
            Field::Key => &mut self.key,
            Field::Value => &mut self.value,
            Field::Domain => &mut self.domain,
            Field::Path => &mut self.path,
            Field::SameSite => &mut self.same_site,
            Field::CookieExpires => &mut self.cookie_expires,
            Field::Expression => &mut self.expression,
            Field::Watch => &mut self.watch,
            Field::Jwt => &mut self.jwt,
            Field::BreakpointUrl => &mut self.breakpoint_url,
            Field::BreakpointLine => &mut self.breakpoint_line,
            Field::BreakpointCondition => &mut self.breakpoint_condition,
        };
        ui.label(label);
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .id_salt(("inspector", label))
                .hint_text(label)
                .password(secret && !reveal)
                .char_limit(match field {
                    Field::Jwt => 32768,
                    Field::Filter => 512,
                    Field::Key => 4096,
                    Field::Value => 65536,
                    Field::Domain => 512,
                    Field::Path => 2048,
                    Field::SameSite => 16,
                    Field::CookieExpires | Field::BreakpointLine => 32,
                    Field::BreakpointUrl => 8192,
                    _ => 16384,
                })
                .desired_width(ui.available_width().max(30.)),
        );
        r.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, ui.is_enabled(), label)
        });
        self.focus.insert(r.id);
    }
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &Snapshot,
        tokens: &Tokens,
    ) -> (Vec<Action>, bool, bool) {
        let identity = state
            .session
            .as_ref()
            .map(|s| (s.tab, s.epoch, state.document_epoch, s.origin.clone()));
        if self.session != identity {
            let ratio = self.ratio;
            let target = self.target;
            *self = Self {
                target,
                session: identity,
                ratio,
                cookie_session: true,
                path: "/".into(),
                same_site: "Lax".into(),
                ..Default::default()
            };
        }
        self.theme = tokens.theme;
        let mut actions = Vec::new();
        let mut close = false;
        let mut restart = false;
        ui.spacing_mut().item_spacing = egui::vec2(6., 4.);
        ui.horizontal(|ui| {
            let title_width = (ui.available_width() - 52.).max(20.);
            ui.add_sized(
                [title_width, 24.],
                egui::Label::new(egui::RichText::new("Inspetor frontend").strong()).truncate(),
            );
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 24.), egui::Sense::hover());
            if state.busy {
                egui::Spinner::new().size(14.).paint_at(ui, rect);
            }
            if icon_button(ui, Icon::Close, "Fechar inspetor", false, 24.).clicked() {
                close = true;
            }
        });
        if let Some(s) = &state.session {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&s.origin)
                        .small()
                        .color(color(tokens.muted)),
                )
                .truncate(),
            )
            .on_hover_text(&s.origin);
        }
        let width = ui.available_width();
        let compact = width < 400.;
        let columns = if width < 180. {
            2
        } else if compact {
            3
        } else if width < 620. {
            2
        } else {
            5
        };
        egui::Grid::new("inspector.sections")
            .num_columns(columns)
            .spacing([6., 4.])
            .show(ui, |ui| {
                for (index, (section, label, short)) in [
                    (Section::Network, "Rede", "Rede"),
                    (Section::Dom, "Componentes / DOM", "DOM"),
                    (Section::Storage, "Armazenamento", "Dados"),
                    (Section::Variables, "Variáveis", "JS"),
                    (Section::Tokens, "Tokens", "Tokens"),
                ]
                .into_iter()
                .enumerate()
                {
                    let response = ui
                        .add(
                            egui::Button::selectable(
                                self.section == section,
                                if compact { short } else { label },
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        )
                        .on_hover_text(label);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            ui.is_enabled(),
                            label,
                        )
                    });
                    if response.clicked() {
                        self.section = section;
                        match section {
                            Section::Dom => actions.push(Action::RefreshDom),
                            Section::Storage => actions.push(Action::RefreshStorage),
                            _ => {}
                        }
                    }
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        if state.lifecycle == Lifecycle::Starting {
            ui.label("Conectando à página…");
        }
        if let Some(error) = &self.error {
            ui.colored_label(color(tokens.error), error);
        }
        if let Some(error) = &state.error {
            ui.colored_label(color(tokens.error), error);
        }
        if state.lifecycle == Lifecycle::Failed && ui.button("Reconectar inspetor").clicked() {
            restart = true;
        }
        if state.lifecycle != Lifecycle::Ready {
            return (actions, close, restart);
        }
        egui::ScrollArea::both()
            .id_salt("inspector.content")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 12.).max(100.));
                match self.section {
                    Section::Network => self.network(ui, state, &mut actions),
                    Section::Dom => self.dom(ui, state, &mut actions),
                    Section::Storage => self.storage(ui, state, &mut actions),
                    Section::Variables => self.variables(ui, state, &mut actions),
                    Section::Tokens => self.tokens(ui),
                }
            });
        (actions, close, restart)
    }
    fn secrets(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(if self.reveal {
                "Ocultar valores"
            } else {
                "Revelar valores"
            })
            .clicked()
        {
            self.reveal = !self.reveal;
        }
    }
    fn network(&mut self, ui: &mut egui::Ui, s: &Snapshot, a: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            let mut enabled = s.network_enabled;
            if ui.checkbox(&mut enabled, "Capturar rede").changed() {
                a.push(Action::Network { enabled });
            }
            let mut preserve = s.preserve_network;
            if ui.checkbox(&mut preserve, "Preservar histórico").changed() {
                a.push(Action::PreserveNetwork(preserve));
            }
            if ui.button("Limpar rede").clicked() {
                self.request = None;
                a.push(Action::ClearNetwork);
            }
            self.secrets(ui);
        });
        self.edit(ui, "Filtrar requests", Field::Filter, false);
        ui.label(egui::RichText::new("Até 200 requests · respostas sob demanda até 1 MiB · capture antes de recarregar a página").small());
        let filter = self.filter.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("inspector.network.rows")
            .max_height(148.)
            .show(ui, |ui| {
                for r in s.network.iter().rev().filter(|r| {
                    filter.is_empty()
                        || r.url.to_lowercase().contains(&filter)
                        || r.method.to_lowercase().contains(&filter)
                }) {
                    let status = r.status.map(|x| format!("{x:.0}")).unwrap_or_else(|| {
                        if r.failed.is_some() {
                            "erro".into()
                        } else {
                            "…".into()
                        }
                    });
                    let label = format!(
                        "{}  {}  {}  {}  {}",
                        status,
                        r.method,
                        r.kind,
                        r.duration_ms
                            .map(|x| format!("{x:.0} ms"))
                            .unwrap_or_default(),
                        masked_url(&r.url, self.reveal)
                    );
                    if ui
                        .add_sized(
                            [ui.available_width(), 20.],
                            egui::Button::selectable(self.request.as_deref() == Some(&r.id), label)
                                .truncate(),
                        )
                        .on_hover_text(masked_url(&r.url, self.reveal))
                        .clicked()
                    {
                        self.request = Some(r.id.clone());
                    }
                }
            });
        let Some(r) = self
            .request
            .as_ref()
            .and_then(|id| s.network.iter().find(|r| &r.id == id))
        else {
            ui.label("Selecione uma request para inspecionar os detalhes.");
            return;
        };
        ui.separator();
        ui.label(masked_url(&r.url, self.reveal));
        if ui
            .button("Copiar URL da request")
            .on_hover_text("Copia o endereço real da request, inclusive parâmetros ocultos")
            .clicked()
        {
            ui.ctx().copy_text(r.url.clone());
        }
        ui.horizontal_wrapped(|ui| {
            for (i, label) in ["Headers", "Payload", "Response"].iter().enumerate() {
                ui.selectable_value(&mut self.request_detail, i, *label);
            }
        });
        ui.label(format!(
            "{} · {} · {:.0} bytes transferidos · {} decodificados{}",
            r.protocol,
            r.mime_type,
            r.encoded_bytes,
            r.decoded_bytes
                .map(|x| format!("{x:.0} bytes"))
                .unwrap_or_else(|| "tamanho desconhecido".into()),
            if r.from_cache { " · cache" } else { "" }
        ));
        if let Some(error) = &r.failed {
            ui.label(error);
        }
        match self.request_detail {
            0 => {
                ui.strong("Request headers");
                headers(ui, &r.request_headers, self.reveal);
                ui.strong("Response headers");
                headers(ui, &r.response_headers, self.reveal);
            }
            1 => {
                if let Some(body) = &r.post_data {
                    payload(ui, body, self.reveal, self.theme);
                } else {
                    ui.label("Sem corpo enviado capturado.");
                }
            }
            _ => {
                if ui
                    .add_enabled(
                        r.finished && r.failed.is_none(),
                        egui::Button::new("Capturar resposta"),
                    )
                    .clicked()
                {
                    a.push(Action::GetResponseBody {
                        request_id: r.id.clone(),
                    });
                }
                if let Some(body) = s.bodies.get(&r.id) {
                    if let Some(error) = &body.error {
                        ui.label(error);
                    }
                    if body.base64 {
                        ui.label("Conteúdo binário (base64)");
                    }
                    if body.truncated {
                        ui.label("Conteúdo limitado pelo limite de captura.");
                    }
                    payload(ui, &body.text, self.reveal, self.theme);
                } else {
                    ui.label("O corpo é obtido apenas por solicitação e após a conclusão do carregamento.");
                }
            }
        }
    }
    fn dom(&mut self, ui: &mut egui::Ui, s: &Snapshot, a: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Atualizar DOM").clicked() {
                a.push(Action::RefreshDom);
            }
            self.secrets(ui);
            ui.label("DOM real · custom elements · shadow DOM");
        });
        ui.label(egui::RichText::new("A árvore mostra elementos da página. Props/state internos de React/Vue não são uma árvore DOM.").small());
        if s.dom_truncated {
            ui.label("Árvore limitada a 500 nós / 8 níveis / tamanho de captura. F12 oferece a inspeção completa do DOM.");
        }
        egui::ScrollArea::vertical()
            .id_salt("inspector.dom.rows")
            .max_height(180.)
            .show(ui, |ui| {
                for node in &s.dom {
                    let name = if node.node_type == 3 {
                        format!("#text {}", preview(&node.value, 80))
                    } else {
                        let id = node
                            .attributes
                            .iter()
                            .find(|(k, _)| k == "id")
                            .map(|(_, v)| format!("#{v}"))
                            .unwrap_or_default();
                        format!(
                            "{}{}{}{}",
                            node.name.to_lowercase(),
                            id,
                            if node.shadow_root {
                                "  [shadow root]"
                            } else {
                                ""
                            },
                            if node.custom_element {
                                "  [componente]"
                            } else {
                                ""
                            }
                        )
                    };
                    ui.horizontal(|ui| {
                        ui.add_space((node.depth.min(16) * 10) as f32);
                        if ui
                            .selectable_label(s.selected_node == Some(node.id), name)
                            .clicked()
                        {
                            a.push(Action::SelectNode { node_id: node.id });
                        }
                    });
                }
            });
        ui.separator();
        ui.strong("Atributos");
        pairs(ui, &s.attributes, self.reveal);
        ui.strong("Estilos computados");
        pairs(ui, &s.styles, true);
        ui.strong("Propriedades DOM");
        properties(ui, &s.properties, self.reveal, a);
    }
    fn storage(&mut self, ui: &mut egui::Ui, s: &Snapshot, a: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Atualizar armazenamento").clicked() {
                a.push(Action::RefreshStorage);
            }
            self.secrets(ui);
        });
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [
                (Storage::Cookies, "Cookies"),
                (Storage::Local, "localStorage"),
                (Storage::Session, "sessionStorage"),
            ] {
                if ui.selectable_label(self.storage == kind, label).clicked() {
                    self.storage = kind;
                    self.key.clear();
                    self.value.clear();
                }
            }
            if ui.button("Nova entrada").clicked() {
                self.key.clear();
                self.value.clear();
                self.domain.clear();
                self.path = "/".into();
                self.secure = false;
                self.http_only = false;
                self.same_site = "Lax".into();
                self.cookie_session = true;
                self.cookie_expires.clear();
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("inspector.storage.rows")
            .max_height(130.)
            .show(ui, |ui| {
                if self.storage == Storage::Cookies {
                    for c in &s.cookies {
                        let label = format!(
                            "{}  {}{}  {}{}",
                            c.name,
                            c.domain,
                            c.path,
                            if c.http_only { "HttpOnly  " } else { "" },
                            if self.reveal {
                                preview(&c.value, 120)
                            } else {
                                "••••••".into()
                            }
                        );
                        if ui
                            .selectable_label(
                                self.key == c.name
                                    && self.domain == c.domain
                                    && self.path == c.path,
                                label,
                            )
                            .clicked()
                        {
                            self.key = c.name.clone();
                            self.value = c.value.clone();
                            self.domain = c.domain.clone();
                            self.path = c.path.clone();
                            self.secure = c.secure;
                            self.http_only = c.http_only;
                            self.same_site = c.same_site.clone().unwrap_or_default();
                            self.cookie_session = c.session;
                            self.cookie_expires = c.expires.to_string();
                        }
                    }
                } else {
                    let rows = if self.storage == Storage::Local {
                        &s.local_storage
                    } else {
                        &s.session_storage
                    };
                    for (k, v) in rows {
                        if ui
                            .selectable_label(
                                self.key == *k,
                                format!(
                                    "{k}  {}",
                                    if self.reveal {
                                        preview(v, 140)
                                    } else {
                                        "••••••".into()
                                    }
                                ),
                            )
                            .clicked()
                        {
                            self.key = k.clone();
                            self.value = v.clone();
                        }
                    }
                }
            });
        ui.separator();
        self.edit(ui, "Nome / chave", Field::Key, false);
        self.edit(ui, "Valor armazenado", Field::Value, true);
        if self.storage == Storage::Cookies {
            self.edit(ui, "Domínio do cookie", Field::Domain, false);
            self.edit(ui, "Caminho do cookie", Field::Path, false);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.secure, "Secure");
                ui.checkbox(&mut self.http_only, "HttpOnly");
            });
            self.edit(ui, "SameSite (Lax, Strict ou None)", Field::SameSite, false);
            ui.checkbox(&mut self.cookie_session, "Cookie de sessão");
            if !self.cookie_session {
                self.edit(ui, "Expiração Unix do cookie", Field::CookieExpires, false);
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.key.is_empty(), egui::Button::new("Salvar valor"))
                .clicked()
            {
                a.push(if self.storage == Storage::Cookies {
                    Action::SetCookie {
                        cookie: Cookie {
                            name: self.key.clone(),
                            value: self.value.clone(),
                            domain: self.domain.clone(),
                            path: self.path.clone(),
                            secure: self.secure,
                            http_only: self.http_only,
                            same_site: (!self.same_site.is_empty()).then(|| self.same_site.clone()),
                            session: self.cookie_session,
                            expires: self.cookie_expires.parse().unwrap_or(0.),
                        },
                    }
                } else {
                    Action::SetStorage {
                        kind: self.storage_kind(),
                        key: self.key.clone(),
                        value: self.value.clone(),
                    }
                });
            }
            if ui
                .add_enabled(!self.key.is_empty(), egui::Button::new("Excluir valor"))
                .clicked()
            {
                a.push(if self.storage == Storage::Cookies {
                    Action::DeleteCookie {
                        name: self.key.clone(),
                        domain: self.domain.clone(),
                        path: self.path.clone(),
                    }
                } else {
                    Action::DeleteStorage {
                        kind: self.storage_kind(),
                        key: self.key.clone(),
                    }
                });
                self.value.clear();
            }
            if ui
                .add_enabled(!self.value.is_empty(), egui::Button::new("Copiar valor"))
                .clicked()
            {
                ui.ctx().copy_text(self.value.clone());
            }
            if ui
                .add_enabled(
                    !self.value.is_empty(),
                    egui::Button::new("Examinar como JWT"),
                )
                .clicked()
            {
                self.jwt = self.value.clone();
                self.decoded = None;
                self.section = Section::Tokens;
            }
        });
    }
    fn storage_kind(&self) -> StorageKind {
        if self.storage == Storage::Local {
            StorageKind::Local
        } else {
            StorageKind::Session
        }
    }
    fn variables(&mut self, ui: &mut egui::Ui, s: &Snapshot, a: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            let paused = s.paused.is_some();
            for (enabled, action, label) in [
                (!paused, Action::Pause, "Pausar JavaScript"),
                (paused, Action::Resume, "Continuar JavaScript"),
                (paused, Action::StepOver, "Step over (F8)"),
                (paused, Action::StepInto, "Step into (F7)"),
                (paused, Action::StepOut, "Step out (Shift+F8)"),
            ] {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    a.push(action);
                }
            }
            self.secrets(ui);
        });
        if let Some(p) = &s.paused {
            ui.strong(format!("JavaScript pausado · {}", p.reason));
            for frame in &p.frames {
                if ui
                    .selectable_label(
                        p.selected.as_deref() == Some(&frame.id),
                        format!(
                            "{}  {}:{}",
                            frame.function_name,
                            masked_url(&frame.url, self.reveal),
                            frame.line + 1
                        ),
                    )
                    .clicked()
                {
                    a.push(Action::SelectFrame {
                        frame_id: frame.id.clone(),
                    });
                }
            }
            if let Some(frame) = p
                .frames
                .iter()
                .find(|f| p.selected.as_deref() == Some(&f.id))
                .or(p.frames.first())
            {
                for scope in &frame.scopes {
                    ui.collapsing(format!("{} · {}", scope.kind, scope.name), |ui| {
                        if ui.button("Ler variáveis do escopo").clicked() {
                            a.push(Action::GetProperties {
                                object_id: scope.object_id.clone(),
                            });
                        }
                        if let Some(values) = s.scope_values.get(&scope.object_id) {
                            properties(ui, values, self.reveal, a);
                        }
                    });
                }
            }
        } else {
            ui.label("JavaScript em execução. Pause ou use um breakpoint para inspecionar variáveis locais.");
        }
        ui.separator();
        self.edit(ui, "Expressão JavaScript", Field::Expression, false);
        if ui
            .add_enabled(
                !self.expression.trim().is_empty(),
                egui::Button::new("Avaliar expressão"),
            )
            .clicked()
        {
            a.push(Action::Evaluate {
                expression: self.expression.clone(),
            });
        }
        ui.small("Avaliar executa a expressão no contexto selecionado da página.");
        if let Some(e) = &s.evaluation {
            evaluation(ui, e, self.reveal, a);
        }
        self.edit(ui, "Expressão de watch", Field::Watch, false);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.watch.trim().is_empty(),
                    egui::Button::new("Adicionar watch"),
                )
                .clicked()
            {
                let mut watches = s
                    .watches
                    .iter()
                    .map(|w| w.expression.clone())
                    .collect::<Vec<_>>();
                if !watches.contains(&self.watch) {
                    watches.push(self.watch.clone());
                }
                a.push(Action::SetWatches(watches));
                self.watch.clear();
            }
            if ui.button("Atualizar watches").clicked() {
                a.push(Action::RefreshWatches);
            }
            if ui.button("Limpar watches").clicked() {
                a.push(Action::SetWatches(Vec::new()));
            }
        });
        for watch in &s.watches {
            ui.label(&watch.expression);
            evaluation(ui, watch, self.reveal, a);
        }
        if !s.properties.is_empty() {
            ui.strong("Objeto selecionado");
            properties(ui, &s.properties, self.reveal, a);
        }
        ui.collapsing("Breakpoints por URL e linha", |ui| {
            self.edit(ui, "URL do script", Field::BreakpointUrl, false);
            self.edit(
                ui,
                "Linha do breakpoint (a partir de 1)",
                Field::BreakpointLine,
                false,
            );
            self.edit(
                ui,
                "Condição do breakpoint (opcional)",
                Field::BreakpointCondition,
                false,
            );
            let line = self.breakpoint_line.parse::<u32>().ok().filter(|x| *x > 0);
            if ui
                .add_enabled(
                    line.is_some() && !self.breakpoint_url.is_empty(),
                    egui::Button::new("Adicionar breakpoint"),
                )
                .clicked()
            {
                a.push(Action::SetBreakpoint {
                    url: self.breakpoint_url.clone(),
                    line: line.unwrap() - 1,
                    condition: self.breakpoint_condition.clone(),
                });
            }
            for b in &s.breakpoints {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{}:{}",
                        masked_url(&b.url, self.reveal),
                        b.line + 1
                    ));
                    if ui.small_button("Remover breakpoint").clicked() {
                        a.push(Action::RemoveBreakpoint {
                            breakpoint_id: b.id.clone(),
                        });
                    }
                });
            }
        });
    }
    fn tokens(&mut self, ui: &mut egui::Ui) {
        ui.label("Examinar JWT localmente");
        self.secrets(ui);
        self.edit(ui, "Token JWT", Field::Jwt, true);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Decodificar JWT").clicked() {
                self.decoded = Some(decode_jwt(&self.jwt));
            }
            if ui.button("Limpar token").clicked() {
                self.jwt.clear();
                self.decoded = None;
            }
        });
        ui.strong("Decodificação não verifica a assinatura nem a autenticidade do token.");
        if let Some(result) = &self.decoded {
            match result {
                Ok((header, body)) => {
                    ui.strong("Header");
                    payload(ui, header, self.reveal, self.theme);
                    ui.strong("Payload");
                    payload(ui, body, self.reveal, self.theme);
                }
                Err(error) => {
                    ui.label(error);
                }
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Field {
    Filter,
    Key,
    Value,
    Domain,
    Path,
    SameSite,
    CookieExpires,
    Expression,
    Watch,
    Jwt,
    BreakpointUrl,
    BreakpointLine,
    BreakpointCondition,
}
fn preview(text: &str, max: usize) -> String {
    let mut out = text.chars().take(max).collect::<String>();
    if text.chars().nth(max).is_some() {
        out.push('…');
    }
    out
}
fn masked_url(url: &str, reveal: bool) -> String {
    if reveal {
        return preview(url, 2048);
    }
    let no_fragment = url.split('#').next().unwrap_or(url);
    let (base, query) = no_fragment.split_once('?').unwrap_or((no_fragment, ""));
    let base = if let Some((scheme, rest)) = base.split_once("://") {
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(authority_end);
        if let Some((_, host)) = authority.rsplit_once('@') {
            format!("{scheme}://{host}{path}")
        } else {
            base.into()
        }
    } else {
        base.into()
    };
    if query.is_empty() {
        preview(&base, 2048)
    } else {
        format!(
            "{}?{}",
            preview(&base, 1024),
            query
                .split('&')
                .take(32)
                .map(|x| format!("{}=••••", preview(x.split('=').next().unwrap_or(""), 80)))
                .collect::<Vec<_>>()
                .join("&")
        )
    }
}
fn headers(ui: &mut egui::Ui, rows: &[(String, String)], reveal: bool) {
    if ui
        .button("Copiar headers")
        .on_hover_text("Copia os valores reais, inclusive os ocultos")
        .clicked()
    {
        ui.ctx().copy_text(
            rows.iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    for (k, v) in rows {
        let sensitive = [
            "authorization",
            "cookie",
            "set-cookie",
            "token",
            "api-key",
            "secret",
        ]
        .iter()
        .any(|x| k.to_ascii_lowercase().contains(x));
        ui.horizontal_wrapped(|ui| {
            ui.monospace(k);
            ui.label(if sensitive && !reveal {
                "••••••".into()
            } else {
                preview(v, 4096)
            });
        });
    }
}
fn pairs(ui: &mut egui::Ui, rows: &[(String, String)], reveal: bool) {
    for (k, v) in rows {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(k);
            ui.label(if reveal {
                preview(v, 2048)
            } else {
                "••••••".into()
            });
        });
    }
}
fn properties(ui: &mut egui::Ui, rows: &[Property], reveal: bool, a: &mut Vec<Action>) {
    for p in rows {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(&p.name);
            if p.getter {
                ui.label("getter (não executado)");
            } else {
                ui.label(if reveal {
                    preview(&p.value.description, 512)
                } else {
                    "••••••".into()
                });
            }
            if let Some(id) = &p.value.object_id
                && ui.small_button("Expandir objeto").clicked()
            {
                a.push(Action::GetProperties {
                    object_id: id.clone(),
                });
            }
        });
    }
}
fn evaluation(ui: &mut egui::Ui, e: &Evaluation, reveal: bool, a: &mut Vec<Action>) {
    if let Some(error) = &e.error {
        ui.label(error);
    }
    if let Some(value) = &e.value {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(if reveal {
                preview(&value.description, 4096)
            } else {
                "••••••".into()
            });
            if let Some(id) = &value.object_id
                && ui.button("Expandir resultado").clicked()
            {
                a.push(Action::GetProperties {
                    object_id: id.clone(),
                });
            }
        });
    }
}
fn payload(ui: &mut egui::Ui, text: &str, reveal: bool, theme: codisseu_core::ThemePreference) {
    if !reveal {
        ui.label("Conteúdo oculto. Use Revelar valores para visualizar.");
        return;
    }
    // Formatting only on bounded chunks avoids repeated parsing/copying megabyte bodies per frame.
    let parsed = (text.len() <= 32768)
        .then(|| serde_json::from_str::<serde_json::Value>(text).ok())
        .flatten();
    let rendered = parsed
        .as_ref()
        .and_then(pretty_json_preview)
        .unwrap_or_else(|| preview(text, 32768));
    if ui.button("Copiar conteúdo").clicked() {
        ui.ctx().copy_text(text.to_owned());
    }
    let font = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Monospace)
        .cloned()
        .unwrap_or_else(|| FontId::monospace(13.));
    let job = highlight::layout_themed(
        &rendered,
        if parsed.is_some() { "json" } else { "txt" },
        font,
        theme,
    );
    ui.add(egui::Label::new(job).wrap());
    if text.len() > 32768 {
        ui.small(
            "Visualização limitada a 32 mil caracteres; Copiar conteúdo copia o corpo capturado.",
        );
    }
}
// Pretty printing must not amplify deeply nested JSON into megabytes each frame.
fn pretty_json_preview(value: &serde_json::Value) -> Option<String> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > 32768 {
                return Err(std::io::Error::other("JSON preview limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Bounded(Vec::new());
    serde_json::to_writer_pretty(&mut out, value).ok()?;
    String::from_utf8(out.0).ok()
}
fn decode_jwt(input: &str) -> Result<(String, String), String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let input = input.trim();
    if input.len() > 32768 {
        return Err("Token excede o limite de 32 KiB.".into());
    }
    let parts = input.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
        return Err("JWT deve conter header, payload e assinatura separados por pontos.".into());
    }
    let decode = |part: &str| -> Result<String, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(part.trim_end_matches('='))
            .map_err(|_| "JWT contém base64url inválido.")?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "JWT contém JSON inválido.")?;
        if !value.is_object() {
            return Err("Header e payload devem ser objetos JSON.".into());
        }
        pretty_json_preview(&value)
            .ok_or_else(|| "JSON decodificado excede o limite de visualização de 32 KiB.".into())
    };
    Ok((decode(parts[0])?, decode(parts[1])?))
}
impl CodisseuApp {
    pub(super) fn browser_inspector_action(&mut self, action: Action) {
        self.browser_inspector_ui.error = self.service.dispatch_browser_inspector(action).err();
    }
    pub(super) fn browser_inspector_active(&self, id: BrowserId) -> bool {
        self.service.browser_inspector.state.lifecycle != Lifecycle::Stopped
            && (self
                .service
                .browser_inspector
                .state
                .session
                .as_ref()
                .is_some_and(|s| s.tab == id)
                || self.browser_inspector_ui.target() == Some(id))
    }
    pub(super) fn browser_inspector_layout(
        &mut self,
        ui: &mut egui::Ui,
        id: BrowserId,
        rect: egui::Rect,
    ) {
        let ratio = if self.browser_inspector_ui.ratio == 0. {
            0.47
        } else {
            self.browser_inspector_ui.ratio
        };
        let at = rect.top() + rect.height() * ratio;
        let page = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.right(), (at - 3.).max(rect.top())),
        );
        let tools = egui::Rect::from_min_max(
            egui::pos2(rect.left(), (at + 3.).min(rect.bottom())),
            rect.max,
        );
        let divider = egui::Rect::from_min_max(
            egui::pos2(rect.left(), at - 3.),
            egui::pos2(rect.right(), at + 3.),
        );
        let drag = ui
            .interact(
                divider,
                egui::Id::unique(("browser.inspector.split", id)),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if drag.dragged()
            && let Some(pos) = drag.interact_pointer_pos()
        {
            self.browser_inspector_ui.ratio =
                ((pos.y - rect.top()) / rect.height()).clamp(0.15, 0.75);
        }
        ui.painter()
            .rect_filled(divider, 0., color(self.tokens.border));
        self.browser_surface(ui, id, Surface::Page, page);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(tools)
                .id_salt(("browser.inspector", id)),
        );
        child.set_clip_rect(tools.intersect(ui.clip_rect()));
        child
            .painter()
            .rect_filled(tools, 0., color(self.tokens.panel));
        let (actions, close, restart) = egui::Frame::new()
            .inner_margin(8)
            .show(&mut child, |ui| {
                self.browser_inspector_ui.show(
                    ui,
                    &self.service.browser_inspector.state,
                    &self.tokens,
                )
            })
            .inner;
        self.browser_inspector_ui.bounds = Some(tools.intersect(ui.clip_rect()));
        for action in actions {
            self.browser_inspector_action(action);
        }
        if close {
            self.service.stop_browser_inspector();
        } else if restart {
            self.browser_inspector_open(id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jwt_is_bounded_and_decodes_without_verification() {
        let token = "eyJhbGciOiJub25lIn0.eyJzdWIiOiI0MiJ9.";
        let (h, p) = decode_jwt(token).unwrap();
        assert!(h.contains("none"));
        assert!(p.contains("42"));
        assert!(decode_jwt("broken").is_err());
        assert!(decode_jwt(&"a".repeat(32769)).is_err());
        assert!(decode_jwt("W10.W10.sig").is_err());
        assert!(decode_jwt("é.eyJzdWIiOiI0MiJ9.sig").is_err());
    }
    #[test]
    fn deep_json_cannot_amplify_preview_memory() {
        let mut value = serde_json::json!((0..1000).collect::<Vec<_>>());
        for _ in 0..32 {
            value = serde_json::json!({"nested":value});
        }
        assert!(serde_json::to_string(&value).unwrap().len() < 32768);
        assert!(pretty_json_preview(&value).is_none());
        assert!(
            pretty_json_preview(&serde_json::json!({"answer":42}))
                .unwrap()
                .contains("42")
        );
    }
    #[test]
    fn urls_hide_query_credentials_and_fragments() {
        let value = masked_url(
            "https://user:secret@example.test/path?token=abc&code=xyz#secret",
            false,
        );
        assert!(!value.contains("abc"));
        assert!(!value.contains("xyz"));
        assert!(!value.contains("user"));
        assert!(!value.contains("secret"));
        assert!(value.contains("token="));
        assert_eq!(
            masked_url("https://example.test/users/@alice", false),
            "https://example.test/users/@alice"
        );
    }
}
