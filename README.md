# proton-drive-sync-engine

Rust split-architecture prototype for a bidirectional Proton Drive sync daemon (`proton-syncd`) and control CLI (`proton-sync`).

## Binaries

- `proton-syncd`: background daemon that watches a local directory, persists a SQLite index, listens on a Unix socket, and reconciles changes with `proton-drive`.
- `proton-sync`: companion control CLI with `status`, `pause`, `resume`, and `syncnow` commands.

## Quick start

```bash
cargo run --bin proton-syncd -- \
  --local-root /path/to/local/folder \
  --remote-root /Drive/RemoteFolder

cargo run --bin proton-sync -- status
```

The daemon stores its SQLite index in `sync_index.db` under the local root by default and uses `/tmp/proton-sync.sock` for IPC.
