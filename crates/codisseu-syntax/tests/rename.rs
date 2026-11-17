use codisseu_syntax::*;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn range(text: &str, start: usize, end: usize) -> TextRange {
    let pos = |n| {
        let p = &text[..n];
        (
            p.bytes().filter(|b| *b == b'\n').count() as u32,
            p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
        )
    };
    let (a, b) = pos(start);
    let (c, d) = pos(end);
    TextRange {
        start_line: a,
        start_character: b,
        end_line: c,
        end_character: d,
    }
}
fn snapshot(path: &str, text: &str, old: &str, new: &str) -> RenameSnapshot {
    let offsets = text.match_indices(old).map(|(n, _)| n).collect::<Vec<_>>();
    selected(path, text, old, new, &offsets)
}
fn selected(path: &str, text: &str, old: &str, new: &str, offsets: &[usize]) -> RenameSnapshot {
    let edits = offsets
        .iter()
        .map(|&a| RenameEdit {
            range: range(text, a, a + old.len()),
            new_text: new.into(),
        })
        .collect();
    let mut after = text.to_owned();
    for &a in offsets.iter().rev() {
        after.replace_range(a..a + old.len(), new);
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(after),
        edits,
    }
}
fn check(s: RenameSnapshot, name: &str) -> RenameValidation {
    validate_method_rename(&[s], name, &AtomicBool::new(false))
}
fn snippet<'a>(text: &'a str, r: &TextRange) -> &'a str {
    fn b(text: &str, line: u32, col: u32) -> usize {
        let mut n = 0;
        for _ in 0..line {
            n += text[n..].find('\n').unwrap() + 1;
        }
        let mut u = 0;
        for (i, c) in text[n..].char_indices() {
            if u == col {
                return n + i;
            }
            u += c.len_utf16() as u32;
        }
        text.len()
    }
    &text[b(text, r.start_line, r.start_character)..b(text, r.end_line, r.end_character)]
}
#[test]
fn php_duplicates_ignore_ascii_case_and_report_existing_and_renamed_before_ranges() {
    let src = "<?php\r\n/* 🌧 e\u{301} */ class Weather { public function rainfall(){} public function ForeCast(){} }\r\n$w->RainFall();";
    let mut s = snapshot("/p/Weather.php", src, "rainfall", "forecast");
    let call = src.find("RainFall").unwrap();
    s.edits.push(RenameEdit {
        range: range(src, call, call + 8),
        new_text: "forecast".into(),
    });
    s.after = Arc::from(s.after.replace("RainFall", "forecast"));
    let report = check(s, "forecast");
    assert_eq!(report.coverage, Coverage::Rejected, "{report:?}");
    assert_eq!(report.checked_declarations, 1);
    let c = &report.conflicts[0];
    assert_eq!(c.container_name.as_deref(), Some("Weather"));
    assert_eq!(snippet(src, &c.range), "ForeCast");
    assert_eq!(snippet(src, &c.related.as_ref().unwrap().range), "rainfall");
    assert_eq!(c.kind, ConflictKind::DuplicateMember);
}
#[test]
fn php_static_and_instance_share_method_namespace_but_properties_do_not() {
    for src in [
        "<?php class A { function rainfall(){} static function forecast(){} }",
        "<?php class A { static function rainfall(){} function forecast(){} }",
    ] {
        assert_eq!(
            check(snapshot("A.php", src, "rainfall", "forecast"), "forecast").coverage,
            Coverage::Rejected
        );
    }
    let src = "<?php class A { public $forecast; function rainfall(){} }";
    assert_eq!(
        check(snapshot("A.php", src, "rainfall", "forecast"), "forecast").coverage,
        Coverage::Verified
    );
}
#[test]
fn typescript_case_static_instance_and_class_boundaries_are_distinct() {
    for (src, expected) in [
        ("class A { rainfall(){} forecast(){} }", Coverage::Rejected),
        ("class A { rainfall(){} Forecast(){} }", Coverage::Verified),
        (
            "class A { rainfall(){} static forecast(){} }",
            Coverage::Verified,
        ),
        (
            "class A { static rainfall(){} forecast(){} }",
            Coverage::Verified,
        ),
        (
            "class A { static rainfall(){} static forecast(){} }",
            Coverage::Rejected,
        ),
        (
            "class A { rainfall(){} } class B { forecast(){} }",
            Coverage::Verified,
        ),
        ("class A { rainfall(){} forecast = 3; }", Coverage::Rejected),
    ] {
        let r = check(snapshot("A.ts", src, "rainfall", "forecast"), "forecast");
        assert_eq!(r.coverage, expected, "{src}: {r:?}");
    }
}
#[test]
fn homonymous_comments_and_strings_are_not_members_and_edits_inside_them_are_unsupported() {
    for (path, src) in [
        (
            "A.ts",
            "class A { rainfall() { return 'forecast rainfall'; } /* forecast rainfall */ } a.rainfall();",
        ),
        (
            "A.php",
            "<?php class A { function rainfall() { return 'forecast rainfall'; } /* forecast rainfall */ } $a->rainfall();",
        ),
    ] {
        let positions = src
            .match_indices("rainfall")
            .map(|(n, _)| n)
            .collect::<Vec<_>>();
        let s = selected(
            path,
            src,
            "rainfall",
            "forecast",
            &[positions[0], *positions.last().unwrap()],
        );
        assert_eq!(check(s, "forecast").coverage, Coverage::Verified);
        let invalid = snapshot(path, src, "rainfall", "forecast");
        let r = check(invalid, "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported);
        assert!(
            r.issues
                .iter()
                .any(|i| i.kind == IssueKind::UnsupportedEdit)
        );
    }
}
#[test]
fn hierarchy_traits_interfaces_and_uncertain_typescript_members_fail_closed() {
    let php = [
        "<?php class A extends B { function rainfall(){} }",
        "<?php class A implements B { function rainfall(){} }",
        "<?php class A { use T; function rainfall(){} }",
        "<?php trait A { function rainfall(){} }",
        "<?php interface A { function rainfall(); }",
        "<?php $a=new class { function rainfall(){} };",
        "<?php enum A { function rainfall(){} }",
    ];
    for src in php {
        let r = check(snapshot("A.php", src, "rainfall", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
    let ts = [
        "class A extends B { rainfall(){} }",
        "class A implements B { rainfall(){} }",
        "class A { rainfall(){} ['x'](){} }",
        "class A { rainfall(){} 'x'(){} }",
        "class A { rainfall(){} x(a:string):string; x(a:any){return a;} }",
        "class A { rainfall(){} get x(){return 1;} }",
        "class A { constructor(public forecast:string){} rainfall(){} }",
        "class A { rainfall(){} #x=1; }",
    ];
    for src in ts {
        let r = check(snapshot("A.ts", src, "rainfall", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
}
#[test]
fn syntax_before_is_unsupported_and_introduced_syntax_is_rejected() {
    let r = check(
        snapshot("A.ts", "class A { rainfall() { ", "rainfall", "forecast"),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Unsupported);
    assert!(r.issues.iter().any(|i| i.kind == IssueKind::SyntaxBefore));
    let r = check(
        snapshot("A.ts", "class A { rainfall(){} }", "class", "break"),
        "break",
    );
    assert_eq!(r.coverage, Coverage::Rejected);
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::InvalidSyntax)
    );
}
#[test]
fn wrong_after_text_surrogate_half_and_nonmethod_bindings_never_verify() {
    let mut s = snapshot(
        "A.ts",
        "/* 😀 */ class A { rainfall(){} }",
        "rainfall",
        "forecast",
    );
    s.after = Arc::from(format!("{} ", s.after));
    assert_eq!(check(s, "forecast").coverage, Coverage::Unsupported);
    let mut s = snapshot(
        "A.ts",
        "/* 😀 */ class A { rainfall(){} }",
        "rainfall",
        "forecast",
    );
    s.edits[0].range.start_character = 4;
    assert_eq!(check(s, "forecast").coverage, Coverage::Unsupported);
    for (path, src) in [
        ("A.ts", "function rainfall(){} rainfall();"),
        ("A.php", "<?php function rainfall(){} rainfall();"),
        ("A.ts", "const obj={rainfall(){}}; obj.rainfall();"),
        ("A.ts", "class A { foo(){} } obj.rainfall();"),
    ] {
        assert_eq!(
            check(snapshot(path, src, "rainfall", "forecast"), "forecast").coverage,
            Coverage::Unsupported,
            "{src}"
        );
    }
}
#[test]
fn declaration_and_uses_in_separate_dirty_snapshots_and_context_are_checked() {
    let a = snapshot(
        "Weather.ts",
        "// 🌧 e\u{301}\r\nclass Weather { rainfall(){} }\r\n",
        "rainfall",
        "forecast",
    );
    let b = snapshot(
        "main.ts",
        "// unsaved\nconsole.log('🌧 e\u{301}'); weather.rainfall();\n",
        "rainfall",
        "forecast",
    );
    let context = RenameSnapshot {
        path: "other.ts".into(),
        before: Arc::from("class Other { forecast(){} }"),
        after: Arc::from("class Other { forecast(){} }"),
        edits: vec![],
    };
    let r = validate_method_rename(&[a, b, context], "forecast", &AtomicBool::new(false));
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    assert_eq!(r.checked_files, 3);
    assert_eq!(r.incremental_reparses, 2);
    assert!(r.note.contains("fora dos snapshots"));
    assert_eq!(r.scope, ValidationScope::SameClassMemberNamespace);
}
#[test]
fn unsupported_extensions_magic_constructors_partial_names_and_preexisting_duplicates() {
    for (path, src, old, new) in [
        ("A.js", "class A { rainfall(){} }", "rainfall", "forecast"),
        (
            "A.ts",
            "class A { constructor(){} }",
            "constructor",
            "forecast",
        ),
        (
            "A.ts",
            "class A { rainfall(){} }",
            "rainfall",
            "constructor",
        ),
        (
            "A.php",
            "<?php class A { function rainfall(){} }",
            "rainfall",
            "__call",
        ),
        ("A.ts", "class A { rainfall(){} }", "rain", "snow"),
        (
            "A.php",
            "<?php class A { function rainfall(){} function DUP(){} function dup(){} }",
            "rainfall",
            "forecast",
        ),
    ] {
        let r = check(snapshot(path, src, old, new), new);
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
}
#[test]
fn bounds_cancellation_zero_time_and_node_budget_never_become_verified() {
    let s = snapshot("A.ts", "class A { rainfall(){} }", "rainfall", "forecast");
    let c = AtomicBool::new(true);
    let r = validate_method_rename(std::slice::from_ref(&s), "forecast", &c);
    assert_eq!(r.coverage, Coverage::Unsupported);
    assert!(r.issues.iter().any(|i| i.kind == IssueKind::Cancelled));
    for limits in [
        ValidationLimits {
            max_nodes: 0,
            max_duration: Duration::from_secs(1),
        },
        ValidationLimits {
            max_nodes: 100,
            max_duration: Duration::ZERO,
        },
        ValidationLimits {
            max_nodes: 10,
            max_duration: Duration::from_secs(1),
        },
    ] {
        let r = validate_method_rename_with_limits(
            std::slice::from_ref(&s),
            "forecast",
            &AtomicBool::new(false),
            limits,
        );
        assert_eq!(r.coverage, Coverage::Unsupported);
        assert!(r.issues.iter().any(|i| i.kind == IssueKind::Limit));
    }
    assert_eq!(
        validate_method_rename(&vec![s.clone(); 65], "forecast", &AtomicBool::new(false)).coverage,
        Coverage::Unsupported
    );
    let huge = RenameSnapshot {
        path: "huge.ts".into(),
        before: Arc::from(" ".repeat(1024 * 1024 + 1)),
        after: Arc::from(""),
        edits: vec![],
    };
    assert_eq!(check(huge, "forecast").coverage, Coverage::Unsupported);
}

#[test]
fn implicit_static_members_merging_and_promoted_constructor_properties_are_not_certified() {
    for new in ["prototype", "name", "length", "caller", "arguments"] {
        let s = snapshot("A.ts", "class A { static rainfall(){} }", "rainfall", new);
        assert_eq!(check(s, new).coverage, Coverage::Unsupported);
    }
    for src in [
        "class A { rainfall(){} } interface A { x():void; }",
        "class A { rainfall(){} } namespace A { export const x=1; }",
        "class A { constructor(readonly forecast:string){} rainfall(){} }",
        "class A { @decorator rainfall(){} }",
    ] {
        let r = check(snapshot("A.ts", src, "rainfall", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
}

fn context(path: &str, text: &str) -> RenameSnapshot {
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(text),
        edits: vec![],
    }
}
fn corpus(files: Vec<RenameSnapshot>) -> RenameValidation {
    validate_method_rename(&files, "forecast", &AtomicBool::new(false))
}
#[test]
fn resolved_cross_file_php_namespace_and_import_alias_allow_independent_method() {
    for child in [
        "<?php namespace App; use Vendor\\Base as ParentClass; class Child extends ParentClass { function rainfall(){} }",
        "<?php namespace App { use Vendor\\Base as ParentClass; class Child extends ParentClass { function rainfall(){} } }",
        "<?php namespace App; class Child extends \\Vendor\\Base { function rainfall(){} }",
    ] {
        let r = corpus(vec![
            context(
                "/p/Base.php",
                "<?php namespace Vendor; class Base { function other(){} }",
            ),
            snapshot("/p/Child.php", child, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Verified, "{child}: {r:?}");
        assert_eq!(r.scope, ValidationScope::KnownHierarchyMemberNamespaces);
        assert!(r.note.contains("corpus"));
    }
    let r = corpus(vec![
        context("/p/Grand.php", "<?php namespace App; class Grand {}"),
        context(
            "/p/Base.php",
            "<?php namespace App; class Base extends namespace\\Grand {}",
        ),
        snapshot(
            "/p/Child.php",
            "<?php namespace App; use App as Local; class Child extends Local\\Base { function rainfall(){} }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn php_known_ancestor_and_descendant_names_block_with_real_before_locations() {
    let child =
        "<?php namespace App; /* 🌧 e\u{301} */ class Child extends Base { function rainfall(){} }";
    let base = "<?php namespace App; class Base { function ForeCast(){} }";
    let r = corpus(vec![
        context("/p/Base.php", base),
        snapshot("/p/Child.php", child, "rainfall", "forecast"),
    ]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    let c = r
        .conflicts
        .iter()
        .find(|c| c.kind == ConflictKind::InheritedMember)
        .unwrap();
    assert_eq!(c.path.to_str(), Some("/p/Base.php"));
    assert_eq!(snippet(base, &c.range), "ForeCast");
    assert_eq!(c.container_name.as_deref(), Some("Base"));
    assert_eq!(
        snippet(child, &c.related.as_ref().unwrap().range),
        "rainfall"
    );
    let r = corpus(vec![
        snapshot(
            "/p/Base.php",
            "<?php class Base { function rainfall(){} }",
            "rainfall",
            "forecast",
        ),
        context(
            "/p/Child.php",
            "<?php class Child extends Base { static function FORECAST(){} }",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert_eq!(r.conflicts[0].container_name.as_deref(), Some("Child"));
}
#[test]
fn ts_relative_named_import_alias_and_same_names_in_distinct_modules_resolve() {
    for import in ["./base", "./base.ts", "./base.js"] {
        let r = corpus(vec![
            context("/p/base.ts", "export class Base { other(){} }"),
            context("/p/unrelated.ts", "export class Base { forecast(){} }"),
            snapshot(
                "/p/child.ts",
                &format!(
                    "import {{ Base as Parent }} from '{import}'; export class Child extends Parent {{ rainfall(){{}} }}"
                ),
                "rainfall",
                "forecast",
            ),
        ]);
        assert_eq!(r.coverage, Coverage::Verified, "{import}: {r:?}");
        assert_eq!(r.scope, ValidationScope::KnownHierarchyMemberNamespaces);
    }
    let r = corpus(vec![
        context("/p/base/index.ts", "export class Base {}"),
        snapshot(
            "/p/child.ts",
            "import { Base } from './base'; class Child extends Base { rainfall(){} }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn ts_inheritance_collision_respects_case_and_static_instance_namespaces() {
    for (base, child, expected) in [
        ("forecast(){}", "rainfall(){}", Coverage::Rejected),
        ("Forecast(){}", "rainfall(){}", Coverage::Verified),
        ("static forecast(){}", "rainfall(){}", Coverage::Verified),
        (
            "static forecast(){}",
            "static rainfall(){}",
            Coverage::Rejected,
        ),
        ("forecast=1;", "rainfall(){}", Coverage::Rejected),
    ] {
        let r = corpus(vec![
            context("base.ts", &format!("export class Base {{ {base} }}")),
            snapshot(
                "child.ts",
                &format!("import {{ Base }} from './base'; class Child extends Base {{ {child} }}"),
                "rainfall",
                "forecast",
            ),
        ]);
        assert_eq!(r.coverage, expected, "{base}/{child}: {r:?}");
    }
}
#[test]
fn siblings_do_not_collide_and_unedited_descendant_is_still_examined() {
    let r = corpus(vec![
        context("base.ts", "export class Base {}"),
        snapshot(
            "left.ts",
            "import { Base } from './base'; class Left extends Base { rainfall(){} }",
            "rainfall",
            "forecast",
        ),
        context(
            "right.ts",
            "import { Base } from './base'; class Right extends Base { forecast(){} }",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = corpus(vec![
        snapshot(
            "base.ts",
            "export class Base { rainfall(){} }",
            "rainfall",
            "forecast",
        ),
        context(
            "leaf.ts",
            "import { Base } from './base'; class Leaf extends Base { forecast(){} }",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert_eq!(r.conflicts[0].path.to_str(), Some("leaf.ts"));
}
#[test]
fn partial_override_rename_rejected_and_complete_override_not_falsely_certified() {
    for (a, b) in [
        (
            context("A.ts", "class A { rainfall(){} }"),
            snapshot(
                "B.ts",
                "class B extends A { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ),
        (
            snapshot("A.ts", "class A { rainfall(){} }", "rainfall", "forecast"),
            context("B.ts", "class B extends A { rainfall(){} }"),
        ),
    ] {
        let r = corpus(vec![a, b]);
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::OverrideChanged)
        );
    }
    for files in [
        vec![
            snapshot("A.ts", "class A { rainfall(){} }", "rainfall", "forecast"),
            snapshot(
                "B.ts",
                "class B extends A { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            snapshot(
                "A.php",
                "<?php class A { function rainfall(){} }",
                "rainfall",
                "forecast",
            ),
            snapshot(
                "B.php",
                "<?php class B extends A { function RAINFALL(){} }",
                "RAINFALL",
                "forecast",
            ),
        ],
    ] {
        let r = corpus(files);
        assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
        assert!(
            r.issues
                .iter()
                .any(|i| i.message.contains("Retorno explícito"))
        );
    }
}
#[test]
fn missing_ambiguous_cyclic_dynamic_or_generic_heritage_blocks() {
    let cases = [
        vec![snapshot(
            "a.ts",
            "class A extends Missing { rainfall(){} }",
            "rainfall",
            "forecast",
        )],
        vec![snapshot(
            "a.ts",
            "class A extends B { rainfall(){} } class B extends A {}",
            "rainfall",
            "forecast",
        )],
        vec![
            context("base.ts", "export class Base {}"),
            context("base/index.ts", "export class Base {}"),
            snapshot(
                "a.ts",
                "import { Base } from './base'; class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base {}"),
            snapshot(
                "a.ts",
                "import type { Base } from './base'; class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base {}"),
            snapshot(
                "a.ts",
                "import Base from './base'; class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![snapshot(
            "a.ts",
            "class A extends mixin(Base) { rainfall(){} }",
            "rainfall",
            "forecast",
        )],
        vec![snapshot(
            "a.ts",
            "class A<T> { rainfall(){} }",
            "rainfall",
            "forecast",
        )],
        vec![snapshot(
            "a.php",
            "<?php namespace App; use X\\{Base}; class A extends Base { function rainfall(){} }",
            "rainfall",
            "forecast",
        )],
        vec![snapshot(
            "a.php",
            "<?php class A extends B { function rainfall(){} } class B extends A {}",
            "rainfall",
            "forecast",
        )],
        vec![snapshot(
            "a.php",
            "<?php class A { function rainfall(){} } class_alias(A::class,'B');",
            "rainfall",
            "forecast",
        )],
    ];
    for files in cases {
        let r = corpus(files);
        assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    }
}
#[test]
fn unresolved_unrelated_hierarchy_blocks_only_edited_language_and_context_errors_block() {
    let target = snapshot("a.ts", "class A { rainfall(){} }", "rainfall", "forecast");
    let r = corpus(vec![
        target.clone(),
        context("unrelated.ts", "class U extends Missing {}"),
    ]);
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    let r = corpus(vec![
        target.clone(),
        context("unrelated.php", "<?php class U extends Missing {}"),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = corpus(vec![target, context("bad.ts", "class U { ")]);
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
}
#[test]
fn uncertain_related_members_and_contracts_are_not_verified() {
    for base in [
        "class Base { ['forecast'](){} }",
        "class Base { get forecast(){return 1;} }",
        "class Base implements X {}",
    ] {
        let r = corpus(vec![
            context("base.ts", base),
            snapshot(
                "a.ts",
                "class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{base}: {r:?}");
    }
    for base in [
        "<?php class Base { private function forecast(){} }",
        "<?php class Base { final function forecast(){} }",
        "<?php class Base { use T; }",
    ] {
        let r = corpus(vec![
            context("base.php", base),
            snapshot(
                "a.php",
                "<?php class A extends Base { function rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{base}: {r:?}");
    }
}

#[test]
fn aliases_duplicate_identities_reexports_and_default_binding_ambiguities_block() {
    let cases = vec![
        vec![
            context("base.php", "<?php namespace Vendor; class Base {}"),
            snapshot(
                "a.php",
                "<?php use Vendor\\Base as P; use Elsewhere\\Base as P; class A extends P { function rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("b.php", "<?php class BASE {}"),
            context("c.php", "<?php class base {}"),
            snapshot(
                "a.php",
                "<?php class A extends Base { function rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base {}"),
            context("reexport.ts", "export { Base } from './base';"),
            snapshot(
                "a.ts",
                "import { Base } from './reexport'; class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base {}"),
            snapshot(
                "a.ts",
                "import Base from './base'; class Base {} class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base {}"),
            snapshot(
                "a.ts",
                "import * as Parent from './base'; class A extends Parent.Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "export class Base { x(){} x(){} }"),
            snapshot(
                "a.ts",
                "import { Base } from './base'; class A extends Base { rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
        vec![
            context("base.ts", "class Base { rainfall(){} }"),
            snapshot(
                "a.ts",
                "class A extends Base { override rainfall(){} }",
                "rainfall",
                "forecast",
            ),
        ],
    ];
    for files in cases {
        let r = corpus(files);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn php_class_names_are_folded_only_within_namespace_and_multiple_namespaces_stay_distinct() {
    let r = corpus(vec![
        context(
            "base.php",
            "<?php namespace Vendor; class BaSe {} namespace Other; class Base { function forecast(){} }",
        ),
        snapshot(
            "a.php",
            "<?php namespace App; use Vendor\\base as P; class Child extends p { function rainfall(){} }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn graph_traversal_obeys_budget_without_mutating_snapshots() {
    let a = snapshot(
        "a.ts",
        "class Root { rainfall(){} }",
        "rainfall",
        "forecast",
    );
    let mut input = vec![a];
    for n in 0..30 {
        input.push(context(
            &format!("child{n}.ts"),
            &format!(
                "class Child{n} extends {} {{}}",
                if n == 0 {
                    "Root".into()
                } else {
                    format!("Child{}", n - 1)
                }
            ),
        ));
    }
    let before = input
        .iter()
        .map(|s| (s.before.clone(), s.after.clone()))
        .collect::<Vec<_>>();
    let report = validate_method_rename_with_limits(
        &input,
        "forecast",
        &AtomicBool::new(false),
        ValidationLimits {
            max_nodes: 2000,
            max_duration: Duration::from_secs(1),
        },
    );
    assert_eq!(report.coverage, Coverage::Unsupported, "{report:?}");
    assert!(
        report.issues.iter().any(|i| i.kind == IssueKind::Limit),
        "{report:?}"
    );
    for (s, (before, after)) in input.iter().zip(before) {
        assert_eq!(s.before, before);
        assert_eq!(s.after, after);
    }
}

#[test]
fn php_case_only_rename_does_not_invent_broken_override() {
    let files = vec![
        context("base.php", "<?php class Base { function RAINFALL(){} }"),
        snapshot(
            "child.php",
            "<?php class Child extends Base { function rainfall(){} }",
            "rainfall",
            "RainFall",
        ),
    ];
    let r = validate_method_rename(&files, "RainFall", &AtomicBool::new(false));
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    assert!(
        !r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::OverrideChanged)
    );
}

#[test]
fn repeated_namespace_expansion_has_an_explicit_normalized_index_bound() {
    let mut source = format!("<?php namespace {}; ", "N".repeat(100_000));
    for n in 0..43 {
        source.push_str(&format!("class C{n} {{}} "));
    }
    source.push_str("class Target { function rainfall(){} }");
    let r = check(
        snapshot("large.php", &source, "rainfall", "forecast"),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    assert!(r.issues.iter().any(|i| i.kind == IssueKind::Limit), "{r:?}");
}

#[test]
fn complete_primitive_override_chain_is_verified_with_unique_pair_count() {
    for (ext, a, b, c) in [
        (
            "ts",
            "class A { rainfall(value: string): string { return value; } }",
            "class B extends A { override rainfall(value: string): string { return value; } }",
            "class C extends B { rainfall(value: string): string { return value; } }",
        ),
        (
            "php",
            "<?php class A { public function rainfall(string $value): string { return $value; } }",
            "<?php class B extends A { function rainfall(string $value): string { return $value; } }",
            "<?php class C extends B { public function rainfall(string $value): string { return $value; } }",
        ),
    ] {
        let r = corpus(vec![
            snapshot(&format!("A.{ext}"), a, "rainfall", "forecast"),
            snapshot(&format!("B.{ext}"), b, "rainfall", "forecast"),
            snapshot(&format!("C.{ext}"), c, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_override_pairs, 3);
        assert_eq!(r.checked_declarations, 3);
    }
}

#[test]
fn primitive_override_contract_rejects_uncertain_or_divergent_headers() {
    for (base, child) in [
        (
            "rainfall(value: string): string",
            "rainfall(value: number): string",
        ),
        (
            "rainfall(value: string): string",
            "rainfall(other: string): string",
        ),
        ("rainfall(): string", "rainfall(): number"),
        ("rainfall(): string", "protected rainfall(): string"),
        ("rainfall(): string", "rainfall()"),
        (
            "rainfall(value: string): string",
            "rainfall(value?: string): string",
        ),
        (
            "rainfall(value: string): string",
            "rainfall(value: string = 'rain'): string",
        ),
        (
            "rainfall(value: string): string",
            "rainfall(...value: string[]): string",
        ),
        ("rainfall(): string", "rainfall<T>(): string"),
        ("rainfall(): string", "async rainfall(): string"),
        ("rainfall(): string", "*rainfall(): string"),
        ("rainfall(): string", "rainfall(): string | number"),
        ("rainfall(): string", "rainfall(): Alias"),
        (
            "rainfall(value: string): string",
            "rainfall({value}: any): string",
        ),
        ("rainfall(): string", "private rainfall(): string"),
    ] {
        let a = format!("class A {{ {base} {{ return 'rain'; }} }}");
        let b = format!("class B extends A {{ {child} {{ return 'rain'; }} }}");
        let r = corpus(vec![
            snapshot("a.ts", &a, "rainfall", "forecast"),
            snapshot("b.ts", &b, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{a}\n{b}\n{r:?}");
        assert_eq!(r.checked_override_pairs, 0);
    }
    for child in [
        "function rainfall(): int",
        "protected function rainfall(): string",
        "static function rainfall(): string",
        "function &rainfall(): string",
        "function rainfall($value): string",
        "function rainfall(string $value = 'rain'): string",
        "function rainfall(string &$value): string",
        "function rainfall(string ...$value): string",
        "function rainfall(): ?string",
        "function rainfall(): self",
        "function rainfall(): string|int",
        "final function rainfall(): string",
    ] {
        let b = format!("<?php class B extends A {{ {child} {{ return 'rain'; }} }}");
        let r = corpus(vec![
            snapshot(
                "a.php",
                "<?php class A { function rainfall(): string { return 'rain'; } }",
                "rainfall",
                "forecast",
            ),
            snapshot("b.php", &b, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{b}\n{r:?}");
        assert_eq!(r.checked_override_pairs, 0);
    }
}

#[test]
fn static_protected_void_and_primitive_parameters_are_explicitly_equivalent() {
    for (ext, method) in [
        (
            "ts",
            "protected static rainfall(a: boolean, b: number, c: string): void {}",
        ),
        (
            "php",
            "protected static function rainfall(bool $a, float $b, int $c, string $d): void {}",
        ),
    ] {
        let tag = if ext == "php" { "<?php " } else { "" };
        let a = format!("{tag}class A {{ {method} }}");
        let b = format!("{tag}class B extends A {{ {method} }}");
        let r = corpus(vec![
            snapshot(&format!("a.{ext}"), &a, "rainfall", "forecast"),
            snapshot(&format!("b.{ext}"), &b, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_override_pairs, 1);
    }
}

#[test]
fn explicit_override_without_known_ancestor_and_php_yield_stay_unsupported() {
    let r = corpus(vec![
        context("base.ts", "class Base {}"),
        snapshot(
            "a.ts",
            "class A extends Base { override rainfall(): string { return 'rain'; } }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    let r = corpus(vec![
        snapshot(
            "a.php",
            "<?php class A { function rainfall(): string { yield 'rain'; } }",
            "rainfall",
            "forecast",
        ),
        snapshot(
            "b.php",
            "<?php class B extends A { function rainfall(): string { yield 'rain'; } }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
}

#[test]
fn equivalent_override_headers_ignore_comments_but_preserve_parameter_identity() {
    for (ext, a, b) in [
        (
            "ts",
            "class A { rainfall(value: string): string { return value; } }",
            "class B extends A { public /* comment */ rainfall( /* c */ value: /* type */ string): /* return */ string { return value; } }",
        ),
        (
            "php",
            "<?php class A { function rainfall(string $value): string { return $value; } }",
            "<?php class B extends A { public /* comment */ function rainfall(/* c */ string $value): string { return $value; } }",
        ),
    ] {
        let r = corpus(vec![
            snapshot(&format!("a.{ext}"), a, "rainfall", "forecast"),
            snapshot(&format!("b.{ext}"), b, "rainfall", "forecast"),
        ]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_override_pairs, 1);
    }
}

#[test]
fn concrete_methods_in_abstract_containers_do_not_require_abstract_method_contracts() {
    let r = check(
        snapshot(
            "A.ts",
            "abstract class A { rainfall(){} }",
            "rainfall",
            "forecast",
        ),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = check(
        snapshot(
            "A.php",
            "<?php abstract class A { function rainfall(){} }",
            "rainfall",
            "forecast",
        ),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = corpus(vec![
        context("base.ts", "abstract class Base {}"),
        snapshot(
            "a.ts",
            "class A extends Base { rainfall(){} }",
            "rainfall",
            "forecast",
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
