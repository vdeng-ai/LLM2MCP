//! Synthetic, paired matching benchmark; does not call a model or scan a disk.
#[path = "../tests/support/legacy_filters.rs"]
mod legacy_filters;
#[path = "../src/path_filters.rs"]
mod path_filters;
#[path = "../src/search.rs"]
mod search;

use serde_json::{Value, json};
use std::{hint::black_box, time::Instant};

fn measured(mut run: impl FnMut() -> usize) -> (f64, usize) {
    let start = Instant::now();
    let checksum = black_box(run());
    (start.elapsed().as_secs_f64() * 1000.0, checksum)
}

fn paired(
    name: &str,
    repeats: usize,
    setup_ms: f64,
    mut old: impl FnMut() -> usize,
    mut new: impl FnMut() -> usize,
) -> Value {
    assert_eq!(old(), new(), "warmup checksum: {name}");
    let mut before = Vec::new();
    let mut after = Vec::new();
    for iteration in 0..repeats {
        // Alternate order so one implementation does not always run first.
        let (a, b) = if iteration % 2 == 0 {
            (measured(&mut old), measured(&mut new))
        } else {
            let b = measured(&mut new);
            (measured(&mut old), b)
        };
        assert_eq!(a.1, b.1, "checksum: {name}");
        before.push(a.0);
        after.push(b.0);
    }
    before.sort_by(f64::total_cmp);
    after.sort_by(f64::total_cmp);
    let old_ms = before[repeats / 2];
    let new_ms = after[repeats / 2];
    json!({"case": name, "old_ms": old_ms, "new_ms": new_ms,
        "speedup": old_ms / new_ms, "new_setup_ms": setup_ms})
}

fn main() -> anyhow::Result<()> {
    let repeats = std::env::args()
        .nth(1)
        .map(|arg| arg.parse::<usize>())
        .transpose()?
        .unwrap_or(7);
    anyhow::ensure!((1..=101).contains(&repeats), "repeats must be 1..=101");
    let passes = 8;
    let mut results = Vec::new();
    let includes = [
        "src/**/*.rs",
        "*.md",
        "tests/*.rs",
        "*.toml",
        "[literal].rs",
        "assets/*.json",
    ]
    .map(str::to_owned);
    let excludes = ["src/generated/**", "vendor/**", "*_test.rs", "README.md"].map(str::to_owned);
    let paths = (0..8000)
        .map(|index| match index % 8 {
            0 => format!("src/module_{index}/worker.rs"),
            1 => format!("docs/chapter_{index}.md"),
            2 => format!("src/generated/{index}.rs"),
            3 => format!("assets/config_{index}.json"),
            4 => format!("tests/worker_{index}_test.rs"),
            5 => format!("vendor/library_{index}/Cargo.toml"),
            6 => format!("src/模块_{index}/[literal].rs"),
            _ => format!("other/file_{index}.txt"),
        })
        .collect::<Vec<_>>();
    let start = Instant::now();
    let filters = path_filters::PathFilters::new(&includes, &excludes)?;
    let setup = start.elapsed().as_secs_f64() * 1000.0;
    // Verify each path, not just the timed aggregate.
    for path in &paths {
        assert_eq!(
            filters.allows(path),
            legacy_filters::matches_filters(path, &includes, &excludes)
        );
    }
    results.push(paired(
        "path_filters_8000_x8",
        repeats,
        setup,
        || {
            (0..passes)
                .map(|_| {
                    paths
                        .iter()
                        .filter(|path| {
                            legacy_filters::matches_filters(black_box(path), &includes, &excludes)
                        })
                        .count()
                })
                .sum()
        },
        || {
            (0..passes)
                .map(|_| {
                    paths
                        .iter()
                        .filter(|path| filters.allows(black_box(path)))
                        .count()
                })
                .sum()
        },
    ));

    let base = search::keywords(
        "cache auth session worker timeout request parser logging configuration queue",
    );
    for (count, width, ascii, weighted) in [
        (3, 48, false, false),
        (12, 48, false, false),
        (12, 320, false, false),
        (32, 320, false, false),
        (64, 320, false, false),
        (64, 320, true, false),
        (256, 320, false, true),
    ] {
        let words = (0..count)
            .map(|index| {
                if index < base.len() {
                    base[index].clone()
                } else {
                    format!("identifier_{index:04}")
                }
            })
            .collect::<Vec<_>>();
        let lines = (0..4096)
            .map(|index| {
                let mut line = format!(
                    "record {index}: {} {} ",
                    words[index % count],
                    words[(index * 7) % count]
                );
                line.push_str(&"unrelated payload; ".repeat(width / 19));
                if index % 4 == 0 {
                    line = line.to_ascii_uppercase();
                }
                if index % 8 == 0 {
                    line.push_str(" 登录缓存 ΟΣ İ");
                }
                line
            })
            .collect::<Vec<_>>();
        let start = Instant::now();
        let mut matcher = if weighted {
            search::KeywordMatcher::weighted(&words, |word| if word.contains('_') { 4 } else { 1 })
        } else if ascii {
            search::KeywordMatcher::ascii(&words)
        } else {
            search::KeywordMatcher::new(&words)
        };
        let setup = start.elapsed().as_secs_f64() * 1000.0;
        let old_score = |text: &str| {
            let folded = if ascii {
                text.to_ascii_lowercase()
            } else {
                text.to_lowercase()
            };
            words
                .iter()
                .filter(|word| folded.contains(word.as_str()))
                .map(|word| if weighted && word.contains('_') { 4 } else { 1 })
                .sum::<usize>()
        };
        for line in &lines {
            assert_eq!(old_score(line), matcher.score(line));
        }
        let name = format!(
            "keywords_{count}_width_{width}_{}_4096_x8",
            if weighted {
                "weighted"
            } else if ascii {
                "ascii"
            } else {
                "unicode"
            }
        );
        results.push(paired(
            &name,
            repeats,
            setup,
            || {
                (0..passes)
                    .map(|_| {
                        lines
                            .iter()
                            .map(|line| old_score(black_box(line)))
                            .sum::<usize>()
                    })
                    .sum()
            },
            || {
                (0..passes)
                    .map(|_| {
                        lines
                            .iter()
                            .map(|line| matcher.score(black_box(line)))
                            .sum::<usize>()
                    })
                    .sum()
            },
        ));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "kind": "synthetic matching only; no disk I/O, model latency or token savings",
            "repeats": repeats, "statistic": "median milliseconds; setup excluded from matching",
            "results": results
        }))?
    );
    Ok(())
}
