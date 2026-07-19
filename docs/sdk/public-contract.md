# Public SDK contract

The public API manifest contains exactly these nine Git-consumable crates:

- `rshome-schema`
- `rshome-config`
- `rshome-codegen`
- `rshome-mcp`
- `rshome-actor`
- `rshome-entity`
- `rshome-state`
- `rshome-svc`
- `rshome-wizard`

`rshome-device-link`, `rshome-native-api`, `rshome-wasm-host`, and `rshome-wf`
are workspace-only source closure crates. They are not public contract entries.

`Cargo.lock` is a controlled release artifact generated and validated on the Linux
build host. It is required for reproducible `cargo metadata --locked` and SDK tests.

The release record must pin only the repository's single source-root commit;
the release metadata commit and version tag may follow it.
