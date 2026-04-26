# agent-shape report: rune

run_timestamp: `unix:1777212181`
judge_model: `claude-haiku-4-5`

## Tuning battery

- n_trials: 50
- mean_score: 0.550
- completion_rate: 84.0%
- mean_tokens: 2388
- mean_turns: 10.52
- total_invented_commands: 12
- total_fallback_to_sql: 3

## Holdout battery

_empty in v1 (schema supports it; corpus deferred)_

## Per-cell breakdown

| section | task | model | n | score | stddev | tokens | turns | invented | fallback | irr_delta |
|---------|------|-------|---|-------|--------|--------|-------|----------|----------|-----------|
| tuning | add-from-registry-01 | claude-opus-4-7 | 5 | 0.000 | 0.000 | 1659 | 7.20 | rune add documentation --registry anthropic-mirror --type skill | 0 | 0.000 |
| tuning | add-from-registry-01 | claude-sonnet-4-6 | 5 | 0.450 | 0.371 | 3546 | 22.20 | --project flag (not in documented syntax); --type skill (should be -t); rune add documentation --type skill --from anthropic-mirror --project /Users/andrewdunn/gitlab.com/nomograph/rune/fixtures/agent-shape-realistic/project; rune add documentation --type skill --registry anthropic-mirror; rune add with --project flag; rune add with --registry flag (should be --from) | 0 | 0.100 |
| tuning | diagnose-broken-manifest-01 | claude-opus-4-7 | 5 | 0.700 | 0.411 | 3675 | 9.60 | — | 0 | 0.150 |
| tuning | diagnose-broken-manifest-01 | claude-sonnet-4-6 | 5 | 0.250 | 0.000 | 2264 | 8.60 | — | 2 | 0.000 |
| tuning | exploration-01 | claude-opus-4-7 | 5 | 1.000 | 0.000 | 1134 | 4.60 | — | 0 | 0.000 |
| tuning | exploration-01 | claude-sonnet-4-6 | 5 | 0.800 | 0.274 | 1343 | 6.20 | rune config list; rune registries; rune registry list | 0 | 0.100 |
| tuning | publish-local-edit-01 | claude-opus-4-7 | 5 | 0.800 | 0.274 | 1461 | 4.40 | — | 0 | 0.200 |
| tuning | publish-local-edit-01 | claude-sonnet-4-6 | 5 | 0.900 | 0.224 | 1557 | 5.60 | — | 0 | 0.050 |
| tuning | upstream-update-01 | claude-opus-4-7 | 5 | 0.300 | 0.411 | 3508 | 13.40 | — | 1 | 0.050 |
| tuning | upstream-update-01 | claude-sonnet-4-6 | 5 | 0.300 | 0.112 | 3736 | 23.40 | — | 0 | 0.000 |
