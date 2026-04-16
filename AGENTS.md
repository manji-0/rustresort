## Test Execution

- Use `devbox run test` for repository test execution.
- `devbox run test` is backed by `cargo nextest run`.
- For targeted runs, use `NEXTEST_FILTER='<filter>' devbox run test`.
- Prefer `devbox run fmt` and other `devbox run ...` entry points over invoking toolchain commands directly when a devbox script exists.
