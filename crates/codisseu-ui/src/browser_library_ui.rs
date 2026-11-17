//! Native favorites and origin-scoped access controls; no credential in Debug/JSON.
use super::*;
use codisseu_core::browser_library::{self as saved, Action, Secret};
#[derive(Default, PartialEq)]
pub(super) enum Mode {
    #[default]
    Favorites,
    Bookmark,
    Access,
}
#[derive(Default)]
pub(super) struct LibraryUi {
    pub open: bool,
    pub tab: u64,
    mode: Mode,
    id: Option<u64>,
    name: String,
    url: String,
    project: String,
    filter: String,
    username: String,
    password: Secret,
    reveal: bool,
    pub(super) notice: String,
    only_project: bool,
    revision: u64,
}
impl CodisseuApp {
    pub(super) fn browser_saved_open(&mut self, id: u64, mode: Mode) {
        let Some(tab) = self.service.browser.tabs.get(&id) else {
            return;
        };
        self.browser_ui.library = LibraryUi {
            open: true,
            tab: id,
            mode,
            name: tab.title.clone(),
            url: tab.url.clone(),
            revision: self.service.browser_library.revision,
            project: if self.service.workspace_path().is_some() {
                self.service.workspace_label()
            } else {
                String::new()
            },
            ..Default::default()
        };
        if self.browser_ui.library.mode == Mode::Bookmark
            && let Some(existing) = self
                .service
                .browser_library
                .state
                .bookmarks
                .iter()
                .find(|b| {
                    b.url == self.browser_ui.library.url
                        && b.project == self.browser_ui.library.project
                })
        {
            self.browser_ui.library.id = Some(existing.id);
            self.browser_ui.library.name = existing.name.clone();
        }
    }
    pub(super) fn browser_saved_tick(&mut self, ctx: &egui::Context) {
        if self.browser_ui.library.revision != self.service.browser_library.revision {
            self.browser_ui.library.revision = self.service.browser_library.revision;
            self.browser_ui.library.notice.clear();
        }
        if let Some(login) = self.service.take_browser_login() {
            let ui = &mut self.browser_ui.library;
            if ui.open
                && ui.mode == Mode::Access
                && ui.tab == login.tab
                && saved::origin(&ui.url).ok().as_deref() == Some(&login.origin)
            {
                ui.username = login.username;
                ui.password = login.password;
                ui.notice.clear();
            }
        }
        let ui = &self.browser_ui.library;
        if ui.open
            && ui.mode == Mode::Access
            && self
                .service
                .browser
                .tabs
                .get(&ui.tab)
                .is_none_or(|t| t.url != ui.url)
        {
            self.browser_ui.library = LibraryUi::default();
            self.service.cancel_browser_login();
        }
        if self.service.browser_library.is_busy() || self.service.browser_login_busy() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    fn saved_action(&mut self, action: Action) {
        if matches!(&action, Action::SaveLogin { .. }) {
            self.service.cancel_browser_login();
        }
        self.browser_ui.library.notice = match self.service.browser_library.dispatch(action) {
            Ok(()) => "Salvando…".into(),
            Err(e) => e,
        };
    }
    pub(super) fn browser_saved_window(&mut self, ctx: &egui::Context) {
        if !self.browser_ui.library.open {
            return;
        }
        let mut close = false;
        let mut action = None;
        let mut navigate = None;
        let mut edit = None;
        let mut read = false;
        let mut fill = None;
        let busy = self.service.browser_library.is_running() || self.service.browser_login_busy();
        let title = match self.browser_ui.library.mode {
            Mode::Favorites => "Favoritos do navegador",
            Mode::Bookmark => "Salvar favorito",
            Mode::Access => "Acessos do navegador",
        };
        let modal=egui::Modal::new(egui::Id::unique("browser.saved.library"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20))
            .show(ctx,|ui|{
                self.tokens.form_style(ui);
                ui.set_width((ctx.content_rect().width()-80.).clamp(240.,560.));
                ui.heading(title);ui.add_space(4.);ui.separator();ui.add_space(8.);
                // A stable body keeps the dialog centered and its targets still while
                // the vault, list or status changes, including between dialog modes.
                egui::ScrollArea::vertical().id_salt("browser.library.body")
                    .max_height((ctx.content_rect().height()-260.).clamp(80.,460.))
                    .auto_shrink([false,false]).codisseu().show(ui,|ui|{
                    let u=&mut self.browser_ui.library;
                    match u.mode {
                        Mode::Bookmark=>{
                            ui.add(egui::Label::new(RichText::new("Guarde este endereço e organize seus frontends por projeto.").color(color(self.tokens.muted))).wrap());
                            ui.add_space(12.);
                            saved_field(ui,"Nome","Nome do favorito",&mut u.name,256);
                            saved_field(ui,"URL","URL do favorito",&mut u.url,8192);
                            saved_field(ui,"Projeto / grupo","Projeto do favorito",&mut u.project,4096);
                        }
                        Mode::Favorites=>{
                            let search=ui.add(egui::TextEdit::singleline(&mut u.filter).desired_width(f32::INFINITY).hint_text("Buscar por nome, URL ou projeto").char_limit(256));
                            ui.ctx().accesskit_node_builder(search.id,|node|node.set_label("Buscar favoritos"));
                            ui.checkbox(&mut u.only_project,"Somente projeto atual");ui.add_space(8.);
                            let filter=u.filter.to_lowercase();
                            let items=self.service.browser_library.state.bookmarks.iter().filter(|b|(!u.only_project||b.project==u.project)&&(filter.is_empty()||b.name.to_lowercase().contains(&filter)||b.url.to_lowercase().contains(&filter)||b.project.to_lowercase().contains(&filter))).collect::<Vec<_>>();
                            if items.is_empty(){ui.add_space(16.);ui.label(RichText::new("Nenhum favorito encontrado.").color(color(self.tokens.muted)));}
                            egui::ScrollArea::vertical().id_salt("browser.favorites").max_height(300.).codisseu().show_rows(ui,64.,items.len(),|ui,range|{
                                for row in range {let b=items[row];ui.push_id(b.id,|ui|{
                                    egui::Frame::new().fill(color(self.tokens.editor)).stroke(egui::Stroke::new(1.,color(self.tokens.border))).corner_radius(self.tokens.control_corner_radius)
                                        .inner_margin(egui::Margin::symmetric(12,6)).show(ui,|ui|{
                                        ui.horizontal(|ui|{
                                            let width=(ui.available_width()-76.).max(70.);
                                            let link=ui.allocate_ui_with_layout(egui::vec2(width,24.),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_width(width);ui.add(egui::Label::new(RichText::new(&b.name).color(color(self.tokens.accent))).truncate().sense(egui::Sense::click()))}).inner;
                                            link.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::Button,ui.is_enabled(),&b.name));
                                            if link.on_hover_text(&b.url).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){navigate=Some(b.url.clone());}
                                            if components::database_icon_button(ui,Icon::Settings,"Editar favorito",!busy).clicked(){edit=Some(b.clone());}
                                            if components::database_icon_button(ui,Icon::Delete,"Excluir favorito",!busy).clicked(){action=Some(Action::DeleteBookmark(b.id));}
                                        });
                                        ui.add(egui::Label::new(RichText::new(format!("{}  ·  {}",b.project,b.url)).small().color(color(self.tokens.muted))).truncate()).on_hover_text(&b.url);
                                    });
                                });}
                            });
                        }
                        Mode::Access=>{
                            let origin=saved::origin(&u.url).ok();
                            egui::Frame::new().fill(color(self.tokens.editor)).corner_radius(self.tokens.control_corner_radius).inner_margin(12).show(ui,|ui|{
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui|{
                                    components::paint_icon(ui,egui::Rect::from_min_size(ui.cursor().min,egui::vec2(16.,16.)),Icon::PrimaryKey,color(self.tokens.muted));
                                    ui.add_space(24.);
                                    ui.add(egui::Label::new(RichText::new(origin.as_deref().unwrap_or("Abra uma página HTTP/HTTPS para salvar um acesso.")).color(color(self.tokens.muted))).truncate()).on_hover_text(origin.as_deref().unwrap_or(""));
                                });
                            });
                            ui.add_space(12.);ui.strong("Salvar acesso");ui.add_space(4.);
                            saved_field(ui,"Usuário","Usuário do acesso",&mut u.username,1024);
                            ui.label("Senha");
                            let password=ui.add(egui::TextEdit::singleline(u.password.edit()).desired_width(f32::INFINITY).password(!u.reveal).char_limit(16384));
                            ui.ctx().accesskit_node_builder(password.id,|node|node.set_label("Senha do acesso"));
                            ui.checkbox(&mut u.reveal,"Revelar senha digitada");ui.add_space(4.);
                            ui.horizontal(|ui|{
                                read=ui.add_enabled(!busy&&origin.is_some(),self.tokens.dialog_button("Ler formulário",false)).clicked();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                    if ui.add_enabled(!busy&&origin.is_some()&&!u.username.trim().is_empty()&&!u.password.expose().is_empty(),self.tokens.dialog_button("Salvar no cofre",true)).clicked(){
                                        action=Some(Action::SaveLogin{origin:origin.clone().unwrap(),username:u.username.clone(),password:std::mem::take(&mut u.password)});u.reveal=false;
                                    }
                                });
                            });
                            ui.add(egui::Label::new(RichText::new("A senha fica no cofre do sistema. Preencher não envia o formulário.").small().color(color(self.tokens.muted))).wrap());
                            ui.add_space(16.);ui.separator();ui.add_space(8.);ui.strong("Acessos salvos");
                            let items=self.service.browser_library.state.logins.iter().filter(|l|Some(&l.origin)==origin.as_ref()).collect::<Vec<_>>();
                            if items.is_empty(){ui.label(RichText::new("Nenhum acesso salvo para este site.").color(color(self.tokens.muted)));}
                            egui::ScrollArea::vertical().id_salt("browser.saved.logins").max_height(160.).codisseu().show_rows(ui,48.,items.len(),|ui,range|{
                                for row in range {let l=items[row];ui.push_id(&l.id,|ui|{
                                    egui::Frame::new().fill(color(self.tokens.editor)).stroke(egui::Stroke::new(1.,color(self.tokens.border))).corner_radius(self.tokens.control_corner_radius)
                                        .inner_margin(egui::Margin::symmetric(12,8)).show(ui,|ui|{
                                        ui.horizontal(|ui|{
                                            let width=(ui.available_width()-136.).max(60.);
                                            ui.allocate_ui_with_layout(egui::vec2(width,26.),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_width(width);ui.add(egui::Label::new(&l.username).truncate()).on_hover_text(&l.username)});
                                            if ui.add_enabled(!busy,self.tokens.dialog_button("Preencher",false)).clicked(){fill=Some(l.id.clone());}
                                            if components::database_icon_button(ui,Icon::Delete,"Excluir acesso",!busy).clicked(){action=Some(Action::DeleteLogin(l.id.clone()));}
                                        });
                                    });
                                });}
                            });
                        }
                    }
                });
                ui.add_space(8.);
                let error=self.service.browser_library.error.as_deref();
                let notice=if !self.browser_ui.library.notice.is_empty(){self.browser_ui.library.notice.as_str()}else{self.service.browser_login_notice().unwrap_or(&self.service.browser_library.notice)};
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),24.),egui::Layout::left_to_right(egui::Align::Center),|ui|{
                    if busy {ui.add(egui::Spinner::new().size(16.));}
                    let message=error.unwrap_or(if busy {"Aguarde…"}else{notice});
                    ui.add(egui::Label::new(RichText::new(message).small().color(color(if error.is_some(){self.tokens.error}else{self.tokens.muted}))).truncate()).on_hover_text(message);
                });
                ui.add_space(12.);ui.separator();ui.add_space(8.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    let u=&self.browser_ui.library;
                    if u.mode==Mode::Bookmark && ui.add_enabled(!busy&&!u.name.trim().is_empty()&&!u.url.trim().is_empty(),self.tokens.dialog_button("Salvar favorito",true)).clicked(){action=Some(Action::SaveBookmark{id:u.id,name:u.name.clone(),url:u.url.clone(),project:u.project.clone()});}
                    close=ui.add(self.tokens.dialog_button("Concluído",u.mode!=Mode::Bookmark)).clicked();
                });
            });
        if let Some(action) = action {
            self.saved_action(action);
        }
        if let Some(b) = edit {
            let u = &mut self.browser_ui.library;
            u.mode = Mode::Bookmark;
            u.id = Some(b.id);
            u.name = b.name;
            u.url = b.url;
            u.project = b.project;
        }
        let id = self.browser_ui.library.tab;
        if read && let Err(e) = self.service.read_browser_login(id) {
            self.browser_ui.library.notice = e;
        }
        if let Some(key) = fill
            && let Err(e) = self.service.fill_browser_login(id, &key)
        {
            self.browser_ui.library.notice = e;
        }
        if let Some(url) = navigate {
            self.open_browser_url(&url);
            close = true;
        }
        if close || modal.should_close() {
            self.browser_ui.library = LibraryUi::default();
            self.service.cancel_browser_login();
            self.focus_editor = false;
        }
    }
}
fn saved_field(ui: &mut egui::Ui, label: &str, name: &str, text: &mut String, limit: usize) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .char_limit(limit),
    );
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
    ui.add_space(4.);
}
