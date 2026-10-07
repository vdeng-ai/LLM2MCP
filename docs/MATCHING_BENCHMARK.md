# Local matching benchmark

Run from the repository root:

```bash
cargo run --release --locked --example matching_benchmark -- 7
```

The example compares the original 0.4.1 algorithms with current production
matchers using identical generated inputs. It checks every filter decision and
line score before timing, warms both variants, alternates execution order and
reports seven-run medians. Each timed run makes eight passes over its inputs.
One-time matcher setup is measured separately, once, and excluded from matching
time. Compilation is excluded entirely. Output is JSON; no model is called.

## Recorded sample

2026-10-07, Linux x86_64, virtualized AMD EPYC 9V74 shared host, Rust 1.99.0,
release profile. [Full recorded output](benchmarks/matching-0.4.2-linux.json).
The speedup is old matching time divided by new matching time.

| Workload | Old ms | New ms | Speedup | New setup ms |
| --- | ---: | ---: | ---: | ---: |
| 8,000 paths, 6 include / 4 exclude rules | 511.869 | 17.022 | 30.07× | 0.318 |
| 3 keywords, short lines, Unicode folding | 2.201 | 2.195 | 1.00× | 0.001 |
| 12 keywords, short lines, Unicode folding | 4.223 | 4.307 | 0.98× | 0.001 |
| 12 keywords, longer lines, Unicode folding | 7.053 | 7.074 | 1.00× | 0.002 |
| 32 keywords, longer lines, Unicode folding | 16.772 | 16.487 | 1.02× | 0.004 |
| 64 keywords, longer lines, Unicode folding | 32.999 | 22.656 | 1.46× | 0.223 |
| 64 keywords, longer lines, ASCII folding | 32.786 | 20.203 | 1.62× | 0.120 |
| 256 keywords, longer lines, preview weights | 130.033 | 36.602 | 3.55× | 0.220 |

Path inputs mix source files, nested directories, Chinese names, literal
brackets, generated/vendor paths and excluded basenames. Keyword inputs use
4,096 lines with two query words, repeated unrelated filler, ASCII uppercase
on every fourth line and Chinese/Greek/Turkish text on every eighth line. Short
and longer lines use target filler widths of 48 and 320 bytes, plus headers and
keywords. Preview weights give identifiers containing `_` four points. The
corpus is deterministic; it is a synthetic workload, not a sampled repository.

An initial trial using Aho-Corasick from eight keywords regressed at 12 words,
particularly on longer lines. Current code therefore keeps native substring
search below 64 words and retains the original single lowercase pass there;
its small-query timings are essentially unchanged in this sample. Larger
queries use the automaton. This threshold is an empirical, conservative choice,
not a guarantee for every word set, machine or match density.

These numbers measure matching CPU work only. They exclude directory traversal,
file reads, Tree-sitter, symbol caches, sort overhead, HTTP and model inference.
They do not establish overall application speedup, model quality or token/cost
savings. Initialization can outweigh matching savings on very small workloads;
repeat the benchmark on the target machine and profile the actual repository
before adding parallel scanning. For model-specific comparisons use
[paired effectiveness evaluation](EVALUATION.md).
