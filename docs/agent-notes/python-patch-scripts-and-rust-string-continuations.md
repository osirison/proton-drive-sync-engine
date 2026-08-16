---
trigger: python3 patch script, heredoc edit of .rs, Rust string line continuation, cargo fmt --all
depends_on: src/**/*.rs (any multi-line string literal)
recorded: 2026-08-16
---

# A patch script must use a raw string to write a Rust `\`-continuation

**Symptom:** a Rust string literal written by a Python patch script renders with a long run
of literal spaces in it, e.g.

```
the proton-drive CLI session is signed out or expired; run                                    `proton-drive login`
```

The source *looks* correct — `\` at the end of the line, continuation indented under it —
and `cat -A` on the committed file shows the `\` and the newline are simply gone, with the
indentation baked into the literal.

**Fix:** when a script writes Rust that contains a `\`-newline string continuation, the
Python string holding it must be raw (`r"""…"""`) or the backslash must be doubled
(`"…run \\\n"`). A bare `\` at the end of a line inside a non-raw Python string is a
*Python* line continuation and is consumed before the file is ever written — so `rustc`
never sees one, and the next line's indentation becomes part of the literal.

The same hazard applies to `\n`, `\t` and `\"` inside Rust literals being written by a
script. Raw strings are the default worth reaching for.

**Why it was not obvious:** nothing in this repo's gates can see it. `cargo fmt` reformats
around the literal but never inside it, `clippy -D warnings` has no lint for it, and every
test passed — the constant was compared against *itself*, which a mangled value satisfies as
happily as a clean one. It was found by an external code review reading the rendered string.

**Two habits that catch it:**

- After a scripted edit that touches a string literal, `grep -n '<const name>' -A2 file | cat -A`
  and read the actual bytes. `cat -A` is the tell; a plain read shows nothing wrong.
- Assert the **rendered** value, not the escape. The guard that now pins this one is
  `assert!(!AUTH_DECLINE_REASON.contains("  "))` in `daemon::tests` — a test comparing the
  constant to itself proves nothing about how it renders.

Related, same family: scripted replaces over already-formatted source can miss silently, so
verify *which* sites changed rather than trusting a replacement count.
