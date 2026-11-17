use codisseu_db::mongo_text::execution_range;
fn target(text: &str, at: usize) -> Result<String, String> {
    let r = execution_range(text, at, None)?;
    Ok(text.chars().skip(r.start).take(r.len()).collect())
}
#[test]
fn cursor_targets_nested_multiline_json_not_the_whole_console() {
    let first = r#"{"collection":"users","filter":{"label":"á 😀 ; } [ \\\""}}"#;
    let second =
        "{\r\n  \"collection\": \"users\",\r\n  \"filter\": {\"_id\": {\"$in\": [2, 3]}}\r\n}";
    let text = format!("// consultas 😀\r\n{first};\r\n\r\n/* segundo */\r\n{second}\r\n");
    let at = text[..text.find("$in").unwrap()].chars().count();
    assert_eq!(target(&text, at).unwrap(), second);
    assert_eq!(target(&text, text.chars().count()).unwrap(), second);
    assert_eq!(
        target(&text, text[..text.find(';').unwrap()].chars().count()).unwrap(),
        first
    );
}
#[test]
fn semicolons_gaps_comments_and_explicit_selection_keep_precise_boundaries() {
    let first = r#"{"collection":"one"}"#;
    let second = r#"{"collection":"two"}"#;
    let text = format!("{first};  {second}");
    assert_eq!(target(&text, first.len()).unwrap(), first);
    assert_eq!(target(&text, first.len() + 2).unwrap(), first);
    assert_eq!(target(&text, first.len() + 4).unwrap(), second);
    let text = format!("{first};\n\n{second}");
    assert_eq!(target(&text, first.len() + 2).unwrap(), second);
    let r = execution_range(&text, 0, Some(1..4)).unwrap();
    assert_eq!(r, 1..4);
    assert!(execution_range(&text, 0, Some(std::ops::Range { start: 4, end: 1 })).is_err());
    assert!(execution_range(&text, text.len() + 1, None).is_err());
    assert!(execution_range(&text, 0, Some(0..text.len() + 1)).is_err());
}
#[test]
fn invalid_or_incomplete_target_fails_without_falling_back_to_another_block() {
    for text in [
        "",
        "  // only a comment",
        "/* unclosed",
        "{\"collection\":",
        "{\"x\": [1}",
        "[{}]",
    ] {
        assert!(target(text, 0).is_err(), "{text}");
    }
    let text = "{\"collection\":\"one\"};\n{\"collection\":\"unfinished";
    assert_eq!(target(text, 2).unwrap(), "{\"collection\":\"one\"}");
    assert!(target(text, text.len()).is_err());
}

#[test]
fn shell_call_is_one_block_across_pipeline_and_chain_lines() {
    let first = "db.getSiblingDB('outra')\r\n // cadeia 😀\r\n .getCollection('a-b')\r\n .aggregate([{$match: {x: {$in: [1,2]}}}, /* ] ; */ {$sort:{x:-1,_id:1}}])";
    let second = "db.users.find({label: 'á 😀; \\' }'}).limit(1)";
    let text = format!("{first};\n\n{second};\n");
    for needle in ["$in", "$sort", "getCollection"] {
        let at = text[..text.find(needle).unwrap()].chars().count();
        assert_eq!(target(&text, at).unwrap(), first);
    }
    assert_eq!(target(&text, text.chars().count()).unwrap(), second);
    let newline = format!("{first}\n{second}");
    assert_eq!(target(&newline, newline.chars().count()).unwrap(), second);
    assert!(target("db.users.aggregate([{$match:{}}]", 3).is_err());
}
