# rshome SDK

This repository is the public, source-only rshome SDK workspace. It contains
the SDK crates listed in the versioned contract and the private implementation
closure they require to compile.

The repository has one clean Git root. Do not import historical repositories,
deployment material, device firmware, operational evidence, credentials, or
private runtime components.

## Verification

Run the static public-release gates from a fresh clone:

```sh
bash scripts/verify-public-history.sh --require-history
bash scripts/verify-public-boundary.sh
bash scripts/verify-sdk-contract.sh
```

`scripts/release-sdk-metadata.sh` records the clean source-root revision in a
separate release-metadata commit. The published-release contract then verifies
that the pinned revision remains that source root.
