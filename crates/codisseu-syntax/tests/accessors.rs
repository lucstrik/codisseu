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
fn selected(path: &str, text: &str, old: &str, new: &str, offsets: &[usize]) -> RenameSnapshot {
    let edits = offsets
        .iter()
        .map(|&i| RenameEdit {
            range: range(text, i, i + old.len()),
            new_text: new.into(),
        })
        .collect();
    let mut after = text.to_owned();
    for &i in offsets.iter().rev() {
        after.replace_range(i..i + old.len(), new);
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(after),
        edits,
    }
}
fn all(path: &str, text: &str, old: &str, new: &str) -> RenameSnapshot {
    selected(
        path,
        text,
        old,
        new,
        &text.match_indices(old).map(|(i, _)| i).collect::<Vec<_>>(),
    )
}
fn plan(s: &[RenameSnapshot]) -> Result<Vec<AccessorRenameProposal>, String> {
    plan_property_accessor_renames(
        s,
        "title",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
}
fn validate(p: &AccessorRenameProposal) -> RenameValidation {
    validate_rename_with_limits(
        &p.snapshots,
        &p.new_name,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
}
#[test]
fn php_accessors_preserve_homonyms_literals_and_property_edits() {
    let text = "<?php class A { public string $name; function getName(): string { return $this->name; } function setName(string $v): void { $this->name = $v; } } class B { function getName(): string { return 'name'; } } $x = (new A())->getName(); (new A())->setName('name'); (new B())->getName(); // getName\n";
    let positions: Vec<_> = text.match_indices("name").take(3).map(|(i, _)| i).collect();
    let snapshot = selected("A.php", text, "name", "title", &positions);
    let proposals = plan(&[snapshot]).unwrap();
    assert_eq!(proposals.len(), 2);
    for p in &proposals {
        assert_eq!(
            validate(p).coverage,
            Coverage::Verified,
            "{:?}",
            validate(p)
        );
    }
    let get = proposals.iter().find(|p| p.old_name == "getName").unwrap();
    assert_eq!(get.snapshots[0].edits.len(), 2);
    assert!(get.snapshots[0].after.contains("(new B())->getName()"));
    assert!(get.snapshots[0].after.contains("// getName"));
    assert!(get.snapshots[0].after.contains("$this->name"));
    assert!(!get.snapshots[0].after.contains("$title"));
}
#[test]
fn accessor_family_cross_file_override_and_inherited_calls() {
    let a = all(
        "A.php",
        "<?php class A { public string $name; function getName(): string { return $this->name; } }",
        "name",
        "title",
    );
    let b = selected(
        "B.php",
        "<?php class B extends A { function getName(): string { return parent::getName(); } } echo (new B())->getName();",
        "name",
        "title",
        &[],
    );
    let proposals = plan(&[a, b]).unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].snapshots[0].edits.len(), 1);
    assert_eq!(proposals[0].snapshots[1].edits.len(), 3);
    assert_eq!(
        validate(&proposals[0]).coverage,
        Coverage::Verified,
        "{:?}",
        validate(&proposals[0])
    );
}
#[test]
fn collision_is_a_proposal_rejected_by_method_validator() {
    let s = all(
        "A.php",
        "<?php class A { public string $name; function getName(): string { return $this->name; } function getTitle(): string { return ''; } }",
        "name",
        "title",
    );
    let p = plan(&[s]).unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(validate(&p[0]).coverage, Coverage::Rejected);
}
#[test]
fn no_accessor_and_typescript_produce_empty_plan() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public $name; function read() { return $this->name; } }",
        ),
        (
            "A.ts",
            "class A { name = ''; getName() { return this.name; } }",
        ),
    ] {
        assert!(
            plan(&[all(path, text, "name", "title")])
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn ambiguous_receiver_and_missing_ancestor_block_discovery() {
    for text in [
        "<?php class A { public $name; function getName() { return $this->name; } } function f($x) { $x->getName(); }",
        "<?php class A extends Unknown { public $name; function getName() { return $this->name; } }",
    ] {
        assert!(plan(&[all("A.php", text, "name", "title")]).is_err());
    }
}
#[test]
fn promoted_property_and_boolean_accessors() {
    let s = all(
        "A.php",
        "<?php class A { function __construct(public bool $name) {} function isName(): bool { return $this->name; } function hasName(): bool { return $this->name; } } (new A(true))->isName(); (new A(true))->hasName();",
        "name",
        "title",
    );
    let p = plan(&[s]).unwrap();
    assert_eq!(p.len(), 2);
    for proposal in p {
        assert_eq!(
            validate(&proposal).coverage,
            Coverage::Verified,
            "{:?}",
            validate(&proposal)
        );
    }
}
#[test]
fn cancelled_limited_and_invalid_snapshots_fail() {
    let mut s = all(
        "A.php",
        "<?php class A { public $name; function getName() { return $this->name; } }",
        "name",
        "title",
    );
    assert!(
        plan_property_accessor_renames(
            &[s.clone()],
            "title",
            &AtomicBool::new(true),
            ValidationLimits::default()
        )
        .is_err()
    );
    assert!(
        plan_property_accessor_renames(
            &[s.clone()],
            "title",
            &AtomicBool::new(false),
            ValidationLimits {
                max_nodes: 1,
                max_duration: Duration::from_secs(1)
            }
        )
        .is_err()
    );
    s.after = Arc::from("<?php changed");
    assert!(plan(&[s]).is_err());
}

#[test]
fn inherited_accessor_of_distinct_private_property_is_not_selected() {
    let text = "<?php class A { private string $name; function getName(): string { return $this->name; } } class B extends A { private string $name; }";
    let offset = text.rfind("name").unwrap();
    let s = selected("A.php", text, "name", "title", &[offset]);
    assert!(plan(&[s]).unwrap().is_empty());
}
