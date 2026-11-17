mod cli;
#[cfg(test)]
mod cli_tests;
mod diagnostics;
mod new_window;

use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Options {
    cli_mode: bool,
    wait: bool,
    new_window: bool,
    runtime_info: bool,
    browsers: Vec<String>,
    workspace: Option<PathBuf>,
    files: Vec<PathBuf>,
    session: Option<PathBuf>,
    profile: Option<PathBuf>,
    capture: Option<PathBuf>,
    close_after_capture: bool,
    check: Option<PathBuf>,
    no_restore: bool,
    database: Option<PathBuf>,
    providers: Option<PathBuf>,
    start_providers: bool,
    no_auto_providers: bool,
    execute_sql: bool,
    wait_diagnostics: bool,
    wait_nonempty_diagnostics: bool,
    problems: bool,
}
fn options() -> Result<Option<Options>, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Diretório atual indisponível: {e}"))?;
    parse_options(std::env::args_os().skip(1), &cwd)
}
fn parse_options(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
    cwd: &std::path::Path,
) -> Result<Option<Options>, String> {
    let mut result = Options::default();
    let mut args = arguments.into_iter();
    let mut positional_only = false;
    while let Some(arg) = args.next() {
        if positional_only {
            cli::add_path(&mut result, arg, cwd)?;
            continue;
        }
        match arg.to_str() {
            Some("--") => positional_only = true,
            Some("--cli") => result.cli_mode = true,
            Some("--wait" | "-w") => result.wait = true,
            Some("--new-window" | "-n") => result.new_window = true,
            Some("--version" | "-v") => {
                println!(
                    "Codisseu {} ({}-{})",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                );
                return Ok(None);
            }
            Some("--help" | "-h") => {
                println!(concat!(
                    "Comando instalado: codisseu [PASTA | ARQUIVO…]\n",
                    "  codisseu .              abrir a pasta atual\n",
                    "  codisseu                retomar o último projeto\n",
                    "  codisseu \"Minha pasta\" abrir caminho com espaços\n",
                    "  -w, --wait              aguardar a janela encerrar\n",
                    "  -n, --new-window        abrir uma nova janela vazia\n",
                    "  -v, --version           mostrar versão\n",
                    "  --                      fim das opções; aceitar nomes iniciados por hífen\n"
                ));
                println!(
                    "Codisseu — IDE nativa\n\nUso: codisseu [--workspace PASTA_OU_CODE_WORKSPACE] [--open ARQUIVO]… [--session ARQUIVO]\n       codisseu --check-file ARQUIVO\n\n--browser URL            abrir uma aba de navegador integrada explicitamente\n--runtime-info           verificar caminhos dos motores incluídos no pacote\n--no-restore             iniciar sem restaurar a sessão\n--capture PNG            capturar framebuffer da janela após carregamento\n--close-after-capture    persistir sessão e encerrar após captura\n--database ARQUIVO       conectar SQLite existente, somente leitura\n--providers JSON         carregar configuração dos servidores LSP\n--start-providers        autorizar e executar os servidores configurados\n--no-auto-providers      desativar inicialização automática nesta sessão\n--execute-sql            executar documento SQL ativo após carregamento\n--wait-diagnostics       aguardar publicação real de PHP/TS antes da captura\n--wait-nonempty-diagnostics exigir diagnóstico por documento PHP/TS (fixtures com erro)\n--problems               mostrar painel de diagnósticos\n\nAtalhos: Ctrl+Shift+N (nova janela), Ctrl+O, Ctrl+Shift+O, Ctrl+S, Ctrl+Z, Ctrl+Shift+Z, Ctrl+W, Ctrl+Enter."
                );
                return Ok(None);
            }
            Some("--runtime-info") => result.runtime_info = true,
            Some("--browser") => {
                if result.browsers.len() >= 8 {
                    return Err("Limite de8abas de navegador".into());
                }
                result.browsers.push(
                    args.next()
                        .ok_or("Falta URL")?
                        .into_string()
                        .map_err(|_| "URL inválida")?,
                );
            }
            Some("--workspace") => {
                result.workspace = Some(PathBuf::from(args.next().ok_or("Falta pasta")?))
            }
            Some("--open") => result
                .files
                .push(PathBuf::from(args.next().ok_or("Falta arquivo")?)),
            Some("--session") => {
                result.session = Some(PathBuf::from(args.next().ok_or("Falta sessão")?))
            }
            Some("--profile") => {
                result.profile = Some(PathBuf::from(args.next().ok_or("Falta perfil")?))
            }
            Some("--capture") => {
                result.capture = Some(PathBuf::from(args.next().ok_or("Falta PNG")?))
            }
            Some("--check-file") => {
                result.check = Some(PathBuf::from(args.next().ok_or("Falta arquivo")?))
            }
            Some("--database") => {
                result.database = Some(PathBuf::from(args.next().ok_or("Falta arquivo SQLite")?))
            }
            Some("--providers") => {
                result.providers = Some(PathBuf::from(
                    args.next().ok_or("Falta configuração dos providers")?,
                ))
            }
            Some("--start-providers") => result.start_providers = true,
            Some("--no-auto-providers") => result.no_auto_providers = true,
            Some("--execute-sql") => result.execute_sql = true,
            Some("--wait-diagnostics") => result.wait_diagnostics = true,
            Some("--wait-nonempty-diagnostics") => {
                result.wait_diagnostics = true;
                result.wait_nonempty_diagnostics = true;
            }
            Some("--problems") => result.problems = true,
            Some("--close-after-capture") => result.close_after_capture = true,
            Some("--no-restore") => result.no_restore = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("Opção desconhecida: {value}. Use codisseu --help"));
            }
            _ => cli::add_path(&mut result, arg, cwd)?,
        }
    }
    if result.close_after_capture && result.capture.is_none() {
        return Err("--close-after-capture exige --capture".into());
    }
    if result.wait_diagnostics && result.capture.is_none() {
        return Err("--wait-diagnostics exige --capture".into());
    }
    if result.wait_diagnostics && !result.start_providers && result.no_auto_providers {
        return Err("--wait-diagnostics exige providers automáticos ou --start-providers".into());
    }
    Ok(Some(result))
}
fn state_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("codisseu/session.json"));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/state/codisseu/session.json"))
        .ok_or("Defina --session: diretório de estado indisponível".into())
}
fn run() -> Result<(), String> {
    let Some(mut options) = options()? else {
        return Ok(());
    };
    if options.runtime_info {
        let resources = codisseu_runtime::discover()?
            .ok_or("Esta instalação de desenvolvimento não contém pacote gerenciado de motores")?;
        println!("{}", resources.report_json()?);
        return Ok(());
    }
    let use_default_session = options.session.is_none()
        || state_path().is_ok_and(|path| options.session.as_ref() == Some(&path));
    let default_session = options.session.clone().map_or_else(state_path, Ok)?;
    cli::prepare_profile(&mut options, &default_session)?;
    if options.cli_mode && !options.wait && options.check.is_none() {
        if cfg!(target_os = "linux")
            && std::env::var_os("DISPLAY").is_none()
            && std::env::var_os("WAYLAND_DISPLAY").is_none()
        {
            return Err(
                "Não há uma sessão gráfica disponível. Abra o comando em um terminal do desktop"
                    .into(),
            );
        }
        return cli::launch(&new_window::executable()?, &options);
    }
    let session_path = options.session.take().ok_or("Sessão indisponível")?;
    let profile_path = options.profile.take().ok_or("Perfil indisponível")?;
    // Explicit --session also isolates trust/preferences for native fixtures.
    // The core refuses automatic trust when this profile is within the project.
    let provider_profile = profile_path.with_extension("provider-policy.json");
    let legacy = session_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("storm/session.json"));
    let window_session = session_path.clone();
    let diagnostics_session = session_path.clone();
    let mut service = AppService::new_with_profile(session_path, profile_path.clone());
    if let Some(path) = options.check {
        service.dispatch(Command::OpenFile(path))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while service.is_busy() {
            service.poll();
            if Instant::now() > deadline {
                return Err("Timeout ao abrir arquivo".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(error) = &service.last_error {
            return Err(error.clone());
        }
        let document = service.active_document().ok_or("Documento não carregado")?;
        println!(
            "{}: {} bytes UTF-8, versão {}",
            document.path.display(),
            document.text().len(),
            document.version
        );
        return Ok(());
    }
    let diagnostics = match diagnostics::Log::new(&diagnostics_session) {
        Ok(log) => {
            log.install_panic_hook();
            Some(log)
        }
        Err(error) => {
            eprintln!("Codisseu diagnostics unavailable: {error}");
            None
        }
    };
    if options.workspace.is_none() && options.files.is_empty() && !options.no_restore {
        if use_default_session {
            service.dispatch(Command::RestoreSessionWithFallback { legacy })?;
        } else {
            service.dispatch(Command::RestoreSession)?;
        }
    }
    if let Some(path) = options.workspace {
        service.dispatch(Command::OpenWorkspace(path))?;
    }
    for path in options.files {
        service.dispatch(Command::OpenFile(path))?;
    }
    let mut app = CodisseuApp::new(service);
    app.configure_new_window_launcher(move || new_window::launch(&window_session, &profile_path));
    app.configure_startup(
        options.database,
        options.providers,
        options.start_providers,
        options.execute_sql,
        options.problems,
    );
    if !options.no_auto_providers {
        app.configure_provider_autostart(
            provider_profile,
            Some(PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.tools/providers.json"
            ))),
        );
    }
    for url in options.browsers {
        app.queue_browser_url(&url);
    }
    app.wait_for_diagnostics = options.wait_diagnostics;
    app.wait_for_nonempty_diagnostics = options.wait_nonempty_diagnostics;
    if let Some(path) = options.capture {
        app.capture_to(path, options.close_after_capture);
    }
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Codisseu — workspace de desenvolvimento")
            .with_app_id("io.codisseu.Codisseu")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../../../assets/icons/io.codisseu.Codisseu-128.png"
                ))
                .map_err(|e| format!("Ícone Codisseu inválido: {e}"))?,
            )
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        // Preserve the measured flat Dark palette instead of adding output noise.
        dithering: false,
        ..Default::default()
    };
    let window_diagnostics = diagnostics.clone();
    let result = eframe::run_native(
        "Codisseu",
        native,
        Box::new(move |cc| {
            if let Some(log) = window_diagnostics {
                codisseu_ui::set_diagnostic_sink(&cc.egui_ctx, move |message| log.record(message));
            }
            if let Some(state) = &cc.wgpu_render_state {
                eprintln!("Codisseu renderer: {:?}", state.adapter.get_info());
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string());
    if let Some(log) = diagnostics {
        log.record(if result.is_ok() {
            "native_window_closed"
        } else {
            "native_window_error"
        });
        log.flush();
    }
    result
}
fn main() {
    if let Some(code) = codisseu_core::git::rebase::helper_main() {
        std::process::exit(code);
    }

    if let Err(error) = run() {
        eprintln!("Codisseu: {error}");
        std::process::exit(1);
    }
}
