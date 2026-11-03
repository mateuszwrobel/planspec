//! The built-in manual: `planspec help [topic]`.
//!
//! Pure output only. This module reads no spec file, no profile, no git
//! repository, and spawns no process. The caller dispatches `help` before
//! anything is loaded, so a help run has no side effects. A topic that exists
//! prints its text and exits 0; an unknown topic is an error the caller maps
//! to the usage exit code (64).

/// The topic index printed by `planspec help` with no topic, and by
/// `planspec help topics`.
pub const HELP: &str = "\
usage: planspec help [topic]
planspec built-in manual: the commands, the exit-code contract, the
planspec.toml profile, and the diagnostics catalog.

topics:
  commands       every command with its purpose and flags
  exit-codes     what every exit code means and which command emits it
  configuration  the plan-spec.toml and planspec.toml files and their keys
  diagnostics    every gate name and STALE notice: meaning, cause, and the fix

run 'planspec help <topic>' for a topic
";

/// The commands topic: every subcommand with its purpose and flags.
pub const COMMANDS: &str = "\
# planspec help commands — the command surface
usage: planspec <command> [flags]

commands:
  verify [--spec PATH] [--profile PATH] [--repo PATH] [--strict] [--json]
              Verify a workplan spec against the gates. Gate red exits 1;
              a missing or unreadable spec or profile exits 70. Default spec is
              ./plan-spec.toml; when it is absent the repo is swept for
              every committed *.planspec.toml (tracked in the git index;
              uncommitted scratch is invisible to the sweep) and each is
              verified. A bare run resolves the repository root with
              `git rev-parse --show-toplevel`, so any subdirectory sees
              the same committed specs as the root. Default profile is
              ./planspec.toml, else the repo root's planspec.toml, else
              built-in defaults. --strict also fails tasks that cite no
              evidence and declare no human note (a scaffold-generated
              dedup note is not clearance), refuses done work in a later
              phase while an earlier declared phase is unfinished, and
              requires every task to declare a phase when the plan
              declares any. It warns on legacy free-text log cites.
  status [--spec PATH] [--profile PATH] [--repo PATH] [--json] [--md] [--check]
              Show derived per-task verdicts. Statuses are DERIVED from
              evidence, never declared. This is not a gate: it exits 0 even
              when tasks are STALE, and prints a warning when any are.
              --md emits the whole ledger as one Markdown table. --check is
              the freshness mode: it regenerates that table per spec and
              byte-compares it against the committed ledger artifact
              <spec-stem>.status.md beside the spec (the archspec-artifact
              pattern). A missing artifact, any ledger drift, or a spec
              with zero task rows exits 1.
  scaffold --id ID --title TITLE [--from-table PATH]
              Print a schema-valid workplan skeleton to stdout. It never
              writes a file; redirect if you want one. --from-table reads a
              markdown file and turns every pipe-table data row into a
              [[task]] block: rows without an id cell get minted ids
              row-<n>, identical rows deduplicate with a generated note
              (a human must edit it before --strict accepts the row), and
              no evidence is invented — cite each task before gating.
  help [topic]
              Print this manual. Known topics exit 0; an unknown topic
              exits 64 like any other usage error.

evidence resolution root:
  --repo wins; else the directory of an explicit --profile file; else the
  root of the git repository holding the current directory (falling back
  to the CWD outside a repository). Evidence never resolves against a
  partial subtree when a profile or repository declares the project.
";

/// The exit-codes topic: the contract every command obeys.
pub const EXIT_CODES: &str = "\
# planspec help exit-codes — the contract

| Code | Meaning                                                     |
|-----:|-------------------------------------------------------------|
|    0 | Verdicts produced, or the manual printed.                    |
|    1 | Gate red: one named-gate line per failure on stdout.         |
|   64 | Usage error: bad arguments, an unknown command or flag, or   |
|      | `help` naming a topic the manual does not have.              |
|   70 | Operational refusal (fail-closed): a spec or profile is      |
|      | missing or unreadable, or the resolved repo is not a git     |
|      | repository (refused: evidence must never resolve somewhere   |
|      | unknown). Never answered as a verdict.                       |

Notes:
- `status` is a display, not a gate: it exits 0 even when tasks are STALE.
- The one exception is `status --check`: the freshness mode exits 1 when
  a committed ledger artifact is missing, differs from the regenerated
  table, or guards a spec with zero task rows.
- `status` shares `verify`'s spec discovery: with no `--spec` and no
  `./plan-spec.toml`, it sweeps the repo for every committed
  `*.planspec.toml` (tracked in the git index) and reports each,
  labeling rows with the spec path when more than one spec is in play.
- `verify` is the gate: any finding on any named gate exits 1.
- `--help` and `--version` print and exit 0.
";

/// The configuration topic: both input files and their keys.
pub const CONFIGURATION: &str = "\
# planspec help configuration — the input files

plan-spec.toml — the workplan spec (what is planned and what proves it):
  [plan]        id, title, and optional source-cites.
  [[phase]]     optional phase declarations: a number (the only order the
                tool honours) and a title. Tasks cite a phase by number.
                Deleting every [[phase]] block is the legal opt-out from
                the phase gate — it then checks nothing, completeness
                included.
  [[task]]      one task: id (profile id-grammar), behavior, depends-on,
                done-when, optional phase (must name a declared [[phase]];
                when the plan declares any [[phase]], --strict requires a
                phase on every task), optional note (a declared human
                annotation: it never lowers a resolving Done, and it
                satisfies --strict for a Pending row; a scaffold-generated
                dedup note does not), and an optional [task.evidence]
                table with commit, commits (a list: all must resolve),
                git-message (equals the full subject of exactly one commit
                on the trunk's first-parent history — see the contract in
                the profile section), check, log (the legacy free-text
                form), file + contains (the structured pair: exact path
                plus exact literal), and sentinel cites. Status is never
                declared; it is derived from the cites.
  [[tombstone]] a retired task id and its optional split-into list, so
                children may cite the retired id. With children it reads
                Superseded; without any yet it reads Parked.
  Unknown keys are refused, never ignored.

planspec.toml — the project profile (where proof lives); all keys optional:
  trunk-ref      git ref cites must be an ancestor of (default: HEAD).
                 Git-message cites search this ref's FIRST-PARENT history
                 only, matching the full subject line of a commit
                 (`git log --format=%s`, anchored equality). Two
                 consequences: body text never counts as a match, and
                 commits that never sit on the first-parent chain (the
                 second-parent side of a merge) are invisible — an
                 off-trunk duplicate subject stays GREEN by documented
                 design.
  check-spec     file listing the valid check names (default: checks.toml).
  log-dir        directory holding cited log files (default: logs).
  log-roots      directories structured file cites may resolve under,
                 besides log-dir itself; an empty list (the v0.1 default)
                 leaves file cites resolved by exact path from the root,
                 with only the root-escape refusal.
  sentinel-glob  glob naming completion sentinels (default: empty).
  id-grammar     regex every task id must match (default: the plan id
                 grammar, alphanumerics plus . _ -).
  Unknown keys are refused, never ignored.

A present-but-broken profile exits 70. The default profile file is optional;
an explicit --profile path must exist.
";

/// The diagnostics topic: every gate name and STALE notice.
pub const DIAGNOSTICS: &str = "\
# planspec help diagnostics — gates and STALE notices

Gates (verify prints one line per finding, then exits 1):
  schema          a key, id, or shape the spec grammar refuses, including
                  a duplicate phase number.
  dag             a depends-on cycle or a dependency on an unknown task.
  cite-resolution a cite the resolver cannot honour (also the gate name
                  for STALE verdicts): path escapes, a file cite without
                  its contains pair, an empty cite, or a phase number no
                  [[phase]] declares.
  phase           done work in a later declared phase while an earlier
                  declared phase still has unfinished tasks, or a task
                  without a phase in a plan that declares phases (--strict).

STALE notices (a cite exists but does not prove the task; verify exits 1,
status reports it and exits 0):
  'commit cite <c> is not a valid rev'     git refuses the cite outright.
  'commit cite <c> ... no such object'     the object is absent from the
                                           resolved repository.
  'commit cite <c> is not an ancestor of <trunk-ref>'  the commit exists
                                           but never landed on the trunk.
  'trunk ref <r> ... no such ref'          the profile names a ref the
                                           repository does not have.
  '<dir> is not a git repository'          evidence would resolve
                                           somewhere unknown.
  'git-message cite <m> matches no commit subject'  no commit on the
                                           trunk's first-parent history
                                           carries that FULL SUBJECT. The
                                           cite must equal the whole
                                           subject line; a body mention
                                           proves nothing.
  'git-message cite <m> is the subject of N commits'  N first-parent
                                           commits share the subject;
                                           the hash list is named — make
                                           the landing subject unique.
  'check cite <c> is not in check-spec list' the check is undeclared.
  'log cite <name> escapes the project root' the log path leaves the repo.
  'log cite <name> has no file at <path>'  the log file is absent.
  'log cite <name> at <path> is unreadable' the file exists but cannot be
                                           read — a different fault than
                                           absence, named as such.
  'file cite <f> resolves outside the configured log-roots' the path sits
                                           outside the profile's allowlist.
  'file cite <f> has no file at <path>'    the structured cite names a
                                           path that does not exist.
  'file cite <f> at <path> does not contain the cited text'  the literal
                                           the cite promised is absent.

A task citing no evidence passes unless verify runs with --strict; under
--strict a note on the row declares why it has no evidence and clears it.
A done row resting on a legacy free-text log cite warns under --strict;
migrate it to file+contains.
";

/// Print a topic. `None` prints the topic index. An unknown topic name
/// returns the error text; the caller maps it to the usage exit code.
pub fn run(topic: Option<&str>) -> Result<(), String> {
    let text = match topic {
        None | Some("topics") => HELP,
        Some("commands") => COMMANDS,
        Some("exit-codes") => EXIT_CODES,
        Some("configuration") => CONFIGURATION,
        Some("diagnostics") => DIAGNOSTICS,
        Some(unknown) => {
            return Err(format!(
                "unknown help topic '{unknown}' — try: planspec help (topics index)"
            ));
        }
    };
    print!("{text}");
    Ok(())
}
