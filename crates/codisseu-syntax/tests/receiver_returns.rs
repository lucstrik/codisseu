use codisseu_syntax::*;
use std::sync::{Arc, atomic::AtomicBool};
fn snapshot(path: &str, source: &str, selected: &[usize]) -> RenameSnapshot {
    let positions = source
        .match_indices("rainfall")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let offsets = selected
        .iter()
        .map(|&index| positions[index])
        .collect::<Vec<_>>();
    let mut after = source.to_owned();
    let mut edits = Vec::new();
    for &start in &offsets {
        let position = |n: usize| {
            let p = &source[..n];
            (
                p.bytes().filter(|&b| b == b'\n').count() as u32,
                p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
            )
        };
        let (a, b) = position(start);
        let (c, d) = position(start + 8);
        edits.push(RenameEdit {
            range: TextRange {
                start_line: a,
                start_character: b,
                end_line: c,
                end_character: d,
            },
            new_text: "precipitation".into(),
        });
    }
    for &start in offsets.iter().rev() {
        after.replace_range(start..start + 8, "precipitation");
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(source),
        after: Arc::from(after),
        edits,
    }
}
fn check(snapshots: Vec<RenameSnapshot>) -> RenameValidation {
    validate_rename(&snapshots, "precipitation", &AtomicBool::new(false))
}
#[test]
fn direct_factory_returns_and_single_assignments_are_nominal_receivers() {
    for (path, source) in [
        (
            "/p/a.php",
            "<?php class A{function rainfall(): int{return 1;}} function make(): A{return new A();} make()->rainfall(); $x=make(); $x->rainfall();",
        ),
        (
            "/p/a.ts",
            "class A{rainfall(): number{return 1;}} function make(): A{return new A();} make().rainfall(); const x=make(); x.rainfall();",
        ),
    ] {
        let r = check(vec![snapshot(path, source, &[0, 1, 2])]);
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn php_factory_import_alias_and_namespace_returns_follow_declarations() {
    let r = check(vec![
        snapshot(
            "/p/a.php",
            "<?php namespace Weather; class A{function rainfall(): int{return 1;}} function make(): A{return new A();}",
            &[0],
        ),
        snapshot(
            "/p/main.php",
            "<?php namespace App; use function Weather\\make as build; build()->rainfall();",
            &[0],
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn php_method_return_and_property_type_chains_preserve_homonyms() {
    let source = "<?php class A{function rainfall(): int{return 1;}} class B{function rainfall(): int{return 2;}} class Box{public A $item; function get(): A{return new A();}} (new Box())->get()->rainfall(); (new Box())->item->rainfall(); (new B())->rainfall();";
    let r = check(vec![snapshot("/p/a.php", source, &[0, 2, 3])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn typed_typescript_property_initializer_is_checked_nominally() {
    let source = "class A{rainfall(): number{return 1;}} class Box{item: A = new A();} new Box().item.rainfall();";
    let r = check(vec![snapshot("/p/a.ts", source, &[0, 1])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn incomplete_factory_call_edits_are_rejected() {
    for (path, source) in [
        (
            "/p/a.php",
            "<?php class A{function rainfall(): int{return 1;}} function make(): A{return new A();} make()->rainfall();",
        ),
        (
            "/p/a.ts",
            "class A{rainfall(): number{return 1;}} function make(): A{return new A();} make().rainfall();",
        ),
    ] {
        let r = check(vec![snapshot(path, source, &[0])]);
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::MissingReference)
        );
    }
}
#[test]
fn nullable_dynamic_mutated_and_structural_sources_are_not_certified() {
    for (path, source, edits) in [
        (
            "/p/a.php",
            "<?php class A{function rainfall(): int{return 1;}} function make(): ?A{return null;} make()->rainfall();",
            vec![0, 1],
        ),
        (
            "/p/a.php",
            "<?php class A{function rainfall(): int{return 1;}} class Box{public ?A $item;} (new Box())->item->rainfall();",
            vec![0, 1],
        ),
        (
            "/p/a.php",
            "<?php class A{function rainfall(): int{return 1;}} class Box{public A $item; function change(A $a){$this->item=$a;}} (new Box())->item->rainfall();",
            vec![0, 1],
        ),
        (
            "/p/a.ts",
            "class A{rainfall(): number{return 1;}} class B{rainfall(): number{return 2;}} function make(): A{return new B();} make().rainfall();",
            vec![0, 2],
        ),
        (
            "/p/a.ts",
            "class A{rainfall(): number{return 1;}} function make(): A{return new A();} let x=make(); x=new A(); x.rainfall();",
            vec![0, 1],
        ),
        (
            "/p/a.ts",
            "class A{rainfall(): number{return 1;}} function make(): A|null{return null;} make()?.rainfall();",
            vec![0, 1],
        ),
    ] {
        let r = check(vec![snapshot(path, source, &edits)]);
        assert_ne!(r.coverage, Coverage::Verified, "{source}: {r:?}");
    }
}
#[test]
fn shadowed_factory_and_duplicate_declarations_never_supply_return_types() {
    for source in [
        "class A{rainfall():number{return 1;}} function make():A{return new A();} function f(make:()=>A){make().rainfall();}",
        "class A{rainfall():number{return 1;}} function make():A{return new A();} function make():A{return new A();} make().rainfall();",
    ] {
        let r = check(vec![snapshot("/p/a.ts", source, &[0, 1])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}

#[test]
fn typescript_method_returns_and_arguments_have_independent_transfer_proof() {
    let source = "class A{rainfall():number{return 1;}} class Factory{make():A{return new A();} consume(a:A):number{return a.rainfall();}} new Factory().make().rainfall(); new Factory().consume(new A());";
    let r = check(vec![snapshot("/p/a.ts", source, &[0, 1, 2])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    for source in [
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Factory{make():A{return new B();}} new Factory().make().rainfall();",
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Factory{consume(a:A):number{return a.rainfall();}} new Factory().consume(new B());",
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Factory{consume(a:A):number{return a.rainfall();}} const f=new Factory().consume; f(new B());",
    ] {
        let r = check(vec![snapshot("/p/a.ts", source, &[0, 2])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn typescript_imported_factory_alias_uses_original_return_declaration() {
    let r = check(vec![
        snapshot(
            "/p/a.ts",
            "export class A{rainfall():number{return 1;}} export function make():A{return new A();}",
            &[0],
        ),
        snapshot(
            "/p/main.ts",
            "import {make as build} from './a'; build().rainfall();",
            &[0],
        ),
    ]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn mutated_callable_generic_property_and_optional_parameter_remain_unsupported() {
    for source in [
        "class A{rainfall():number{return 1;}} function make():A{return new A();} make=unknown; make().rainfall();",
        "class A{rainfall():number{return 1;}} class Box<A>{item:A;} new Box().item.rainfall();",
        "class A{rainfall():number{return 1;}} function f(a?:A){a.rainfall();}",
        "class A{rainfall():number{return 1;}} class Factory{make():A{return new A();}} const f=new Factory(); f.make=unknown; f.make().rainfall();",
    ] {
        let r = check(vec![snapshot("/p/a.ts", source, &[0, 1])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}

#[test]
fn nested_property_structural_transfers_cannot_hide_affected_runtime_class() {
    for source in [
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Box{item:A=new A();} function f(box:Box){box.item.rainfall();} f({item:new B()});",
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Box{item:A=new A();} function make():Box{return {item:new B()};} make().item.rainfall();",
    ] {
        let r = check(vec![snapshot("/p/a.ts", source, &[0, 2])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}

#[test]
fn factory_and_field_overrides_cannot_hide_structural_return_substitution() {
    for source in [
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Factory{make():A{return new A();}} class Derived extends Factory{make():B{return new B();}} function f(factory:Factory){factory.make().rainfall();} f(new Derived());",
        "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} class Box{item:A=new A();} class Derived extends Box{item:B=new B();} function f(box:Box){box.item.rainfall();} f(new Derived());",
    ] {
        let r = check(vec![snapshot("/p/a.ts", source, &[0, 2])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}

#[test]
fn self_return_and_nominal_covariance_remain_supported() {
    let php = "<?php class A{function rainfall():int{return 1;} function myself():self{return $this;}} (new A())->myself()->rainfall();";
    let r = check(vec![snapshot("/p/a.php", php, &[0, 1])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let ts = "class A{rainfall():number{return 1;}} class Child extends A{} class Factory{make():A{return new A();}} class Derived extends Factory{make():Child{return new Child();}} function f(factory:Factory){factory.make().rainfall();} f(new Derived());";
    let r = check(vec![snapshot("/p/a.ts", ts, &[0, 1])]);
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn dynamic_property_access_writes_and_implicit_nullable_defaults_are_not_nominal() {
    for (path, source) in [
        (
            "/p/a.php",
            "<?php class A{function rainfall():int{return 1;}} class Box{public A $item; function f(){return $this->$item->rainfall();}}",
        ),
        (
            "/p/a.php",
            "<?php class A{function rainfall():int{return 1;}} function f(A $a=null){$a->rainfall();}",
        ),
        (
            "/p/a.php",
            "<?php class A{function rainfall():int{return 1;}} class Box{public A $item; function f($name){$this->$name=new A();return $this->item->rainfall();}}",
        ),
        (
            "/p/a.ts",
            "class A{rainfall():number{return 1;}} class Box{item:A=new A(); f(key:string){this[key]=unknown; this.item.rainfall();}}",
        ),
    ] {
        let r = check(vec![snapshot(path, source, &[0, 1])]);
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
