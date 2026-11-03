# planspec

Workplane verifier: read-only posture checks over work-decomposition
specs. `planspec` reads a workplan spec (`plan-spec.toml`) and a profile
(`planspec.toml`), resolves the evidence each task cites against the git
repository, and reports its verdict in exit codes. It never writes a
file and never mutates a repository.

## Commands

```text
planspec verify  [--spec P] [--profile P] [--repo P] [--strict] [--json]
planspec status  [--spec P] [--profile P] [--repo P] [--json] [--md] [--check]
planspec scaffold --id ID --title TITLE [--from-table PATH]
planspec help [commands|exit-codes|configuration|diagnostics]
```

`verify` is the gate: any named-gate finding exits 1. `status` is a
display, not a gate: it exits 0 even when tasks are STALE. `scaffold`
prints a schema-valid workplan skeleton to stdout; it never writes a
file. `planspec help commands` carries the full command manual and
`planspec help exit-codes` carries the exit-code contract.

## Install

Build from a checkout of this repository:

```sh
cargo install --path .
```

## Tests

```sh
cargo test
```

The payload runner `./run-unit-tests.sh` executes the same suite plus the
clippy gate.

## Documentation

- `docs/capability-matrix.md` — every command, the exit code it produces,
  and the test that pins it.
- `planspec help` — the built-in manual, owned by the binary itself.

## License

Apache-2.0. See `LICENSE`.
