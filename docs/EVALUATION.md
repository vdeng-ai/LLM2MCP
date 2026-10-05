# Paired effectiveness evaluation

Compression is a proxy. It does not measure the tokens a primary coding agent actually spends, elapsed time, or answer correctness. GUI history metrics alone cannot prove a saving.

The standard-library runner `scripts/evaluation/paired.py` compares fresh primary conversations with direct approved context and with an actual MCP `analyze` result. It records provider-reported primary input/output usage, answers, end-to-end time, secondary job usage/models/caches and truncation. Source fingerprints and Git commits identify each workspace snapshot; changes invalidate a pair. `usage_reported_calls` identifies incomplete secondary usage totals instead of interpreting absent provider data as zero. Alternating execution order across repetitions reduces order bias. Missing provider usage remains `null`; it is never estimated or treated as zero.

## Prepare cases

Create a local JSON array (do not commit private code or results):

```json
[{"id":"login-error","workspace":"/absolute/project","paths":["src/login.rs"],"task":"Why does login fail?","baseline_context":"Approved, secret-free source corresponding to the selected workspace paths"}]
```

Keep the repository snapshot, evidence scope, task, primary model/settings and evaluation rubric identical between variants. `baseline_context` is explicitly supplied: this harness does not automatically upload arbitrary workspace files. Check it for credentials before execution. Use several real tasks including debugging, large-file evidence and ambiguous discovery. Run multiple repetitions. Record cold/warm cache experiments separately; the harness does not clear existing caches.

## Execute

```bash
python3 scripts/evaluation/paired.py --cases /tmp/cases.json
export PRIMARY_BASE_URL='https://your-primary-provider/v1'
export PRIMARY_MODEL='your-primary-model'
# Set PRIMARY_API_KEY in your environment if required.
python3 scripts/evaluation/paired.py --cases /tmp/cases.json --executable /path/to/llm2mcp \
  --run --repeats 2 --output /tmp/paired-results.jsonl
```

The first command only validates cases. `--run` executes real model requests and may incur API cost. Secondary routing uses the application's existing configuration/environment. Output creation is exclusive and never overwrites an earlier experiment. Results contain source-derived answers; keep them private. Failure rows remain in the output instead of disappearing from the comparison.

For a full coding-agent experiment, use `--primary-command python3 /path/to/adapter.py`. The adapter receives `{task,context}` on stdin and returns `{answer,usage:{prompt_tokens,completion_tokens}}` on stdout. Aggregate **all** actual primary agent requests, including tool loops/retries; isolate the workspace per variant if the agent modifies files. The built-in HTTP adapter measures one answer per fresh conversation, not a full interactive agent session.

## Score and compare

Have a human or independent evaluator score answers against a predefined correctness rubric from 0 to 1. Supply scores keyed by case ID and repetition:

```json
{"login-error:0":{"baseline":1.0,"offload":1.0}}
```

```bash
python3 scripts/evaluation/paired.py --report /tmp/paired-results.jsonl --scores /tmp/scores.json
```

A pair is marked `validated_saving` only when both requests succeeded, actual primary usage exists, external correctness scores exist, offloading did not reduce correctness, and primary tokens decreased. Inspect secondary tokens/cost and elapsed time separately; primary savings do not imply net cost savings. No real-model benchmark result is bundled with this release.
