use codisseu_syntax::*;
use std::sync::{Arc, atomic::AtomicBool};
fn snapshot(path: &str, source: &str, indices: &[usize]) -> RenameSnapshot {
    let mut after = source.to_owned();
    let edits = indices
        .iter()
        .map(|&a| {
            let pos = |n: usize| {
                let p = &source[..n];
                (
                    p.bytes().filter(|b| *b == b'\n').count() as u32,
                    p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
                )
            };
            let (l, c) = pos(a);
            let (el, ec) = pos(a + 8);
            RenameEdit {
                range: TextRange {
                    start_line: l,
                    start_character: c,
                    end_line: el,
                    end_character: ec,
                },
                new_text: "precipitation".into(),
            }
        })
        .collect();
    for &a in indices.iter().rev() {
        after.replace_range(a..a + 8, "precipitation");
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(source),
        after: Arc::from(after),
        edits,
    }
}
fn offsets(source: &str) -> Vec<usize> {
    source.match_indices("rainfall").map(|(a, _)| a).collect()
}
fn check(s: Vec<RenameSnapshot>) -> RenameValidation {
    validate_rename(&s, "precipitation", &AtomicBool::new(false))
}
#[test]
fn typed_parameters_constructors_this_and_homonyms_bind_to_distinct_declarations() {
    for (path, src) in [
        (
            "/p/A.php",
            "<?php class A { function rainfall(){} function run(){ $this->rainfall(); } } class B { function rainfall(){} } function useA(A $a){$a->rainfall();} $a=new A(); $a->rainfall(); $b=new B(); $b->rainfall();",
        ),
        (
            "/p/A.ts",
            "class A { rainfall(){} run(){ this.rainfall(); } } class B { rainfall(){} } function useA(a:A){a.rainfall();} const a=new A(); a.rainfall(); const b=new B(); b.rainfall();",
        ),
    ] {
        let o = offsets(src);
        let r = check(vec![snapshot(path, src, &[o[0], o[1], o[3], o[4]])]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.scope, ValidationScope::KnownReceiverBindings);
    }
}
#[test]
fn provider_omissions_and_wrong_homonymous_edits_are_rejected_at_the_actual_call() {
    let src = "class A { rainfall(){} } class B { rainfall(){} } const a=new A(); a.rainfall(); const b=new B(); b.rainfall();";
    let o = offsets(src);
    let r = check(vec![snapshot("/p/A.ts", src, &[o[0]])]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::MissingReference)
    );
    let r = check(vec![snapshot("/p/A.ts", src, &[o[0], o[2], o[3]])]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::BindingChanged)
    );
}
#[test]
fn unedited_new_name_call_cannot_acquire_a_new_binding() {
    let src = "class A { rainfall(){} } const a=new A(); a.rainfall(); a.precipitation();";
    let r = check(vec![snapshot("/p/A.ts", src, &offsets(src))]);
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::BindingChanged)
    );
}
#[test]
fn unresolved_reassigned_conditional_and_captured_receivers_never_verify() {
    for src in [
        "class A { rainfall(){} } function f(a){a.rainfall();}",
        "class A { rainfall(){} } let a=new A(); a=new A(); a.rainfall();",
        "class A { rainfall(){} } const a=new A(); function f(){a.rainfall();}",
        "class A { rainfall(){} } function f(a:A|undefined){a?.rainfall();}",
        "<?php class A { function rainfall(){} } if(true){$a=new A();} $a->rainfall();",
        "<?php class A { function rainfall(){} } $a=new A(); $a=new A(); $a->rainfall();",
    ] {
        let path = if src.starts_with("<?php") {
            "/p/A.php"
        } else {
            "/p/A.ts"
        };
        let r = check(vec![snapshot(path, src, &offsets(src))]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
}
#[test]
fn relative_named_import_and_php_alias_resolve_receiver_without_polluting_other_modules() {
    for (a, b, main) in [
        (
            snapshot("/p/A.ts", "export class A { rainfall(){} }", &[17]),
            snapshot("/p/B.ts", "export class A { rainfall(){} }", &[]),
            "import {A as Wet} from './A'; const a=new Wet(); a.rainfall();",
        ),
        (
            snapshot(
                "/p/A.php",
                "<?php namespace Wet; class A { function rainfall(){} }",
                &[40],
            ),
            snapshot(
                "/p/B.php",
                "<?php namespace Dry; class A { function rainfall(){} }",
                &[],
            ),
            "<?php use Wet\\A as Wet; $a=new Wet(); $a->rainfall();",
        ),
    ] {
        let path = if main.starts_with("<?php") {
            "/p/main.php"
        } else {
            "/p/main.ts"
        };
        let r = check(vec![a, b, snapshot(path, main, &offsets(main))]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn nominal_interfaces_and_traits_resolve_calls_and_partial_calls_remain_expandable() {
    for (path, src) in [
        (
            "/p/A.ts",
            "interface Port { rainfall():number; } class A implements Port { rainfall():number{return 1;} } function report(a:Port){return a.rainfall();} const a:Port=new A(); a.rainfall();",
        ),
        (
            "/p/A.php",
            "<?php interface Port { function rainfall():int; } trait T { function rainfall():int{return 1;} } class A implements Port { use T; } function report(Port $a){return $a->rainfall();} $a=new A(); $a->rainfall();",
        ),
    ] {
        let o = offsets(src);
        let r = check(vec![snapshot(path, src, &o)]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert!(!r.related_methods.is_empty());
        let r = check(vec![snapshot(path, src, &o[..2])]);
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
        assert!(r.issues.is_empty(), "{r:?}");
        assert!(
            r.conflicts
                .iter()
                .all(|c| c.kind == ConflictKind::MissingReference)
        );
        assert!(!r.related_methods.is_empty());
    }
}
#[test]
fn structural_conversion_is_blocked_before_it_can_introduce_missing_interface_methods() {
    let src = "interface Port { rainfall():number; } class A { rainfall():number{return 1;} } const a:Port=new A(); a.rainfall();";
    let o = offsets(src);
    let r = check(vec![snapshot("/p/A.ts", src, &[o[0], o[2]])]);
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    assert!(r.issues.iter().any(|i| i.message.contains("estrutural")));
}
#[test]
fn unicode_and_php_method_case_resolve_without_replacing_comments_or_strings() {
    let src = "<?php class A { function rainfall(){} } function f(A $árvore){ /* rainfall 🌧 */ $árvore->RAINFALL(); echo 'rainfall'; }";
    let mut s = snapshot("/p/A.php", src, &[src.find("rainfall").unwrap()]);
    let a = src.find("RAINFALL").unwrap();
    let tmp = snapshot("/p/A.php", src, &[a]);
    s.edits.extend(tmp.edits);
    s.after = Arc::from(
        src.replacen("function rainfall", "function precipitation", 1)
            .replacen("->RAINFALL", "->precipitation", 1),
    );
    let r = check(vec![s]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn indirect_mutations_and_explicit_this_cannot_receive_a_guessed_binding() {
    for src in [
        "class A{rainfall(){}} class B{rainfall(){}} let a=new A(); [a]=[new B()]; a.rainfall();",
        "class A{rainfall(){}} class B{rainfall(){}} let a=new A(); {a=new B();} a.rainfall();",
        "class A{rainfall(){}} class B{rainfall(){}} let a=new A(); function change(){a=new B();} change(); a.rainfall();",
        "class A{rainfall(){}} class B{rainfall(){}} function f(){let a=new A();eval('a=new B()');a.rainfall();}",
        "class A{rainfall(){} run(){const object={run(){this.rainfall();}};}}",
        "class A{rainfall(){} run(this:B){this.rainfall();}} class B{rainfall(){}}",
        "<?php class A{function rainfall(){}} class B{function rainfall(){}} $a=new A();[$a]=[new B()];$a->rainfall();",
        "<?php class A{function rainfall(){}} class B{function rainfall(){}} function f(){$a=new A();eval('$a=new B();');$a->rainfall();}",
        "<?php class A{function rainfall(){}} class B{function rainfall(){}} function replace(&$x){$x=new B();} function f(A $a){replace($a);$a->rainfall();}",
    ] {
        let o = offsets(src);
        // The declaration in B and the spelling in eval strings aren't edits.
        let call = src.rfind("rainfall").unwrap();
        let path = if src.starts_with("<?php") {
            "/p/A.php"
        } else {
            "/p/A.ts"
        };
        let r = check(vec![snapshot(path, src, &[o[0], call])]);
        assert_ne!(r.coverage, Coverage::Verified, "{src}: {r:?}");
        assert!(r.conflicts.len() + r.issues.len() > 0, "{r:?}");
    }
}
#[test]
fn static_instance_dispatch_and_php_like_ts_identifiers_do_not_merge_bindings() {
    for src in [
        "class A{rainfall(){} run(self:B){self.rainfall();}} class B{rainfall(){}}",
        "class A{static rainfall(){} rainfall(){}} const a=new A();a.rainfall();",
    ] {
        let o = offsets(src);
        let selected = if src.contains("self:B") {
            vec![o[0], o[1]]
        } else {
            vec![o[0], o[2]]
        };
        let r = check(vec![snapshot("/p/A.ts", src, &selected)]);
        assert_eq!(r.coverage, Coverage::Rejected, "{src}: {r:?}");
    }
    let src =
        "class A{static rainfall(){} rainfall(){}} A.rainfall(); const a=new A();a.rainfall();";
    let o = offsets(src);
    let r = check(vec![snapshot("/p/A.ts", src, &[o[0], o[2]])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn omitted_computed_and_destructured_references_are_not_certified_as_complete() {
    for src in [
        "class A{rainfall(){}} const a=new A();a['rainfall']();",
        "class A{rainfall(){}} const a=new A();const {rainfall}=a; rainfall();",
    ] {
        let r = check(vec![snapshot("/p/A.ts", src, &[offsets(src)[0]])]);
        assert_eq!(r.coverage, Coverage::Unsupported, "{src}: {r:?}");
    }
}
#[test]
fn combined_dispatch_binding_budget_and_cancellation_never_approve_partial_work() {
    let source = "class A{rainfall(){}} const a=new A();a.rainfall();";
    let s = snapshot("/p/A.ts", source, &offsets(source));
    for limits in [
        ValidationLimits {
            max_nodes: 0,
            max_duration: std::time::Duration::from_secs(2),
        },
        ValidationLimits {
            max_nodes: 250_000,
            max_duration: std::time::Duration::ZERO,
        },
        ValidationLimits {
            max_nodes: 32,
            max_duration: std::time::Duration::from_secs(2),
        },
    ] {
        let r = validate_rename_with_limits(
            std::slice::from_ref(&s),
            "precipitation",
            &AtomicBool::new(false),
            limits,
        );
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
        assert!(r.issues.iter().any(|i| i.kind == IssueKind::Limit), "{r:?}");
    }
    let r = validate_rename(
        std::slice::from_ref(&s),
        "precipitation",
        &AtomicBool::new(true),
    );
    assert!(
        r.issues.iter().any(|i| i.kind == IssueKind::Cancelled),
        "{r:?}"
    );
    assert_eq!(s.before.as_ref(), source);
}
