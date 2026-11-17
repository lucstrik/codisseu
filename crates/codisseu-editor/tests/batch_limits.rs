use codisseu_editor::{EditKind, Selection, TextBuffer, TextDelta};

fn deltas(count: usize) -> Vec<TextDelta> {
    (0..count)
        .map(|index| TextDelta {
            range: index * 2..index * 2 + 1,
            insert: "雨".into(),
        })
        .collect()
}

#[test]
fn formatter_batch_of_8192_sparse_deltas_is_one_directed_unicode_undo_transaction() {
    let original = "a ".repeat(8192);
    let expected = "雨 ".repeat(8192);
    let mut buffer = TextBuffer::new(original.clone()).unwrap();
    let before = Selection {
        anchor: 16384,
        head: 1,
    };
    let after = Selection {
        anchor: 16384,
        head: 0,
    };
    buffer.set_selection(before).unwrap();
    assert!(
        buffer
            .apply_batch_with_selection(deltas(8192), before, after)
            .unwrap()
    );
    assert_eq!(buffer.text(), expected);
    assert_eq!(buffer.selection(), after);
    assert_eq!(buffer.revision(), 1);
    assert_eq!(buffer.last_selection_map().unwrap().len(), 8192);
    assert!(buffer.undo());
    assert_eq!(buffer.text(), original);
    assert_eq!(buffer.selection(), before);
    assert!(!buffer.can_undo());
    assert!(buffer.redo());
    assert_eq!(buffer.text(), expected);
    assert_eq!(buffer.selection(), after);
    assert!(!buffer.can_redo());
}

#[test]
fn formatter_batch_of_8193_deltas_is_rejected_without_touching_text_selection_or_redo() {
    let original = "a ".repeat(8193);
    let mut buffer = TextBuffer::new(original.clone()).unwrap();
    buffer
        .apply_delta(
            TextDelta {
                range: 0..0,
                insert: "prefix".into(),
            },
            EditKind::Atomic,
        )
        .unwrap();
    assert!(buffer.undo());
    let before = Selection {
        anchor: 16386,
        head: 0,
    };
    buffer.set_selection(before).unwrap();
    let revision = buffer.revision();
    let identity = buffer.content_identity();
    let change = buffer.last_change().cloned();
    let selection_revision = buffer.selection_revision();
    assert!(
        buffer
            .apply_batch_with_selection(deltas(8193), before, Selection::caret(0))
            .is_err()
    );
    assert_eq!(buffer.text(), original);
    assert_eq!(buffer.selection(), before);
    assert_eq!(buffer.revision(), revision);
    assert_eq!(buffer.content_identity(), identity);
    assert_eq!(buffer.last_change(), change.as_ref());
    assert_eq!(buffer.selection_revision(), selection_revision);
    assert!(buffer.can_redo());
    assert!(!buffer.can_undo());
    assert!(buffer.redo());
    assert_eq!(buffer.text(), format!("prefix{original}"));
}
