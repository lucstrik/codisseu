//! Real editor input with an empty controlled provider. Proves the Rust resource
//! adapter and shared editor behavior; does not claim real Phpactor semantics.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn wait(h: &mut H, ready: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        assert!(
            h.state().service.last_error.is_none(),
            "{:?}",
            h.state().service.last_error
        );
        if ready(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}; completion={:?}; navigation={:?}",
            h.state().service.status,
            h.state().service.completion.error,
            h.state().service.navigation.error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn idle(h: &mut H) {
    wait(h, |a| !a.service.is_busy());
}
fn key(h: &mut H, mut modifiers: Modifiers, key: Key) {
    modifiers.command = modifiers.ctrl;
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.step();
}
fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn fixture(name: &str, text: &str, provider: bool) -> (tempfile::TempDir, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "composer.json",
        r#"{"name":"codisseu/resources-ui","require":{"laravel/framework":"^12.0"},"autoload":{"psr-4":{"App\\":"app/"}},"scripts":{"post-autoload-dump":"touch HOOK_EXECUTED"}}"#,
    );
    put(
        root,
        "resources/views/admin/dashboard.blade.php",
        "<h1>Our dashboard</h1>\n",
    );
    put(
        root,
        "config/weather.php",
        "<?php\nreturn ['forecast' => ['city' => 'Our city']];\n",
    );
    put(
        root,
        "lang/en/messages.php",
        "<?php\nreturn ['weather' => ['rain' => 'Our rain']];\n",
    );
    put(
        root,
        "lang/en.json",
        "{\"Our weather forecast\":\"Our translation\"}\n",
    );
    put(
        root,
        "routes/web.php",
        "<?php\nuse Illuminate\\Support\\Facades\\Route;\nuse App\\Http\\Controllers\\WeatherController;\nRoute::get('/weather', [WeatherController::class, 'index'])->name('weather.index');\n",
    );
    put(
        root,
        "app/Http/Controllers/WeatherController.php",
        "<?php\nnamespace App\\Http\\Controllers;\nclass WeatherController { public function index() {} public function inspect() {} protected function hidden() {} }\n",
    );
    put(root, "public/css/weather.css", "body { color: blue; }\n");
    put(root, name, text);
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.into()))
        .unwrap();
    service
        .dispatch(Command::OpenFile(root.join(name)))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    idle(&mut h);
    if provider {
        h.state_mut()
            .service
            .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
                language: Language::Php,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/../codisseu-language/tests/fixtures/laravel_resources_server.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    root.join("protocol.jsonl").to_string_lossy().into_owned(),
                ],
            }]))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::StartProviders)
            .unwrap();
        wait(&mut h, |a| {
            a.service
                .language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
                && !a.service.is_busy()
        });
    }
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}
fn select(h: &mut H, a: usize, b: usize) {
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection { anchor: a, head: b },
        })
        .unwrap();
    h.step();
    h.step();
}
fn text(h: &H) -> String {
    h.state().service.active_document().unwrap().text().into()
}
fn selected(h: &H) -> String {
    let service = &h.state().service;
    let range = service
        .view(service.active_view().unwrap())
        .unwrap()
        .selection()
        .range();
    service
        .active_document()
        .unwrap()
        .snapshot()
        .slice(range)
        .to_string()
}
fn stop(h: &mut H) {
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(h);
}

#[test]
fn php_and_blade_resource_completion_replaces_whole_names_and_undo_keeps_disk() {
    for (name, original, needle, suggestion, accept) in [
        (
            "views.php",
            "<?php\r\n// ação 😀\r\nview('admin.daOLD');",
            "admin.da",
            "admin.dashboard",
            Key::Enter,
        ),
        (
            "config.php",
            "<?php\nconfig('weather.foreOLD');",
            "weather.fore",
            "weather.forecast.city",
            Key::Tab,
        ),
        (
            "translation.php",
            "<?php\n__('messages.weather.raOLD');",
            "messages.weather.ra",
            "messages.weather.rain",
            Key::Enter,
        ),
        (
            "json.php",
            "<?php\n__('Our weather foreOLD');",
            "Our weather fore",
            "Our weather forecast",
            Key::Tab,
        ),
        (
            "route.php",
            "<?php\nroute('weather.inOLD');",
            "weather.in",
            "weather.index",
            Key::Tab,
        ),
        (
            "asset.php",
            "<?php\nasset('css/weaOLD');",
            "css/wea",
            "css/weather.css",
            Key::Enter,
        ),
        (
            "page.blade.php",
            "{{-- ação 😀 --}}\r\n@include('admin.daOLD')",
            "admin.da",
            "admin.dashboard",
            Key::Tab,
        ),
        (
            "controller.php",
            "<?php\nuse Illuminate\\Support\\Facades\\Route;\nuse App\\Http\\Controllers\\WeatherController;\nRoute::get('/x', [WeatherController::class, 'indOLD']);",
            "ind",
            "index",
            Key::Enter,
        ),
    ] {
        let (dir, mut h) = fixture(name, original, true);
        let byte = original.find(needle).unwrap() + needle.len();
        let caret = original[..byte].chars().count();
        select(&mut h, caret, caret);
        key(&mut h, Modifiers::CTRL, Key::Space);
        wait(&mut h, |a| !a.service.completion.items.is_empty());
        let index = h
            .state()
            .service
            .completion
            .items
            .iter()
            .position(|i| i.label == suggestion)
            .unwrap_or_else(|| {
                panic!(
                    "missing {suggestion}: {:?}",
                    h.state().service.completion.items
                )
            });
        for _ in 0..index {
            key(&mut h, Modifiers::NONE, Key::ArrowDown);
        }
        key(&mut h, Modifiers::NONE, accept);
        idle(&mut h);
        let expected = original.replacen(&format!("{needle}OLD"), suggestion, 1);
        assert_eq!(text(&h), expected, "{name}");
        assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
        assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), original);
        key(&mut h, Modifiers::CTRL, Key::Z);
        idle(&mut h);
        assert_eq!(text(&h), original, "Undo {name}");
        if name == "controller.php" {
            // Empty action prefix makes visibility observable: hidden would
            // otherwise be excluded merely because it does not start with ind.
            let start = original[..original.find("indOLD").unwrap()].chars().count();
            select(&mut h, start, start + "indOLD".chars().count());
            key(&mut h, Modifiers::NONE, Key::Backspace);
            key(&mut h, Modifiers::CTRL, Key::Space);
            wait(&mut h, |a| !a.service.completion.items.is_empty());
            let items = &h.state().service.completion.items;
            assert!(items.iter().any(|item| item.label == "index"));
            assert!(items.iter().any(|item| item.label == "inspect"));
            assert!(!items.iter().any(|item| item.label == "hidden"));
            key(&mut h, Modifiers::NONE, Key::Escape);
            key(&mut h, Modifiers::CTRL, Key::Z);
            idle(&mut h);
            assert_eq!(text(&h), original);
        }
        assert!(!dir.path().join("HOOK_EXECUTED").exists());
        stop(&mut h);
    }
}

#[test]
fn definition_f12_ctrl_b_and_history_use_resource_targets_without_save() {
    for (original, needle, target, target_name, shortcut) in [
        (
            "<?php\nview('admin.dashboard');",
            "admin.dashboard",
            "resources/views/admin/dashboard.blade.php",
            "",
            Key::F12,
        ),
        (
            "<?php\nconfig('weather.forecast.city');",
            "weather.forecast.city",
            "config/weather.php",
            "city",
            Key::B,
        ),
        (
            "<?php\nroute('weather.index');",
            "weather.index",
            "routes/web.php",
            "weather.index",
            Key::F12,
        ),
        (
            "<?php\nuse Illuminate\\Support\\Facades\\Route;\nuse App\\Http\\Controllers\\WeatherController;\nRoute::get('/', [WeatherController::class, 'index']);",
            "index",
            "app/Http/Controllers/WeatherController.php",
            "index",
            Key::B,
        ),
    ] {
        let (dir, mut h) = fixture("source.php", original, true);
        let at = original[..original.find(needle).unwrap() + 2]
            .chars()
            .count();
        select(&mut h, at, at);
        key(
            &mut h,
            if shortcut == Key::B {
                Modifiers::CTRL
            } else {
                Modifiers::NONE
            },
            shortcut,
        );
        wait(&mut h, |a| {
            a.service
                .active_document()
                .is_some_and(|d| d.path == dir.path().join(target))
                && !a.service.is_busy()
        });
        let target_text = fs::read_to_string(dir.path().join(target)).unwrap();
        let target_prefix = &target_text[..target_text.find(target_name).unwrap()];
        assert_eq!(
            h.state().cursor,
            (
                target_prefix.chars().filter(|c| *c == '\n').count() + 1,
                target_prefix.rsplit('\n').next().unwrap().chars().count() + 1,
            ),
            "exact declaration position for {target}"
        );
        assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::ArrowLeft);
        idle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().path,
            dir.path().join("source.php")
        );
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            Selection::caret(at)
        );
        assert_eq!(text(&h), original);
        assert_eq!(
            fs::read_to_string(dir.path().join("source.php")).unwrap(),
            original
        );
        stop(&mut h);
    }
}

fn text_point(shape: &egui::Shape, line: &str, column: usize) -> Option<egui::Pos2> {
    match shape {
        egui::Shape::Text(t) if t.galley.job.text == line => {
            let cursor = t.galley.pos_from_cursor(egui::text::CCursor::new(column));
            Some(t.pos + cursor.center().to_vec2())
        }
        egui::Shape::Vec(v) => v.iter().find_map(|s| text_point(s, line, column)),
        _ => None,
    }
}
#[test]
fn ctrl_click_hits_the_painted_resource_and_back_restores_clicked_position() {
    let original = "<?php\nview('admin.dashboard');\n";
    let (dir, mut h) = fixture("source.php", original, true);
    let line = "view('admin.dashboard');";
    let column = line.find("dashboard").unwrap() + 2;
    let point = h
        .output()
        .shapes
        .iter()
        .find_map(|s| text_point(&s.shape, line, column))
        .expect("painted source line");
    h.hover_at(point);
    h.step();
    let modifiers = Modifiers::CTRL.plus(Modifiers::COMMAND);
    h.event(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
        h.step();
    }
    wait(&mut h, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path == dir.path().join("resources/views/admin/dashboard.blade.php"))
            && !a.service.is_busy()
    });
    key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::ArrowLeft);
    idle(&mut h);
    assert_eq!(text(&h), original);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("source.php")
    );
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        Selection::caret(6 + column)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("source.php")).unwrap(),
        original
    );
    stop(&mut h);
}

#[test]
fn new_php_and_blade_templates_tab_reverse_edit_escape_and_undo_keep_disk() {
    for (name, original, first, replacement, second) in [
        (
            "plain.php",
            "<?php\r\n// ação 😀\r\npubf",
            "method",
            "forecast",
            "",
        ),
        ("job.php", "<?php\nljob", "ExampleJob", "ForecastJob", ""),
        (
            "card.blade.php",
            "😀\r\n@props",
            "title",
            "forecast",
            "null",
        ),
        (
            "vite.blade.php",
            "@vite",
            "resources/css/app.css",
            "resources/css/weather.css",
            "resources/js/app.js",
        ),
    ] {
        let (dir, mut h) = fixture(name, original, false);
        select(&mut h, original.chars().count(), original.chars().count());
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), first, "{name}");
        h.event(egui::Event::Text(replacement.into()));
        h.step();
        h.step();
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), second, "{name}");
        if name != "job.php" {
            key(&mut h, Modifiers::SHIFT, Key::Tab);
            assert_eq!(selected(&h), replacement, "{name}");
        }
        let expanded = text(&h);
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert_eq!(text(&h), expanded);
        key(&mut h, Modifiers::CTRL, Key::Z);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(text(&h), original, "{name}");
        assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), original);
    }
}

#[test]
fn ctrl_j_partial_prefix_popup_can_cancel_and_insert_new_templates() {
    for (name, original, label, first, accept) in [
        (
            "source.php",
            "<?php\nlrequ",
            "lrequest · Laravel form request",
            "ExampleRequest",
            Key::Enter,
        ),
        (
            "card.blade.php",
            "@pro",
            "@props · Blade component props",
            "title",
            Key::Tab,
        ),
    ] {
        let (dir, mut h) = fixture(name, original, false);
        select(&mut h, original.chars().count(), original.chars().count());
        key(&mut h, Modifiers::CTRL, Key::J);
        assert!(
            h.query_by_role_and_label(Role::ListBoxOption, label)
                .is_some()
        );
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert!(
            h.query_by_role_and_label(Role::ListBoxOption, label)
                .is_none()
        );
        assert_eq!(text(&h), original);
        key(&mut h, Modifiers::CTRL, Key::J);
        key(&mut h, Modifiers::NONE, accept);
        assert_eq!(selected(&h), first);
        assert!(!text(&h).contains("${"));
        key(&mut h, Modifiers::NONE, Key::Escape);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(text(&h), original);
        assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), original);
    }
}

fn advanced_files(root: &Path) {
    put(
        root,
        "composer.json",
        r#"{"require":{"laravel/framework":"^12","livewire/livewire":"^3","inertiajs/inertia-laravel":"^2"},"autoload":{"psr-4":{"App\\":"app/"}}}"#,
    );
    put(
        root,
        "vendor/composer/installed.json",
        r#"{"packages":[{"name":"laravel/framework","install-path":"../laravel/framework","autoload":{"psr-4":{"Illuminate\\":"src/Illuminate/"}}}]}"#,
    );
    put(
        root,
        "vendor/laravel/framework/src/Illuminate/Foundation/Http/FormRequest.php",
        "<?php namespace Illuminate\\Foundation\\Http; class FormRequest extends \\Illuminate\\Http\\Request {}",
    );
    put(
        root,
        "vendor/laravel/framework/src/Illuminate/Http/Request.php",
        "<?php namespace Illuminate\\Http; class Request {}",
    );
    put(
        root,
        "app/Http/Requests/StoreRequest.php",
        "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest { public function rules() { return ['email'=>'required']; } }",
    );
    put(
        root,
        "resources/views/components/ui/alert.blade.php",
        "@props(['message'])\n<aside>{{ $heading }} {{ $slot }}</aside>",
    );
    put(
        root,
        "app/Livewire/EditPost.php",
        "<?php namespace App\\Livewire; class EditPost extends \\Livewire\\Component { public string $title; public function save() {} public function render() { return view('livewire.edit-post'); } }",
    );
    put(
        root,
        "resources/views/livewire/edit-post.blade.php",
        "<input wire:model='title'>",
    );
    put(
        root,
        "resources/js/Pages/Users/Show.vue",
        "<template><h1>Own page</h1></template>",
    );
    // Small own sources isolate static inference from provider answers.
    put(
        root,
        "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php",
        "<?php namespace Illuminate\\Database\\Eloquent; class Model { function __call($name,$args){return $this->forwardCallTo($this->newQuery(),$name,$args);} function hasNamedScope($name){return method_exists($this,'scope'.$name);} function isScopeMethodWithAttribute($name){} function resolveCustomBuilderClass(){} function newEloquentBuilder($q){return new static::$builder($q);} }",
    );
    put(
        root,
        "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Builder.php",
        "<?php namespace Illuminate\\Database\\Eloquent; class Builder { public function where($a,$b){return $this;} }",
    );
    put(
        root,
        "vendor/laravel/framework/src/Illuminate/Database/Query/Builder.php",
        "<?php namespace Illuminate\\Database\\Query; class Builder {}",
    );
    put(
        root,
        "app/Models/Post.php",
        "<?php namespace App\\Models; class Post extends \\Illuminate\\Database\\Eloquent\\Model { protected $fillable = ['title']; public function scopePublished($query) { return $query; } }",
    );
}

#[test]
fn advanced_laravel_completion_and_definition_use_the_existing_editor_flow() {
    for (path, original, needle, suggestion, destination, declaration) in [
        (
            "probe.php",
            "<?php use App\\Http\\Requests\\StoreRequest; function save(StoreRequest $request) { $request->input('emOLD'); }",
            "em",
            "email",
            "app/Http/Requests/StoreRequest.php",
            "email",
        ),
        (
            "probe.php",
            "<?php use App\\Http\\Requests\\StoreRequest; function save(StoreRequest $request) { $request->emOLD; }",
            "em",
            "email",
            "app/Http/Requests/StoreRequest.php",
            "email",
        ),
        (
            "probe.php",
            "<?php use App\\Models\\Post; Post::pubOLD();",
            "pub",
            "published",
            "app/Models/Post.php",
            "scopePublished",
        ),
        (
            "probe.blade.php",
            "{{-- 😀 --}}\n<x-ui.alOLD/>",
            "ui.al",
            "ui.alert",
            "resources/views/components/ui/alert.blade.php",
            "",
        ),
        (
            "resources/views/livewire/edit-post.blade.php",
            "<input wire:model='tiOLD'>",
            "ti",
            "title",
            "app/Livewire/EditPost.php",
            "$title",
        ),
        (
            "resources/views/livewire/edit-post.blade.php",
            "<button wire:click='saOLD'>Save</button>",
            "sa",
            "save",
            "app/Livewire/EditPost.php",
            "save",
        ),
        (
            "probe.php",
            "<?php use Inertia\\Inertia; Inertia::render('Users/ShOLD');",
            "Users/Sh",
            "Users/Show",
            "resources/js/Pages/Users/Show.vue",
            "",
        ),
    ] {
        eprintln!("advanced scenario {suggestion}: {path}");
        let (dir, mut h) = fixture(path, original, true);
        advanced_files(dir.path());
        // This view is also the query source, so restore its on-disk snapshot.
        put(dir.path(), path, original);
        let start = original.find(&format!("{needle}OLD")).unwrap();
        let caret = original[..start + needle.len()].chars().count();
        select(&mut h, caret, caret);
        key(&mut h, Modifiers::CTRL, Key::Space);
        wait(&mut h, |a| !a.service.completion.items.is_empty());
        let index = h
            .state()
            .service
            .completion
            .items
            .iter()
            .position(|i| i.label == suggestion)
            .unwrap_or_else(|| {
                panic!(
                    "missing {suggestion}: {:?}",
                    h.state().service.completion.items
                )
            });
        for _ in 0..index {
            key(&mut h, Modifiers::NONE, Key::ArrowDown);
        }
        key(&mut h, Modifiers::NONE, Key::Enter);
        idle(&mut h);
        let completed = original.replacen(&format!("{needle}OLD"), suggestion, 1);
        assert_eq!(text(&h), completed, "{path}");
        assert_eq!(fs::read_to_string(dir.path().join(path)).unwrap(), original);
        let target_caret = completed[..start + suggestion.len().min(2)].chars().count();
        select(&mut h, target_caret, target_caret);
        key(&mut h, Modifiers::NONE, Key::F12);
        wait(&mut h, |a| {
            a.service
                .active_document()
                .is_some_and(|d| d.path == dir.path().join(destination))
                && !a.service.is_busy()
        });
        let target_text = fs::read_to_string(dir.path().join(destination)).unwrap();
        let before = &target_text[..target_text.find(declaration).unwrap()];
        assert_eq!(
            h.state().cursor,
            (
                before.chars().filter(|c| *c == '\n').count() + 1,
                before.rsplit('\n').next().unwrap().chars().count() + 1
            ),
            "{suggestion}"
        );
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::ArrowLeft);
        idle(&mut h);
        assert_eq!(text(&h), completed);
        key(&mut h, Modifiers::CTRL, Key::Z);
        idle(&mut h);
        assert_eq!(text(&h), original);
        assert!(!dir.path().join("HOOK_EXECUTED").exists());
        stop(&mut h);
    }
}

fn hover_resource(h: &mut H, line: &str, column: usize) -> egui::Pos2 {
    let point = h
        .output()
        .shapes
        .iter()
        .find_map(|s| text_point(&s.shape, line, column))
        .expect("painted PHP line");
    h.hover_at(point);
    h.event(egui::Event::ModifiersChanged(
        Modifiers::CTRL.plus(Modifiers::COMMAND),
    ));
    h.step();
    point
}

#[test]
fn ctrl_hover_resolves_a_link_without_changing_selection_tabs_history_or_disk() {
    let original = "<?php\nview('admin.dashboard');\n";
    let (dir, mut h) = fixture("source.php", original, true);
    let before = h.state().service.active_document().unwrap().selection();
    let id = h.state().service.active.unwrap();
    let point = hover_resource(&mut h, "view('admin.dashboard');", 15);
    wait(&mut h, |a| {
        a.service
            .navigation
            .preview
            .as_ref()
            .is_some_and(|p| !p.targets.is_empty())
    });
    assert_eq!(h.state().service.active, Some(id));
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        before
    );
    assert!(!h.state().service.navigation.can_back());
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    let built = h.state().editor_metrics().layouts_built;
    for dx in [0.5, 1., 2., 0., 1.] {
        h.hover_at(point + egui::vec2(dx, 0.));
        h.step();
    }
    assert!(h.state().service.navigation.preview.is_some());
    let log = fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    let queries = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["method"] == "textDocument/definition")
        .count();
    assert_eq!(
        queries, 1,
        "stationary symbol hover must reuse its one result"
    );
    assert!(h.state().editor_metrics().layouts_built <= built);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    assert!(h.state().service.navigation.preview.is_none());
    assert_ne!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("source.php")).unwrap(),
        original
    );
    stop(&mut h);
}

#[test]
fn ctrl_hover_on_punctuation_blank_rows_and_a_modal_does_not_resolve_old_links() {
    let original = "<?php\nview('admin.dashboard');\n\n";
    let (_dir, mut h) = fixture("source.php", original, true);
    hover_resource(&mut h, "view('admin.dashboard');", 14);
    wait(&mut h, |a| {
        a.service
            .navigation
            .preview
            .as_ref()
            .is_some_and(|p| !p.targets.is_empty())
    });
    // ')' and the blank area below the actual source have no lexical candidate.
    hover_resource(&mut h, "view('admin.dashboard');", 22);
    for _ in 0..20 {
        h.step();
    }
    assert!(h.state().service.navigation.preview.is_none());
    let rect = h.get_by_role(Role::MultilineTextInput).rect();
    h.hover_at(rect.center_bottom() - egui::vec2(0., 20.));
    for _ in 0..20 {
        h.step();
    }
    assert!(h.state().service.navigation.preview.is_none());
    hover_resource(&mut h, "view('admin.dashboard');", 14);
    wait(&mut h, |a| {
        a.service
            .navigation
            .preview
            .as_ref()
            .is_some_and(|p| !p.targets.is_empty())
    });
    // Existing project path modal owns the layer; it must clear speculative feedback.
    key(&mut h, Modifiers::CTRL, Key::O);
    h.step();
    h.step();
    assert!(h.state().service.navigation.preview.is_none());
    key(&mut h, Modifiers::NONE, Key::Escape);
    stop(&mut h);
}

#[test]
fn php_implementations_use_keyboard_and_ctrl_alt_click_with_an_explicit_target_choice() {
    for pointer in [false, true] {
        let original = "<?php\n$gateway->deliver();\n";
        let (dir, mut h) = fixture("source.php", original, false);
        let targets = [dir.path().join("First.php"), dir.path().join("Second.php")];
        for path in &targets {
            fs::write(
                path,
                "<?php class Service { public function deliver() {} }\n",
            )
            .unwrap();
        }
        h.state_mut()
            .service
            .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
                language: Language::Php,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/../codisseu-language/tests/fixtures/definition_server.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    "multi".into(),
                    dir.path().join("protocol.log").to_string_lossy().into(),
                    targets[0].to_string_lossy().into(),
                    targets[1].to_string_lossy().into(),
                ],
            }]))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::StartProviders)
            .unwrap();
        wait(&mut h, |a| {
            a.service
                .language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
                && !a.service.is_busy()
        });
        let modifiers = Modifiers::CTRL
            .plus(Modifiers::COMMAND)
            .plus(Modifiers::ALT);
        if pointer {
            let point = h
                .output()
                .shapes
                .iter()
                .find_map(|s| text_point(&s.shape, "$gateway->deliver();", 13))
                .unwrap();
            h.hover_at(point);
            h.event(egui::Event::ModifiersChanged(modifiers));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
                h.step();
            }
        } else {
            select(&mut h, 20, 20);
            h.get_by_role(Role::MultilineTextInput).focus();
            h.step();
            key(&mut h, modifiers, Key::B);
        }
        wait(&mut h, |a| a.service.navigation.choices.len() == 2);
        assert!(h.state().service.navigation.is_implementation());
        assert_eq!(h.state().service.documents.len(), 1);
        h.get_by_label("Escolher implementação");
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
        key(&mut h, Modifiers::NONE, Key::Enter);
        wait(&mut h, |a| {
            a.service.active_document().unwrap().path == targets[1] && !a.service.is_busy()
        });
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::ArrowLeft);
        idle(&mut h);
        assert_eq!(text(&h), original);
        assert_eq!(
            fs::read_to_string(dir.path().join("source.php")).unwrap(),
            original
        );
        let log = fs::read_to_string(dir.path().join("protocol.log")).unwrap();
        assert!(log.contains("textDocument/implementation"));
        assert!(!log.contains("textDocument/definition"));
        stop(&mut h);
    }
}

#[test]
#[ignore = "CPU benchmark: stationary PHP link/no modifiers; controlled provider query count, run alone"]
fn stationary_php_link_reuses_one_query_and_keeps_frame_cost_bounded() {
    let (dir, mut h) = fixture("source.php", "<?php\nview('admin.dashboard');\n", true);
    let point = h
        .output()
        .shapes
        .iter()
        .find_map(|s| text_point(&s.shape, "view('admin.dashboard');", 15))
        .unwrap();
    h.hover_at(point);
    h.step();
    let measure = |h: &mut H| {
        let mut frames = Vec::new();
        let mut editor = Vec::new();
        let mut layouts = 0;
        for _ in 0..120 {
            let start = Instant::now();
            h.step();
            frames.push(start.elapsed().as_micros());
            let metrics = h.state().editor_metrics();
            editor.push(metrics.frame_micros);
            layouts += metrics.layouts_built;
        }
        frames.sort();
        editor.sort();
        serde_json::json!({"samples":120,"frame_us":{"p50":frames[60],"p95":frames[114],"max":frames[119]},"editor_us":{"p50":editor[60],"p95":editor[114],"max":editor[119]},"layouts_built":layouts})
    };
    for _ in 0..20 {
        h.step();
    }
    let ordinary = measure(&mut h);
    hover_resource(&mut h, "view('admin.dashboard');", 15);
    wait(&mut h, |a| {
        a.service
            .navigation
            .preview
            .as_ref()
            .is_some_and(|p| !p.targets.is_empty())
    });
    let linked = measure(&mut h);
    let log = fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    let queries = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["method"] == "textDocument/definition")
        .count();
    assert_eq!(queries, 1);
    assert_eq!(linked["layouts_built"], 0);
    eprintln!(
        "{}",
        serde_json::json!({"kind":"CPU Harness debug; no native GPU/FPS claim","fixture":"three PHP lines and owned Laravel resource, controlled provider","ordinary":ordinary,"resolved_link":linked,"definition_requests":queries})
    );
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    stop(&mut h);
}
