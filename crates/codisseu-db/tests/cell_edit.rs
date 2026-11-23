use codisseu_db::{
    cell_edit::{
        CellEditorKind, accepts_input, editor_kind, enum_values, parse_cell_edit,
        validate_cell_edit,
    },
    remote::{Cell, RemoteColumn},
};
fn column(kind: &str, nullable: bool) -> RemoteColumn {
    RemoteColumn {
        name: "value".into(),
        data_type: kind.into(),
        primary_key: false,
        nullable,
    }
}
#[test]
fn exact_integer_bounds_and_numeric_input() {
    for (kind, min, max) in [
        ("tinyint", "-128", "127"),
        ("smallint", "-32768", "32767"),
        ("mediumint", "-8388608", "8388607"),
        ("integer", "-2147483648", "2147483647"),
        ("bigint", "-9223372036854775808", "9223372036854775807"),
        ("bigint unsigned", "0", "18446744073709551615"),
    ] {
        let c = column(kind, false);
        for value in [min, max] {
            assert_eq!(
                parse_cell_edit(&c, value, false).unwrap(),
                Cell::Number(value.into())
            );
        }
        for value in ["", "-", "+", "text", "1.0", "1e2", "18446744073709551616"] {
            assert!(parse_cell_edit(&c, value, false).is_err(), "{kind} {value}");
        }
    }
    for value in ["", "-", "+", "-12", "002"] {
        assert!(accepts_input(&column("bigint", true), value));
    }
    for value in ["1a", "1 2", "--2", "12.3", "1e5"] {
        assert!(!accepts_input(&column("bigint", true), value));
    }
    assert!(!accepts_input(&column("int unsigned", true), "-"));
    assert!(parse_cell_edit(&column("smallint", true), "32768", false).is_err());
}
#[test]
fn nullable_numeric_uses_schema_instead_of_current_null() {
    let c = column("integer", true);
    assert_eq!(
        parse_cell_edit(&c, "12", false).unwrap(),
        Cell::Number("12".into())
    );
    assert!(parse_cell_edit(&c, "not a number", false).is_err());
    assert_eq!(parse_cell_edit(&c, "", true).unwrap(), Cell::Null);
    assert!(parse_cell_edit(&column("integer", false), "", true).is_err());
}
#[test]
fn exact_decimal_precision_scale_and_exponents() {
    let c = column("numeric(30,10)", false);
    let value = "12345678901234567890.1234567890";
    assert_eq!(
        parse_cell_edit(&c, value, false).unwrap(),
        Cell::Number(value.into())
    );
    assert!(parse_cell_edit(&c, "123456789012345678901.1", false).is_err());
    assert!(parse_cell_edit(&c, "1.00000000001", false).is_err());
    for value in [
        "12e-2",
        ".12",
        "+12.00",
        "-0.0000000001",
        "0001.00000000000",
    ] {
        assert!(parse_cell_edit(&c, value, false).is_ok(), "{value}");
    }
    for value in ["NaN", "Infinity", "", ".", "1e", "1e+", "1,2", "1.2.3"] {
        assert!(parse_cell_edit(&c, value, false).is_err(), "{value}");
    }
    for value in ["", "-", ".", "1.", "1e", "1e-", "1e+2"] {
        assert!(accepts_input(&c, value));
    }
}
#[test]
fn decimal_does_not_round_hidden_fractional_digits() {
    let c = column("decimal(4,2)", false);
    for value in ["99.99", "0.001e1", ".1", "0.000"] {
        assert!(parse_cell_edit(&c, value, false).is_ok(), "{value}");
    }
    for value in ["100", "0.001", "1000e-1"] {
        assert!(parse_cell_edit(&c, value, false).is_err(), "{value}");
    }
}
#[test]
fn floating_values_require_finite_numbers() {
    for kind in ["real", "float", "double precision"] {
        for value in ["NaN", "inf", "-Infinity", "1e9999", "text"] {
            assert!(
                parse_cell_edit(&column(kind, true), value, false).is_err(),
                "{kind} {value}"
            );
        }
        assert!(parse_cell_edit(&column(kind, true), "-1.2e-3", false).is_ok());
    }
    assert!(parse_cell_edit(&column("real", true), "1e39", false).is_err());
}
#[test]
fn booleans_have_explicit_choices() {
    let c = column("boolean", true);
    assert_eq!(
        parse_cell_edit(&c, "true", false).unwrap(),
        Cell::Bool(true)
    );
    assert_eq!(parse_cell_edit(&c, "0", false).unwrap(), Cell::Bool(false));
    for v in ["yes", "2", ""] {
        assert!(parse_cell_edit(&c, v, false).is_err());
    }
}
#[test]
fn json_retains_large_numbers_and_distinguishes_sql_null() {
    let c = column("jsonb", true);
    let json = "{\"n\":18446744073709551615,\"f\":1234567890.12345678901234567890}";
    assert_eq!(
        parse_cell_edit(&c, json, false).unwrap(),
        Cell::Json(json.into())
    );
    assert_eq!(
        parse_cell_edit(&c, "null", false).unwrap(),
        Cell::Json("null".into())
    );
    assert_eq!(parse_cell_edit(&c, "null", true).unwrap(), Cell::Null);
    assert!(parse_cell_edit(&c, "{bad}", false).is_err());
}
#[test]
fn date_time_and_uuid_check_actual_format() {
    for (kind, good, bad) in [
        ("date", "2024-02-29", "2025-02-29"),
        ("time without time zone", "23:59:59.123456", "24:00:00"),
        ("time(6)", "-838:59:59.123456", "839:00:00"),
        (
            "datetime(6)",
            "2026-11-23 10:11:12.123456",
            "2026-13-23 10:11:12",
        ),
        (
            "timestamp with time zone",
            "2026-11-23T10:11:12-03:00",
            "2026-11-23T10:11:12",
        ),
        ("uuid", "550e8400-e29b-41d4-a716-446655440000", "not-a-uuid"),
    ] {
        let c = column(kind, true);
        assert!(parse_cell_edit(&c, good, false).is_ok(), "{kind}");
        assert!(parse_cell_edit(&c, bad, false).is_err(), "{kind}");
    }
}
#[test]
fn text_lengths_count_unicode_scalars_and_preserve_empty_string() {
    let c = column("character varying(2)", false);
    assert_eq!(
        parse_cell_edit(&c, "á💙", false).unwrap(),
        Cell::Text("á💙".into())
    );
    assert!(parse_cell_edit(&c, "abc", false).is_err());
    assert_eq!(
        parse_cell_edit(&c, "", false).unwrap(),
        Cell::Text("".into())
    );
}
#[test]
fn generated_binary_and_unknown_types_are_readonly_even_when_null() {
    for kind in [
        "generated integer",
        "generated varchar(10)",
        "bytea",
        "blob",
        "int[]",
        "geometry",
        "some_custom_domain",
    ] {
        let c = column(kind, true);
        assert_eq!(editor_kind(&c), CellEditorKind::Unsupported);
        assert!(parse_cell_edit(&c, "1", false).is_err());
        assert!(parse_cell_edit(&c, "", true).is_err());
    }
}
#[test]
fn enum_and_set_choices_are_schema_values() {
    let c = column("enum('new','in progress','don''t','a(b')", true);
    assert_eq!(
        enum_values(&c).unwrap(),
        ["new", "in progress", "don't", "a(b"]
    );
    assert!(parse_cell_edit(&c, "don't", false).is_ok());
    assert!(parse_cell_edit(&c, "other", false).is_err());
    let set = column("set('a','b')", true);
    assert!(parse_cell_edit(&set, "a,b", false).is_ok());
    assert!(parse_cell_edit(&set, "a,c", false).is_err());
}
#[test]
fn worker_boundary_validates_external_cell_variants() {
    let c = column("integer", true);
    assert!(validate_cell_edit(&c, &Cell::Text("injected text".into())).is_err());
    assert!(validate_cell_edit(&c, &Cell::Number("1.2".into())).is_err());
    assert_eq!(
        validate_cell_edit(&c, &Cell::Bool(true)).unwrap(),
        Cell::Number("1".into())
    );
    assert!(validate_cell_edit(&c, &Cell::Binary(vec![1])).is_err());
}
#[test]
fn limits_apply_before_parsing() {
    assert!(parse_cell_edit(&column("text", true), &"x".repeat(65537), false).is_err());
    assert!(parse_cell_edit(&column("text", true), "bad\0value", false).is_err());
}

#[test]
fn temporal_precision_never_silently_rounds() {
    for kind in [
        "datetime(3)",
        "timestamp(3) without time zone",
        "timestamp(3) with time zone",
        "time(3) without time zone",
    ] {
        let time = kind.starts_with("time(");
        let suffix = if kind.ends_with("with time zone") {
            "-03:00"
        } else {
            ""
        };
        let prefix = if time { "" } else { "2026-11-23 " };
        let c = column(kind, true);
        assert!(
            parse_cell_edit(&c, &format!("{prefix}10:11:12.123{suffix}"), false).is_ok(),
            "{kind}"
        );
        assert!(
            parse_cell_edit(&c, &format!("{prefix}10:11:12.123000{suffix}"), false).is_ok(),
            "{kind}"
        );
        assert!(
            parse_cell_edit(&c, &format!("{prefix}10:11:12.1234{suffix}"), false).is_err(),
            "{kind}"
        );
    }
    assert!(parse_cell_edit(&column("datetime", true), "2026-11-23 10:11:12.1", false).is_err());
    assert!(
        parse_cell_edit(
            &column("timestamp without time zone", true),
            "2026-11-23 10:11:12.1234567",
            false
        )
        .is_err()
    );
}
#[test]
fn decimal_negative_scale_and_fractional_precision_above_precision_are_exact() {
    assert!(parse_cell_edit(&column("numeric(2147483647,-2147483648)", true), "1", false).is_err());
    let thousands = column("numeric(2,-3)", true);
    for value in ["1000", "99000", "1e3", "99000.000"] {
        assert!(parse_cell_edit(&thousands, value, false).is_ok(), "{value}");
    }
    for value in ["1001", "1", "100000", "1e2"] {
        assert!(
            parse_cell_edit(&thousands, value, false).is_err(),
            "{value}"
        );
    }
    let fractional = column("numeric(3,5)", true);
    assert!(parse_cell_edit(&fractional, ".00123", false).is_ok());
    assert!(parse_cell_edit(&fractional, ".01", false).is_err());
    assert!(parse_cell_edit(&fractional, ".000001", false).is_err());
    assert!(parse_cell_edit(&column("numeric", true), "1e65536", false).is_err());
    assert!(parse_cell_edit(&column("numeric", true), "1e-16384", false).is_err());
    assert!(parse_cell_edit(&column("real", true), "1e-99", false).is_err());
    assert!(parse_cell_edit(&column("double precision", true), "1e-999", false).is_err());
}

#[test]
fn mysql_text_byte_limits_do_not_silently_truncate_multibyte_input() {
    assert!(parse_cell_edit(&column("tinytext", true), &"á".repeat(127), false).is_ok());
    assert!(parse_cell_edit(&column("tinytext", true), &"á".repeat(128), false).is_err());
    assert!(parse_cell_edit(&column("text", true), &"x".repeat(65535), false).is_ok());
    assert!(parse_cell_edit(&column("text", true), &"x".repeat(65536), false).is_err());
}
#[test]
fn batch_budget_counts_repeated_originals_before_grouping_and_rejects_large_cells() {
    use codisseu_db::remote::{CellEdit, EditBatch, Engine, TableRef, preview};
    let mut id = column("bigint", false);
    id.name = "id".into();
    id.primary_key = true;
    let mut value = column("varchar(65536)", true);
    value.name = "value".into();
    let row = vec![Cell::Number("1".into()), Cell::Text("x".repeat(32768))];
    let mut batch = EditBatch {
        table: TableRef {
            namespace: "public".into(),
            name: "budget".into(),
        },
        columns: vec![id, value],
        edits: (0..70)
            .map(|_| CellEdit {
                row: row.clone(),
                column: "value".into(),
                value: Cell::Text("y".repeat(32768)),
            })
            .collect(),
    };
    assert!(
        preview(&batch, Engine::Postgres)
            .unwrap_err()
            .contains("4 MiB")
    );
    batch.edits.truncate(1);
    batch.edits[0].value = Cell::Text("x".repeat(65537));
    assert!(
        preview(&batch, Engine::Postgres)
            .unwrap_err()
            .contains("64 KiB")
    );
}
