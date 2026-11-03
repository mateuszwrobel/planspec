# planspec capability matrix

Every command, the exit code it produces, and the test that proves it. This
page is a map of the contract, not a restatement of behavior: the binary owns
each claim (`planspec help commands`, `planspec help exit-codes`), and this
table points at the golden test that pins it.

## Commands

| Command | What it does |
|---------|--------------|
| `verify [--spec P] [--profile P] [--repo P] [--strict] [--json]` | Verify a workplan spec against the named gates (schema, dag, cite-resolution, phase). With no `--spec` and no `./plan-spec.toml`, sweeps the repo for every committed `*.planspec.toml` (tracked in the git index — uncommitted scratch is invisible to the sweep) and verifies each. A bare run resolves the repository root via `git rev-parse --show-toplevel`, so any subdirectory sees the same specs. |
| `status [--spec P] [--profile P] [--repo P] [--json] [--md] [--check]` | Derived per-task verdicts. Shares `verify`'s spec discovery, sweep included. A display, not a gate: exits 0 even when tasks are STALE. `--md` prints the ledger as one Markdown table; `--check` regenerates that table per spec and byte-compares it against the committed `<stem>.status.md` artifact — missing artifact, drift, or zero task rows exit 1. |
| `scaffold --id ID --title TITLE [--from-table P]` | Print a schema-valid workplan skeleton, or convert a markdown status table (`--from-table`: minted `row-<n>` ids, deduplicated identical rows with a generated note that `--strict` rejects until a human edits it, no invented evidence). Never writes a file. |
| `help [topic]` | Print the built-in manual (`commands`, `exit-codes`, `configuration`, `diagnostics`). |

## Command × exit code × tested-in

| Command | Exit | Meaning | Proven by |
|---------|-----:|---------|-----------|
| `verify` (green) | 0 | Every task's evidence resolves. | `golden_all_green_exits_zero`, `golden_tombstone_keeps_child_cites_valid` |
| `verify` (gate red) | 1 | A named-gate finding: dag cycle, STALE cite, unknown check cite. | `golden_cycle_exits_one`, `golden_missing_commit_is_stale_red`, `golden_unknown_check_cite_exits_one`, `foreign_cwd_commit_cite_is_stale` |
| `verify --strict` | 1 | A task citing no evidence and declaring no note also fails. | `strict_fails_tasks_without_evidence` |
| `verify --strict` (phase) | 1 | Done work in a later declared phase while an earlier phase is unfinished; a task without a phase in a plan that declares phases; a `phase` no `[[phase]]` declares. | `strict_refuses_done_work_ahead_of_an_unfinished_phase`, `strict_requires_a_phase_when_the_plan_declares_phases` |
| `verify --strict` (note) | 0 | A Pending row with a declared human note states its non-completion and clears the strict evidence rule; a scaffold-generated dedup note does not clear it. | `note_clears_strict_for_a_row_without_evidence`, `scaffold_from_table_mints_ids_and_dedups` |
| `verify --strict` (legacy log) | 0 | A done row resting on the legacy free-text log cite still resolves but warns. | `strict_warns_on_legacy_free_text_log_cites` |
| `verify` (multi-commit cite) | 0/1 | Every hash in `commits` must be an ancestor of the trunk; the first unresolvable one is named. | `multi_commit_cite_resolves_or_names_the_first_missing` |
| `verify` (git-message cite) | 0/1 | The cite must equal the full subject of exactly one first-parent commit; body mentions never count; zero or many subject matches are STALE, many name every hash. Off-first-parent duplicates are invisible by design. | `git_message_cite_is_a_subject_anchor_not_a_substring`, `git_message_body_mention_never_keeps_a_dead_cite_green`, `git_message_duplicate_subject_off_first_parent_is_invisible` |
| `verify` (file+contains cite) | 0/1 | The exact path (inside the profile's log roots when configured) must carry the exact literal. | `structured_file_cite_resolves_under_log_roots` |
| `verify` (bare sweep) | 0/1 | No `--spec`: every committed (git-index-tracked) `*.planspec.toml` is verified; uncommitted scratch neither aborts nor fakes the sweep; a run from a subdirectory resolves the repo root. | `bare_verify_sweeps_committed_specs`, `bare_verify_from_a_subdirectory_resolves_the_repo_root` |
| `verify --json` | 0/1 | Findings emitted as JSON on stdout. | `verify_json_reports_findings` |
| `verify` (bad spec/profile) | 70 | Spec or profile missing or unreadable — operational refusal, fail-closed. | `golden_invalid_spec_exits_seventy` |
| `verify` (not a repo) | 70 | The resolved root is not a git repository — refused (operational, never a verdict). | `non_repo_root_is_refused` |
| `verify` (evidence root) | 0 | `--repo` wins; else an explicit profile's directory; else the git repository root holding the CWD. | `repo_flag_wins_over_cwd`, `profile_declared_repo_wins_over_cwd` |
| `status` | 0 | Verdicts derived and printed — even when some are STALE, with a loud warning. Tombstones close the vocabulary: Superseded (has children), Parked (reserved, none yet). | `status_derives_verdicts`, `status_all_stale_exits_zero_with_loud_warning` |
| `status --md` | 0 | The whole ledger as one Markdown table. | `status_md_is_the_ledger_table` |
| `status --check` | 0/1 | Generate-and-compare freshness: regenerates the `--md` ledger per spec and byte-compares it against the committed `<spec-stem>.status.md`; missing artifact, any drift, or a zero-task spec exits 1. | `status_check_compares_the_committed_ledger_artifact`, `status_check_refuses_an_empty_ledger` |
| `scaffold` | 0 | A skeleton that `verify` itself accepts. | `scaffold_output_is_verify_green` |
| `scaffold --from-table` | 0 | Row conversion: minted ids, dedup with a generated note, schema-valid output. | `scaffold_from_table_mints_ids_and_dedups` |
| `help <topic>` | 0 | A known topic printed its page; the index prints with no topic. | `help_topics_exit_zero_unknown_exits_sixty_four` |
| `help <unknown>` | 64 | The named topic is not in the manual — a usage error. | `help_topics_exit_zero_unknown_exits_sixty_four` |
| (wrong command line) | 64 | An unknown command or flag — the UNIVERSAL family usage code. | `golden_usage_error_exits_sixty_four` |
| `--help` / `--version` | 0 | Help and version print and exit 0. | `golden_usage_error_exits_sixty_four` |

## Exit-code contract

Exit codes are defined once in `main.rs` (`EXIT_GREEN`, `EXIT_RED`,
`EXIT_USAGE`, `EXIT_OPERATIONAL`) — binary-internal, not library surface.
The taxonomy is the family contract: 64 usage is universal, 70 is the
operational-precondition refusal. See `planspec help exit-codes` for the
same contract from the binary itself. An unknown `help` topic exits **64**:
planspec's single usage code `EXIT_USAGE` — a typo is a usage error, and it
can never imitate a verdict code.
