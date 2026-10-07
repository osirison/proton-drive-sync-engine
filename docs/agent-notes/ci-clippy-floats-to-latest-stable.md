---
trigger: cargo clippy, git push, gh pr checks
depends_on: .github/workflows/ci.yml, rustup
recorded: 2026-10-07
---

# CI clippy runs on the newest stable, not the machine default

**Symptom:** `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean
locally, and CI's `rust` job fails at Lint, e.g.

```
error: use of deprecated method `std::sync::atomic::Atomic::<usize>::fetch_update`: renamed to `try_update`
```

**Cause:** the job uses `dtolnay/rust-toolchain@stable`, which floats to the newest stable
(1.99.0 on 2026-10-07). The machine default can lag (1.97.1 here), so an API deprecated in
between passes locally.

**Before pushing:** install the CI toolchain alongside the default (never `rustup default`
or `rustup update` — other sessions use the default) and lint with it:

```bash
rustup toolchain install <ver> --profile minimal -c clippy -c rustfmt
cargo +<ver> clippy --workspace --all-targets --all-features -- -D warnings
```

Find `<ver>` in the latest CI log line `rustc X.Y.Z`.

**Fix style:** prefer toolchain-neutral code (e.g. a `compare_exchange` loop) over the new
API name when the default toolchain lacks it, so it is warning-free on both.
