use super::*;
use std::fs;
fn put(root: &Path, file: &str, source: &str) -> PathBuf {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, source).unwrap();
    path
}
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    put(
        root,
        "composer.json",
        r#"{"require":{"laravel/framework":"^12","livewire/livewire":"^3","inertiajs/inertia-laravel":"^2"},"autoload":{"psr-4":{"App\\":"app/"}}}"#,
    );
    put(root, "app/Test.php", "<?php");
    put(root, "resources/views/test.blade.php", "");
    put(
        root,
        "resources/views/components/ui/alert.blade.php",
        "{{-- @props(['bad']) --}}\r\n@props(['alertType' => 'info', 'message'])\r\n<aside>{{ $heading }} {{ $message }} {{ $slot }}</aside>",
    );
    put(
        root,
        "app/View/Components/UserCard.php",
        "<?php namespace App\\View\\Components; use Illuminate\\View\\Component as Base; class UserCard extends Base { public function __construct(public string $userName) {} public function render() { return view('components.user-card'); } }",
    );
    put(
        root,
        "resources/views/components/user-card.blade.php",
        "<div>{{ $slot }}</div>",
    );
    put(
        root,
        "app/Livewire/EditPost.php",
        "<?php namespace App\\Livewire; use Livewire\\Component; class EditPost extends Component { public string $title; protected $secret; private function internal() {} public function save() {} public function render() { return view('livewire.edit-post'); } }",
    );
    put(
        root,
        "resources/views/livewire/edit-post.blade.php",
        "<input wire:model=\"title\"><button wire:click=\"save\">Save</button>",
    );
    put(
        root,
        "resources/js/Pages/Users/Show.vue",
        "<template><h1>user</h1></template>",
    );
    put(
        root,
        "resources/js/pages/dashboard.tsx",
        "export default () => <h1>dashboard</h1>;",
    );
    temp
}
fn run(
    root: &Path,
    path: &str,
    marked: &str,
    buffers: &[(PathBuf, &str)],
    completion: bool,
) -> Option<ResultSet> {
    let byte = marked.find('|').unwrap();
    let source = marked.replacen('|', "", 1);
    let (line, character) = position(&source, byte);
    query(
        root,
        &root.join(path),
        &source,
        line,
        character,
        &AtomicBool::new(false),
        buffers,
        completion,
    )
}
fn labels(result: ResultSet) -> Vec<String> {
    result.items.into_iter().map(|i| i.label).collect()
}
#[test]
fn blade_names_have_real_anonymous_or_class_targets() {
    let dir = fixture();
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "<x-ui.al|ert/>",
        &[],
        true,
    )
    .unwrap();
    assert_eq!(labels(result), ["ui.alert"]);
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "<x-user-ca|rd>",
        &[],
        false,
    )
    .unwrap();
    assert_eq!(result.targets.len(), 1);
    assert!(
        result.targets[0]
            .path
            .ends_with("app/View/Components/UserCard.php")
    );
}
#[test]
fn props_and_promoted_constructor_parameters_complete_and_navigate() {
    let dir = fixture();
    for (code, expected) in [
        ("<x-ui.alert mess|age='x'>", "message"),
        ("<x-user-card :user-n|ame='$name'>", "user-name"),
    ] {
        let result = run(
            dir.path(),
            "resources/views/test.blade.php",
            code,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(labels(result), [expected]);
        let result = run(
            dir.path(),
            "resources/views/test.blade.php",
            code,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(result.targets.len(), 1, "{code}");
    }
}
#[test]
fn nested_named_slots_use_nearest_parent_component() {
    let dir = fixture();
    for code in [
        "<x-ui.alert><x-slot:hea|ding>Hi</x-slot:heading></x-ui.alert>",
        "<x-ui.alert><x-slot name='hea|ding'>Hi</x-slot></x-ui.alert>",
    ] {
        let result = run(
            dir.path(),
            "resources/views/test.blade.php",
            code,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(labels(result), ["heading"]);
    }
}
#[test]
fn comments_raw_scripts_and_attribute_values_do_not_create_components() {
    let dir = fixture();
    for code in [
        "{{-- <x-ui.al|ert> --}}",
        "<!-- <x-ui.al|ert> -->",
        "<script>let a = '<x-ui.al|ert>';</script>",
        "<p title='<x-ui.al|ert>'>",
    ] {
        assert!(
            run(
                dir.path(),
                "resources/views/test.blade.php",
                code,
                &[],
                true
            )
            .is_none(),
            "{code}"
        );
    }
}
#[test]
fn livewire_names_directives_and_members_come_from_correct_class() {
    let dir = fixture();
    for code in ["<livewire:edit-po|st />", "@livewire('edit-po|st')"] {
        let result = run(
            dir.path(),
            "resources/views/test.blade.php",
            code,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(labels(result), ["edit-post"]);
    }
    for (code, expected) in [
        ("<input wire:model.live='ti|tle'>", "title"),
        ("<button wire:click='sa|ve(42)'>", "save"),
        ("<form wire:submit.prevent='sa|ve'>", "save"),
    ] {
        let result = run(
            dir.path(),
            "resources/views/livewire/edit-post.blade.php",
            code,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(labels(result), [expected]);
        let result = run(
            dir.path(),
            "resources/views/livewire/edit-post.blade.php",
            code,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(result.targets.len(), 1);
        assert!(
            result.targets[0]
                .path
                .ends_with("app/Livewire/EditPost.php")
        );
    }
    let result = run(
        dir.path(),
        "resources/views/livewire/edit-post.blade.php",
        "<input wire:model='se|cret'>",
        &[],
        true,
    )
    .unwrap();
    assert!(result.items.is_empty());
}
#[test]
fn unrelated_livewire_view_or_nested_model_path_is_not_invented() {
    let dir = fixture();
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "<input wire:model='ti|tle'>",
        &[],
        true,
    )
    .unwrap();
    assert!(result.items.is_empty());
    assert!(
        run(
            dir.path(),
            "resources/views/livewire/edit-post.blade.php",
            "<input wire:model='user.na|me'>",
            &[],
            true
        )
        .is_none()
    );
}
#[test]
fn inertia_facade_alias_helper_and_both_page_layouts() {
    let dir = fixture();
    for code in [
        "<?php use Inertia\\Inertia as I; I::render('Users/Sh|ow');",
        "<?php \\Inertia\\Inertia::render('Users/Sh|ow');",
        "<?php inertia('Users/Sh|ow');",
    ] {
        let result = run(dir.path(), "app/Test.php", code, &[], true).unwrap();
        assert_eq!(labels(result), ["Users/Show"]);
        let result = run(dir.path(), "app/Test.php", code, &[], false).unwrap();
        assert_eq!(result.targets.len(), 1);
        assert!(result.targets[0].path.ends_with("Pages/Users/Show.vue"));
    }
    assert_eq!(
        labels(
            run(
                dir.path(),
                "app/Test.php",
                "<?php inertia('dash|board');",
                &[],
                true
            )
            .unwrap()
        ),
        ["dashboard"]
    );
}
#[test]
fn inertia_negative_receivers_expressions_and_second_arguments() {
    let dir = fixture();
    for code in [
        "<?php Inertia::render('Users/Sh|ow');",
        "<?php use Other\\Inertia; Inertia::render('Users/Sh|ow');",
        "<?php inertia($page . 'Users/Sh|ow');",
        "<?php inertia('dashboard', 'Users/Sh|ow');",
        "<?php // inertia('Users/Sh|ow');",
    ] {
        assert!(
            run(dir.path(), "app/Test.php", code, &[], true).is_none(),
            "{code}"
        );
    }
}
#[test]
fn dirty_component_source_and_view_override_disk() {
    let dir = fixture();
    let path = dir.path().join("app/Livewire/EditPost.php");
    let dirty = "<?php namespace App\\Livewire; use Livewire\\Component; class EditPost extends Component { public $revised; public function publish() {} public function render() { return view('livewire.edit-post'); } }";
    let buffers = [(path, dirty)];
    assert_eq!(
        labels(
            run(
                dir.path(),
                "resources/views/livewire/edit-post.blade.php",
                "<input wire:model='re|vised'>",
                &buffers,
                true
            )
            .unwrap()
        ),
        ["revised"]
    );
    assert!(
        run(
            dir.path(),
            "resources/views/livewire/edit-post.blade.php",
            "<button wire:click='sa|ve'>",
            &buffers,
            true
        )
        .unwrap()
        .items
        .is_empty()
    );
    let view = dir
        .path()
        .join("resources/views/components/ui/alert.blade.php");
    let buffers = [(view, "@props(['unsaved' => true]) {{ $slot }}")];
    assert_eq!(
        labels(
            run(
                dir.path(),
                "resources/views/test.blade.php",
                "<x-ui.alert un|saved='yes'>",
                &buffers,
                true
            )
            .unwrap()
        ),
        ["unsaved"]
    );
}
#[test]
fn package_gates_accept_installed_metadata_without_running_autoload() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "composer.json",
        r#"{"require":{"laravel/framework":"^12"}}"#,
    );
    assert!(
        run(
            root,
            "app/Test.php",
            "<?php inertia('Users/Sh|ow');",
            &[],
            true
        )
        .is_none()
    );
    assert!(
        run(
            root,
            "resources/views/test.blade.php",
            "<livewire:edit-po|st />",
            &[],
            true
        )
        .is_none()
    );
    put(
        root,
        "vendor/composer/installed.json",
        r#"{"packages":[{"name":"livewire/livewire"},{"name":"inertiajs/inertia-laravel"}]}"#,
    );
    assert_eq!(
        labels(
            run(
                root,
                "app/Test.php",
                "<?php inertia('Users/Sh|ow');",
                &[],
                true
            )
            .unwrap()
        ),
        ["Users/Show"]
    );
    assert_eq!(
        labels(
            run(
                root,
                "resources/views/test.blade.php",
                "<livewire:edit-po|st />",
                &[],
                true
            )
            .unwrap()
        ),
        ["edit-post"]
    );
}
#[test]
fn no_laravel_and_cancelled_queries_do_not_resolve() {
    let dir = fixture();
    let root = dir.path();
    assert!(
        query(
            root,
            &root.join("app/Test.php"),
            "<?php inertia('')",
            0,
            15,
            &AtomicBool::new(true),
            &[],
            true
        )
        .is_none()
    );
    put(
        root,
        "composer.json",
        r#"{"require":{"other/framework":"1"}}"#,
    );
    assert!(
        run(
            root,
            "resources/views/test.blade.php",
            "<x-ui.al|ert>",
            &[],
            true
        )
        .is_none()
    );
}
#[test]
fn unicode_crlf_ranges_replace_complete_identifier() {
    let dir = fixture();
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "🌧\r\n<x-ui.al|ert/>",
        &[],
        true,
    )
    .unwrap();
    let edit = result.items[0].text_edit.as_ref().unwrap();
    assert_eq!(
        (
            edit.start_line,
            edit.start_character,
            edit.end_line,
            edit.end_character
        ),
        (1, 3, 1, 11)
    );
    let result = run(
        dir.path(),
        "app/Test.php",
        "<?php /* 🌧 */ inertia('Users/Sh|ow');",
        &[],
        true,
    )
    .unwrap();
    let edit = result.items[0].text_edit.as_ref().unwrap();
    assert_eq!(edit.start_character, 24);
    assert_eq!(edit.end_character, 34);
}
#[cfg(unix)]
#[test]
fn symlinked_components_and_page_directories_are_not_followed() {
    let dir = fixture();
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "hidden.blade.php", "@props(['secret'])");
    std::os::unix::fs::symlink(
        outside.path(),
        dir.path().join("resources/views/components/external"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("hidden.blade.php"),
        dir.path()
            .join("resources/views/components/linked.blade.php"),
    )
    .unwrap();
    let names = labels(
        run(
            dir.path(),
            "resources/views/test.blade.php",
            "<x-|>",
            &[],
            true,
        )
        .unwrap(),
    );
    assert!(
        !names
            .iter()
            .any(|n| n.contains("external") || n.contains("linked"))
    );
}
#[test]
fn definition_refuses_truncated_inventory() {
    let dir = fixture();
    for number in 0..520 {
        put(
            dir.path(),
            &format!("resources/js/Pages/generated{number}.tsx"),
            "",
        );
    }
    let result = run(
        dir.path(),
        "app/Test.php",
        "<?php inertia('Users/Sh|ow');",
        &[],
        false,
    )
    .unwrap();
    assert!(result.truncated);
    assert!(result.targets.is_empty());
}

#[test]
fn unfinished_inertia_literals_complete_without_writing_source() {
    let dir = fixture();
    for source in [
        "<?php inertia('Users/Sh|",
        "<?php use Inertia\\Inertia as I; I::render(\"Users/Sh|",
    ] {
        let result = run(dir.path(), "app/Test.php", source, &[], true).unwrap();
        assert_eq!(labels(result), ["Users/Show"]);
    }
}
#[test]
fn blade_echo_and_php_strings_do_not_become_component_tags() {
    let dir = fixture();
    for source in [
        "{{ '<x-ui.al|ert>' }}",
        "{!! '<x-ui.al|ert>' !!}",
        "<?php echo '<x-ui.al|ert>'; ?>",
        "@php $x = '<x-ui.al|ert>'; @endphp",
    ] {
        assert!(
            run(
                dir.path(),
                "resources/views/test.blade.php",
                source,
                &[],
                true
            )
            .is_none(),
            "{source}"
        );
    }
}
#[test]
fn property_and_props_definitions_have_actual_unicode_ranges() {
    let dir = fixture();
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "<x-ui.alert mess|age='hello'>",
        &[],
        false,
    )
    .unwrap();
    let t = &result.targets[0];
    let text = fs::read_to_string(&t.path).unwrap();
    let start = crate::utf16_byte_offset(&text, t.start_line, t.start_character).unwrap();
    let end = crate::utf16_byte_offset(&text, t.end_line, t.end_character).unwrap();
    assert_eq!(&text[start..end], "message");
    let result = run(
        dir.path(),
        "resources/views/livewire/edit-post.blade.php",
        "<input wire:model='ti|tle'>",
        &[],
        false,
    )
    .unwrap();
    let t = &result.targets[0];
    let text = fs::read_to_string(&t.path).unwrap();
    let start = crate::utf16_byte_offset(&text, t.start_line, t.start_character).unwrap();
    let end = crate::utf16_byte_offset(&text, t.end_line, t.end_character).unwrap();
    assert_eq!(&text[start..end], "$title");
}
#[test]
fn shared_view_keeps_all_livewire_definition_destinations() {
    let dir = fixture();
    put(
        dir.path(),
        "app/Livewire/OtherPost.php",
        "<?php namespace App\\Livewire; use Livewire\\Component; class OtherPost extends Component { public $title; public function render() { return view('livewire.edit-post'); } }",
    );
    let result = run(
        dir.path(),
        "resources/views/livewire/edit-post.blade.php",
        "<input wire:model='ti|tle'>",
        &[],
        false,
    )
    .unwrap();
    assert_eq!(result.targets.len(), 2);
    assert_ne!(result.targets[0].path, result.targets[1].path);
}
#[test]
fn unknown_parent_traits_and_bad_namespaces_do_not_invent_livewire() {
    let dir = fixture();
    for (file, source) in [
        (
            "app/Livewire/Fake.php",
            "<?php namespace App\\Livewire; use Other\\Component; class Fake extends Component {}",
        ),
        (
            "app/Livewire/WithTrait.php",
            "<?php namespace App\\Livewire; use Livewire\\Component; class WithTrait extends Component { use Hidden; }",
        ),
        (
            "app/Livewire/Alien.php",
            "<?php namespace Other; use Livewire\\Component; class Alien extends Component {}",
        ),
    ] {
        put(dir.path(), file, source);
    }
    let result = run(
        dir.path(),
        "resources/views/test.blade.php",
        "<livewire:|>",
        &[],
        true,
    )
    .unwrap();
    assert_eq!(labels(result), ["edit-post"]);
}
#[test]
fn nested_composer_owns_its_own_package_gates() {
    let dir = fixture();
    put(
        dir.path(),
        "nested/composer.json",
        r#"{"require":{"laravel/framework":"^12"}}"#,
    );
    put(dir.path(), "nested/app/Test.php", "<?php");
    assert!(
        run(
            dir.path(),
            "nested/app/Test.php",
            "<?php inertia('Users/Sh|ow');",
            &[],
            true
        )
        .is_none()
    );
}
