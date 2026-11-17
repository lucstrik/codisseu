//! Explicit real compiler checks; source files exist only in the JSON corpus.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn snapshot(root: &Path, path: &str, source: &str, selected: &[usize], new_name: &str) -> Value {
    let positions: Vec<_> = source.match_indices("rainfall").map(|(i, _)| i).collect();
    let offsets: Vec<_> = selected.iter().map(|&index| positions[index]).collect();
    let position = |at: usize| {
        let prefix = &source[..at];
        (
            prefix.bytes().filter(|&b| b == b'\n').count(),
            prefix.rsplit('\n').next().unwrap().encode_utf16().count(),
        )
    };
    let edits:Vec<_>=offsets.iter().map(|&start|{
        let (sl,sc)=position(start);let(el,ec)=position(start+8);
        json!({"range":{"start_line":sl,"start_character":sc,"end_line":el,"end_character":ec},"new_text":new_name})
    }).collect();
    let mut after = source.to_owned();
    for &start in offsets.iter().rev() {
        after.replace_range(start..start + 8, new_name);
    }
    json!({"path":root.join(path),"before":source,"after":after,"edits":edits})
}
fn run(root: &Path, snapshots: Vec<Value>, new_name: &str, config: Option<&str>) -> Value {
    run_named(root, snapshots, new_name, config, "tsconfig.json")
}
fn run_named(
    root: &Path,
    snapshots: Vec<Value>,
    new_name: &str,
    config: Option<&str>,
    config_name: &str,
) -> Value {
    let first = snapshots
        .iter()
        .find(|s| !s["edits"].as_array().unwrap().is_empty())
        .unwrap();
    let range = &first["edits"][0]["range"];
    let payload = json!({"root":root,"origin":{"path":first["path"],"line":range["start_line"],"character":range["start_character"]},"newName":new_name,"snapshots":snapshots,"configText":config,"configName":config_name,"typeScriptModule":std::env::var("CODISSEU_TYPESCRIPT_MODULE").expect("explicit TypeScript module required")});
    let mut child = Command::new(
        std::env::var_os("CODISSEU_NODE_BIN").expect("explicit Node runtime required"),
    )
    .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("semantic/typescript-rename.cjs"))
    .env_remove("NODE_OPTIONS")
    .env_remove("NODE_PATH")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&payload).unwrap().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{:?}", result);
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
const CONFIG: &str = r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs"}}"#;
const MIXED_CONFIG: &str = r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs","allowJs":true,"checkJs":true,"jsx":"preserve"}}"#;

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_rejects_malformed_snapshot_after_utf16_ranges_duplicates_and_overlap() {
    let root = tempfile::tempdir().unwrap();
    let valid = snapshot(
        root.path(),
        "main.ts",
        "/* 😀 */ class A{rainfall(){return 1;}} new A().rainfall();",
        &[0, 1],
        "precipitation",
    );
    for mode in 0..5 {
        let mut invalid = valid.clone();
        let mut snapshots = vec![];
        match mode {
            0 => invalid["after"] = json!("different bytes"),
            1 => invalid["edits"][0]["range"]["start_character"] = json!(4),
            2 => invalid["edits"][0]["range"]["end_character"] = json!(u32::MAX),
            3 => snapshots.push(invalid.clone()),
            4 => {
                let duplicate = invalid["edits"][0].clone();
                invalid["edits"].as_array_mut().unwrap().push(duplicate);
            }
            _ => unreachable!(),
        }
        snapshots.push(invalid);
        let result = run(root.path(), snapshots, "precipitation", Some(CONFIG));
        assert_eq!(result["coverage"], "Unsupported", "mode {mode}: {result}");
        assert!(!result["issues"].as_array().unwrap().is_empty());
    }
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_proves_mixed_javascript_jsx_tsx_consumers_without_omitting_them() {
    let root = tempfile::tempdir().unwrap();
    let base = "export class Weather {rainfall():number{return 1;}}";
    let consumer = "/* dirty 😀 */ import {Weather} from './base'; const weather=new Weather(); weather.rainfall(); class Other{rainfall(){return 2;}} new Other().rainfall();";
    let jsx = "import {Weather} from './base'; const weather=new Weather(); const view=<div>{weather.rainfall()}</div>;";
    let defs = "declare namespace JSX {interface IntrinsicElements{div:{children?:unknown}}}";
    for (path, source) in [("main.js", consumer), ("main.jsx", jsx), ("main.tsx", jsx)] {
        for (selected, expected) in [(vec![0], "Verified"), (vec![], "Rejected")] {
            let result = run(
                root.path(),
                vec![
                    snapshot(root.path(), "base.ts", base, &[0], "precipitation"),
                    snapshot(root.path(), path, source, &selected, "precipitation"),
                    snapshot(root.path(), "jsx.d.ts", defs, &[], "precipitation"),
                ],
                "precipitation",
                Some(MIXED_CONFIG),
            );
            assert_eq!(result["coverage"], expected, "{path}: {result}");
        }
    }
    let result = run(
        root.path(),
        vec![
            snapshot(
                root.path(),
                "base.js",
                "export class Weather {rainfall(){return 1;}}",
                &[0],
                "precipitation",
            ),
            snapshot(
                root.path(),
                "main.ts",
                "import {Weather} from './base'; new Weather().rainfall();",
                &[0],
                "precipitation",
            ),
        ],
        "precipitation",
        Some(MIXED_CONFIG),
    );
    assert_eq!(result["coverage"], "Verified", "JS origin: {result}");
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_preserves_real_javascript_config_and_proves_concrete_unchecked_consumers() {
    let root = tempfile::tempdir().unwrap();
    let base = "export class Weather {rainfall():number{return 1;}}";
    let consumer = "import {Weather} from './base'; new Weather().rainfall();";
    let unchecked = r#"{"compilerOptions":{"target":"ES2022","module":"commonjs","allowJs":true}}"#;
    for (source, config, expected) in [
        (consumer.to_owned(), CONFIG, "Unsupported"),
        (consumer.to_owned(), unchecked, "Verified"),
        (format!("// @ts-check\n{consumer}"), unchecked, "Verified"),
        (
            format!("// @ts-nocheck\n{consumer}"),
            MIXED_CONFIG,
            "Unsupported",
        ),
        (
            format!("// @ts-ignore\n{consumer}"),
            MIXED_CONFIG,
            "Unsupported",
        ),
        (
            format!("{consumer} /** @type {{any}} */ const dynamic={{}}; dynamic.rainfall();"),
            MIXED_CONFIG,
            "Unsupported",
        ),
    ] {
        let result = run(
            root.path(),
            vec![
                snapshot(root.path(), "base.ts", base, &[0], "precipitation"),
                snapshot(root.path(), "main.js", &source, &[0], "precipitation"),
            ],
            "precipitation",
            Some(config),
        );
        assert_eq!(result["coverage"], expected, "{source}: {result}");
    }
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_proves_unchecked_jsconfig_and_inferred_cross_file_method_bindings() {
    let root = tempfile::tempdir().unwrap();
    let declaration = "export class Weather { rainfall(){return 1;} }";
    let dirty = "/* unsaved 😀 */ import {Weather} from './weather'; const item=new Weather(); item.rainfall(); // rainfall comment\nconst spelling='rainfall';";
    let closed = "import {Weather} from './weather'; new Weather().rainfall();";
    let unrelated = "export class Other { rainfall(){return 2;} } new Other().rainfall();";
    for config in [Some("{}"), None] {
        let name = if config.is_some() {
            "jsconfig.json"
        } else {
            "tsconfig.json"
        };
        let result = run_named(
            root.path(),
            vec![
                snapshot(root.path(), "weather.js", declaration, &[0], "forecast"),
                snapshot(root.path(), "main.js", dirty, &[0], "forecast"),
                snapshot(root.path(), "closed.js", closed, &[0], "forecast"),
                snapshot(root.path(), "other.js", unrelated, &[], "forecast"),
            ],
            "forecast",
            config,
            name,
        );
        assert_eq!(
            result["coverage"], "Verified",
            "{name} {config:?}: {result}"
        );
        assert_eq!(result["checkedDeclarations"], 1);
        let result = run_named(
            root.path(),
            vec![
                snapshot(
                    root.path(),
                    "weather.js",
                    "class Weather { rainfall(){return 1;} } module.exports={Weather};",
                    &[0],
                    "forecast",
                ),
                snapshot(
                    root.path(),
                    "main.js",
                    "const {Weather}=require('./weather'); new Weather().rainfall();",
                    &[0],
                    "forecast",
                ),
            ],
            "forecast",
            config,
            name,
        );
        assert_eq!(
            result["coverage"], "Verified",
            "CommonJS {name} {config:?}: {result}"
        );
    }
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_unchecked_javascript_rejects_omissions_homonyms_collision_and_dynamic_receivers() {
    let root = tempfile::tempdir().unwrap();
    for (source, selected, expected) in [
        (
            "class A{rainfall(){return 1;}} new A().rainfall();",
            vec![0],
            "Rejected",
        ),
        (
            "class A{rainfall(){return 1;}} class B{rainfall(){return 2;}} new A().rainfall(); new B().rainfall();",
            vec![0, 1, 2, 3],
            "Rejected",
        ),
        (
            "class A{rainfall(){return 1;} forecast(){return 2;}} new A().rainfall();",
            vec![0, 1],
            "Rejected",
        ),
        (
            "class A{rainfall(){return 1;}} const a=JSON.parse('{}'); a.rainfall();",
            vec![0],
            "Unsupported",
        ),
        (
            "class A{rainfall(){return 1;}} function use(a){a.rainfall();} use(new A());",
            vec![0],
            "Unsupported",
        ),
        (
            "class A{rainfall(){return 1;}} const a=new A(); const key='rainfall'; a[key]();",
            vec![0],
            "Unsupported",
        ),
        (
            "class A{rainfall(){return 1;}} const a=new A(); inspect(a); a.rainfall();",
            vec![0, 1],
            "Unsupported",
        ),
        (
            "class A{rainfall(){return 1;}} const a=new A(); Reflect.get(a,'rainfall'); a.rainfall();",
            vec![0, 2],
            "Unsupported",
        ),
    ] {
        let result = run_named(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.js",
                source,
                &selected,
                "forecast",
            )],
            "forecast",
            Some("{}"),
            "jsconfig.json",
        );
        assert_eq!(result["coverage"], expected, "{source}: {result}");
    }
    for (name, config) in [
        ("../jsconfig.json", Some("{}")),
        ("jsconfig.json", None),
        (
            "jsconfig.json",
            Some(r#"{"compilerOptions":{"allowJs":false}}"#),
        ),
    ] {
        let source = "class A{rainfall(){return 1;}} new A().rainfall();";
        let result = run_named(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.js",
                source,
                &[0, 1],
                "forecast",
            )],
            "forecast",
            config,
            name,
        );
        assert_eq!(
            result["coverage"], "Unsupported",
            "{name} {config:?}: {result}"
        );
    }
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_reports_exact_missing_reference_query_origins_without_synthesizing_edits() {
    let root = tempfile::tempdir().unwrap();
    let declaration = "export class Weather { rainfall(){return 1;} }";
    let consumer = "/* 😀 é */ import {Weather} from './weather'; new Weather().rainfall(); // rainfall\nconst spelling='rainfall';";
    let result = run_named(
        root.path(),
        vec![
            snapshot(root.path(), "weather.js", declaration, &[0], "forecast"),
            snapshot(root.path(), "closed.js", consumer, &[], "forecast"),
        ],
        "forecast",
        Some("{}"),
        "jsconfig.json",
    );
    assert_eq!(result["coverage"], "Rejected", "{result}");
    assert_eq!(
        result["checkedDeclarations"], 1,
        "the known declaration set must stay stable during expansion"
    );
    let start = consumer.find("rainfall").unwrap();
    let range = json!({"start_line":0,"start_character":consumer[..start].encode_utf16().count(),"end_line":0,"end_character":consumer[..start+8].encode_utf16().count()});
    assert_eq!(
        result["missingReferences"],
        json!([{"path":root.path().join("closed.js"),"range":range}])
    );
    assert_eq!(result["conflicts"][0]["range"], range);
    assert!(result.get("edits").is_none());
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_blocks_reflective_keys_serialization_and_escaping_object_shapes() {
    let root = tempfile::tempdir().unwrap();
    let prefix = "class A{rainfall=1;} const a=new A(); a.rainfall; ";
    for tail in [
        "Reflect.get(a,'rainfall');",
        "const getter=Reflect.get; getter(a,'rainfall');",
        "const getter:(o:object,k:string)=>unknown=Reflect.get; getter(a,'rainfall');",
        "Reflect.get.call(null,a,'rainfall');",
        "Reflect.get.apply(null,[a,'rainfall']);",
        "const getter=Reflect.get.bind(null); getter(a,'rainfall');",
        "function getter(o:object,k:string){return Reflect.get(o,k);} getter(a,'rainfall');",
        "Object.keys(a);",
        "const keys=Object.keys; keys(a);",
        "Object.getOwnPropertyDescriptor(a,'rainfall');",
        "Object.defineProperty(a,'rainfall',{value:2});",
        "JSON.stringify(a);",
        "JSON.stringify({nested:a});",
        "JSON.parse('{\"rainfall\":1}');",
        "const copy={...a};",
        "const {...copy}=a;",
        "'rainfall' in a;",
        "for(const key in a){key;}",
        "a.hasOwnProperty('rainfall');",
        "Object.assign({},a);",
    ] {
        let source = format!("{prefix}{tail}");
        let result = run(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.ts",
                &source,
                &[0, 1],
                "precipitation",
            )],
            "precipitation",
            Some(CONFIG),
        );
        assert_eq!(result["coverage"], "Unsupported", "{source}: {result}");
        assert!(
            result["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|issue| issue["message"].as_str().unwrap().contains("runtime")),
            "{source}: {result}"
        );
    }
    let source = format!(
        "{prefix}class B{{rainfall=2;}} const b=new B(); Reflect.get(b,'rainfall'); JSON.stringify(b); Object.keys(b); const copy={{...b}};"
    );
    let result = run(
        root.path(),
        vec![snapshot(
            root.path(),
            "main.ts",
            &source,
            &[0, 1],
            "precipitation",
        )],
        "precipitation",
        Some(CONFIG),
    );
    assert_eq!(result["coverage"], "Verified", "{result}");
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_proves_class_generics_inherited_overrides_and_structural_context() {
    let root = tempfile::tempdir().unwrap();
    for (source, selected) in [
        (
            "class Box<T>{ value:T; constructor(value:T){this.value=value;} rainfall():T{return this.value;} } const box=new Box<number>(1); box.rainfall();",
            vec![0, 1],
        ),
        (
            "class Base<T>{rainfall(value:T):T{return value;}} class Child extends Base<number>{override rainfall(value:number):number{return value+1;}} const c=new Child(); c.rainfall(1);",
            vec![0, 1, 2],
        ),
        (
            "type Port={rainfall():number}; const item:Port={rainfall(){return 1;}}; item.rainfall();",
            vec![0, 1, 2],
        ),
        (
            "interface Port<T>{rainfall(value:T):T;} class A implements Port<number>{rainfall(value:number):number{return value;}} const a:Port<number>=new A(); a.rainfall(1);",
            vec![0, 1, 2],
        ),
    ] {
        let result = run(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.ts",
                source,
                &selected,
                "precipitation",
            )],
            "precipitation",
            Some(CONFIG),
        );
        assert_eq!(result["coverage"], "Verified", "{source}: {result}");
        assert!(!result["compilerVersion"].as_str().unwrap().is_empty());
    }
}
#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_preserves_imported_type_alias_generic_bindings_and_dirty_utf16() {
    let root = tempfile::tempdir().unwrap();
    let a = "/* unsaved 😀 é */ export type Result<T>=T|undefined; export class Base<T>{rainfall(value:T):Result<T>{return value;}}";
    let b = "import {Base as Weather} from './base'; const weather=new Weather<number>(); weather.rainfall(1);";
    let result = run(
        root.path(),
        vec![
            snapshot(root.path(), "base.ts", a, &[0], "precipitation"),
            snapshot(root.path(), "main.ts", b, &[0], "precipitation"),
        ],
        "precipitation",
        Some(CONFIG),
    );
    assert_eq!(result["coverage"], "Verified", "{result}");
}
#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_rejects_omission_homonym_edits_collision_and_capture() {
    let root = tempfile::tempdir().unwrap();
    for (source, selected, new_name) in [
        (
            "class A{rainfall():number{return 1;}} new A().rainfall();",
            vec![0],
            "precipitation",
        ),
        (
            "class A{rainfall():number{return 1;}} class B{rainfall():number{return 2;}} new A().rainfall(); new B().rainfall();",
            vec![0, 1, 2, 3],
            "precipitation",
        ),
        (
            "class A{rainfall():number{return 1;} precipitation():number{return 2;}} new A().rainfall();",
            vec![0, 1],
            "precipitation",
        ),
        (
            "function rainfall():number{return 1;} function inner(){const precipitation=()=>3; return rainfall();}",
            vec![0, 1],
            "precipitation",
        ),
    ] {
        let result = run(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.ts",
                source,
                &selected,
                new_name,
            )],
            new_name,
            Some(CONFIG),
        );
        assert_eq!(result["coverage"], "Rejected", "{source}: {result}");
        assert!(!result["conflicts"].as_array().unwrap().is_empty());
    }
}
#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_never_reads_external_import_and_blocks_dynamic_or_preexisting_errors() {
    let root = tempfile::tempdir().unwrap();
    // A real file intentionally omitted from the corpus must remain inaccessible.
    std::fs::write(root.path().join("outside.ts"), "export class Outside{}").unwrap();
    for source in [
        "import {Outside} from './outside'; class A{rainfall():number{return 1;}} new A().rainfall();",
        "class A{rainfall():number{return 1;}} const a:any=new A(); a.rainfall();",
        "class A{rainfall():number{return 1;}} const key='rainfall'; new A()[key]();",
        "class A{rainfall():number{return 'wrong';}} new A().rainfall();",
    ] {
        let selected = if source.contains("key=") {
            vec![0]
        } else {
            vec![0, 1]
        };
        let result = run(
            root.path(),
            vec![snapshot(
                root.path(),
                "main.ts",
                source,
                &selected,
                "precipitation",
            )],
            "precipitation",
            Some(CONFIG),
        );
        assert_ne!(result["coverage"], "Verified", "{source}: {result}");
    }
    let source = "class A{rainfall():number{return 1;}} new A().rainfall();";
    let result = run(
        root.path(),
        vec![snapshot(
            root.path(),
            "main.ts",
            source,
            &[0, 1],
            "precipitation",
        )],
        "precipitation",
        Some(r#"{"extends":"./outside.json"}"#),
    );
    assert_eq!(result["coverage"], "Unsupported", "{result}");
}

#[test]
#[ignore = "real authorized TypeScript runtime; set CODISSEU_NODE_BIN and CODISSEU_TYPESCRIPT_MODULE"]
fn compiler_checks_exact_prefix_suffix_shorthand_edits_and_unchanged_bindings() {
    let root = tempfile::tempdir().unwrap();
    let source = "const rainfall=1; const object={rainfall}; console.log(object.rainfall);";
    let mut data = snapshot(root.path(), "main.ts", source, &[0, 1], "precipitation");
    data["edits"][1]["new_text"] = json!("rainfall: precipitation");
    data["after"] = json!(
        "const precipitation=1; const object={rainfall: precipitation}; console.log(object.rainfall);"
    );
    let result = run(root.path(), vec![data], "precipitation", Some(CONFIG));
    assert_eq!(result["coverage"], "Verified", "{result}");
    assert_eq!(result["checkedDeclarations"], 1);
}
