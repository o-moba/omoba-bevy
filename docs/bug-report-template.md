# Internal bug report template (copy into issue or chat)

Copy the block below. Replace bracketed fields. Attach logs if possible.

```text
## Summary
[One line: what went wrong]

## Environment
- OS / version:
- GPU / driver (if graphics or crash):
- Git commit / branch:
- Rust version (`rustc -V`):

## MVP impact (required)
- [ ] MVP-blocking (prevents playtest goals in docs/playtest-script.md)
- [ ] Deferrable (nice-to-have; see tasks/MVP-CHECKLIST.md)

## Reproduction
1. [Exact steps, including commands e.g. make start]
2. [...]

## Expected
[What the docs say should happen]

## Actual
[What you observed]

## Logs / screenshots
- Server log snippet:
- Client log snippet:
- Screenshot (if UI/graphics):

## Recovery attempted
[Which RUNBOOK.md troubleshooting steps you tried and outcome]
```

## Report quality bar

- Include **commands** and **environment variables** if you changed defaults (`SERVER_ADDR`, `GAME_SERVER_ADDR`).
- One bug per report when possible; link related issues instead of mixing topics.
