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
fn structural_argument_return_and_unused_initializer_never_receive_nominal_proof() {
    for (source, selected) in [
        (
            "interface Port{rainfall():number;} class A{rainfall():number{return 1;}} function f(a:Port){return a.rainfall();} f(new A());",
            vec![0, 2],
        ),
        (
            "interface Port{rainfall():number;} class A{rainfall():number{return 1;}} function make():Port{return new A();}",
            vec![0],
        ),
        (
            "interface Port{rainfall():number;} class A{rainfall():number{return 1;}} const a:Port=new A();",
            vec![0],
        ),
        (
            "interface Port{rainfall():number;} class A{rainfall():number{return 1;}} function f(a:Port){} f(new A() as Port);",
            vec![0],
        ),
        (
            "interface Port{rainfall():number;} class A{rainfall():number{return 1;}} class Box{p:Port=new A();}",
            vec![0],
        ),
        (
            "interface Port{rainfall():number;} class A implements Port{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} let p:Port=new A(); p=new B();",
            vec![0, 1],
        ),
        (
            "class A{static rainfall():number{return 1;}} class B{static rainfall():number{return 2;}} const p:typeof A=B;",
            vec![0],
        ),
    ] {
        let report = check(vec![snapshot("/p/main.ts", source, &selected)]);
        assert_eq!(
            report.coverage,
            Coverage::Unsupported,
            "{source}: {report:?}"
        );
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.message.contains("Transferência estrutural")
                    || i.message.contains("transferência TypeScript")),
            "{report:?}"
        );
    }
}
#[test]
fn nominal_arguments_returns_and_initializers_preserve_complete_rename() {
    let source = "interface Port{rainfall():number;} class A implements Port{rainfall():number{return 1;}} function reportWeather(a:Port):number{return a.rainfall();} function make():Port{return new A();} const a:Port=new A(); reportWeather(a); reportWeather(new A());";
    let report = check(vec![snapshot("/p/main.ts", source, &[0, 1, 2])]);
    assert_eq!(report.coverage, Coverage::Verified, "{report:?}");
}
#[test]
fn function_alias_shadowing_overloads_and_unknown_returns_stay_unsupported() {
    for (tail, selected) in [
        (
            "function f(a:Port){return a.rainfall();} const g=f; g(new A());",
            vec![0, 2],
        ),
        (
            "function f(a:Port){return a.rainfall();} function g(f:any){f(new A());}",
            vec![0, 2],
        ),
        (
            "function f(a:Port){return a.rainfall();} function f(a:Port){return a.rainfall();} f(new A());",
            vec![0, 2, 3],
        ),
        ("function make():Port{return unknownFactory();}", vec![0]),
        ("type P=Port; function make():P{return new A();}", vec![0]),
        ("function f<T extends Port>(a:T){} f(new A());", vec![0]),
    ] {
        let source = format!(
            "interface Port{{rainfall():number;}} class A{{rainfall():number{{return 1;}}}} {tail}"
        );
        let report = check(vec![snapshot("/p/main.ts", &source, &selected)]);
        assert_eq!(
            report.coverage,
            Coverage::Unsupported,
            "{source}: {report:?}"
        );
        assert!(!report.issues.is_empty());
    }
}
#[test]
fn imported_reexported_and_default_functions_do_not_receive_guessed_call_binding() {
    let port =
        "export interface Port{rainfall():number;} export function f(a:Port){return a.rainfall();}";
    let imported = "import {f as report} from './port'; class A{rainfall():number{return 1;}} report(new A());";
    let report = check(vec![
        snapshot("/p/port.ts", port, &[0, 1]),
        snapshot("/p/main.ts", imported, &[]),
    ]);
    assert_eq!(report.coverage, Coverage::Unsupported, "{report:?}");
    let source = "interface Port{rainfall():number;} export default function f(a:Port){return a.rainfall();}";
    let report = check(vec![snapshot("/p/main.ts", source, &[0, 1])]);
    assert_eq!(report.coverage, Coverage::Unsupported, "{report:?}");
}
#[test]
fn inferred_return_and_primitive_method_arguments_are_not_structural_contracts() {
    let source = "class A{rainfall(city:string):number{return city.length;} run(){return this.rainfall('SP');}} function make(){return new A();} const a=new A();a.rainfall('SP');";
    let report = check(vec![snapshot("/p/main.ts", source, &[0, 1, 2])]);
    assert_eq!(report.coverage, Coverage::Verified, "{report:?}");
}
