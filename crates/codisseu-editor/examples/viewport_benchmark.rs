//! Deterministic local measurements, not an assertion of a frame budget.
//! Run with: cargo run --release -p codisseu-editor --example viewport_benchmark --locked
use codisseu_editor::{EditKind, TextBuffer, TextDelta, TextSnapshot};
use std::{hint::black_box, time::Instant};

fn measured(mut f: impl FnMut(), n: usize) -> (u128, u128, u128) {
    let mut timings = Vec::with_capacity(n);
    for _ in 0..n {
        let start = Instant::now();
        f();
        timings.push(start.elapsed().as_nanos());
    }
    timings.sort_unstable();
    (
        timings[n / 2],
        timings[(n * 95 / 100).min(n - 1)],
        timings[n - 1],
    )
}
fn fixture(bytes: usize) -> String {
    let line = "let weather = 'ação e\u{301} 👩‍💻 🇧🇷';\r\n";
    line.repeat(bytes / line.len())
}
fn main() {
    println!(
        "{{\"unit\":\"nanoseconds\",\"build\":\"{}\",\"repetitions\":500,\"measurements\":[",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for (i, bytes) in [1024 * 1024 - 1024, 4 * 1024 * 1024, 16 * 1024 * 1024]
        .into_iter()
        .enumerate()
    {
        let start = Instant::now();
        let mut snapshot = TextSnapshot::from_str(&fixture(bytes));
        let construct = start.elapsed().as_nanos();
        let mut line = snapshot.len_lines() / 2;
        let view = measured(
            || {
                let cloned = snapshot.clone();
                line = (line + 17) % (cloned.len_lines() - 40);
                let content: Vec<_> = (line..line + 40)
                    .map(|l| cloned.line(l).to_string())
                    .collect();
                black_box(content);
            },
            500,
        );
        let scalar = snapshot.len_chars() / 2;
        let edit = measured(
            || {
                snapshot.insert(scalar, "x");
                snapshot.remove(scalar..scalar + 1);
            },
            500,
        );
        let domain = {
            let mut buffer = TextBuffer::new(snapshot.to_string()).unwrap();
            let (p50, p95, max) = measured(
                || {
                    buffer
                        .apply_delta(
                            TextDelta {
                                range: scalar..scalar,
                                insert: "x".into(),
                            },
                            EditKind::Atomic,
                        )
                        .unwrap();
                    assert!(buffer.undo());
                },
                500,
            );
            format!("{{\"p50\":{p50},\"p95\":{p95},\"max\":{max}}}")
        };
        println!(
            "{}{{\"bytes\":{},\"lines\":{},\"construct\":{},\"viewport40\":{{\"p50\":{},\"p95\":{},\"max\":{}}},\"rope_insert_remove\":{{\"p50\":{},\"p95\":{},\"max\":{}}},\"domain_edit_undo\":{}}}",
            if i > 0 { "," } else { "" },
            snapshot.len_bytes(),
            snapshot.len_lines(),
            construct,
            view.0,
            view.1,
            view.2,
            edit.0,
            edit.1,
            edit.2,
            domain
        );
    }
    println!("]}}");
}
