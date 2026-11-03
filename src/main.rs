//! planspec CLI entry point: files in, exit codes out.
//!
//! Exit codes (family exit taxonomy):
//! - 0: green / verdicts produced.
//! - 1: gate red (one named-gate line per failure).
//! - 64: usage error — bad arguments, unknown command, unknown flag,
//!   `help` typo (UNIVERSAL family code).
//! - 70: operational refusal — spec or profile missing or unreadable, or
//!   the resolved repo is not a git repository (refused: evidence must
//!   never resolve somewhere unknown). Fail-closed: never answered as a
//!   verdict.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::Serialize;

use planspec::config::Profile;
use planspec::gates::{self, Finding};
use planspec::model::Workplan;
use planspec::verdict::{self, TaskVerdict};

const EXIT_GREEN: i32 = 0;
const EXIT_RED: i32 = 1;
const EXIT_OPERATIONAL: i32 = 70;
const EXIT_USAGE: i32 = 64;

#[derive(Parser)]
#[command(
    name = "planspec",
    version,
    about = "Read-only workplane verifier: workplan specs in, verdicts out",
    // `help` is this tool's own manual subcommand below, not clap's generated
    // one, so the built-in manual owns the name. clap's --help/--version stay.
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify a workplan spec against the gates.
    Verify {
        /// Workplan spec file (default: ./plan-spec.toml).
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Project profile file (default: ./planspec.toml, else built-in defaults).
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Project repo root for evidence resolution. Wins over the
        /// profile file's directory; never the process CWD's own repo.
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Also fail tasks that cite no evidence at all.
        #[arg(long)]
        strict: bool,
        /// Emit findings as JSON on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Show derived per-task verdicts (statuses are derived, never declared).
    /// This is not a gate: it exits 0 even when tasks are STALE —
    /// except --check, the freshness mode, which exits 1 on any STALE row.
    Status {
        /// Workplan spec file (default: ./plan-spec.toml).
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Project profile file (default: ./planspec.toml, else built-in defaults).
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Project repo root for evidence resolution. Wins over the
        /// profile file's directory; never the process CWD's own repo.
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Emit verdicts as JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Emit the whole ledger as one Markdown table.
        #[arg(long)]
        md: bool,
        /// Freshness mode: exit 1 when any row is STALE, 0 when fresh.
        #[arg(long)]
        check: bool,
    },
    /// Print a schema-valid workplan skeleton, or convert a markdown
    /// status table (--from-table).
    Scaffold {
        /// Plan id.
        #[arg(long)]
        id: String,
        /// Plan title.
        #[arg(long)]
        title: String,
        /// Markdown file whose pipe-table rows become [[task]] blocks.
        /// Rows without an id cell get minted ids row-<n>; identical rows
        /// deduplicate with a note. No evidence is invented.
        #[arg(long)]
        from_table: Option<PathBuf>,
    },
    /// Print the built-in manual. Without a topic, print the topic index.
    Help {
        /// One of: commands, exit-codes, configuration, diagnostics.
        topic: Option<String>,
    },
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // Help and version print cleanly and exit 0; anything else
            // the parser rejects is a usage error (exit 64).
            let _ = err.print();
            return if err.use_stderr() { EXIT_USAGE } else { EXIT_GREEN };
        }
    };
    match cli.command {
        // The manual is pure output. Answer it before any spec, profile, or
        // repository access, so a `help` run has no side effects.
        Command::Help { topic } => match planspec::help::run(topic.as_deref()) {
            Ok(()) => EXIT_GREEN,
            Err(msg) => {
                eprintln!("error: {msg}");
                EXIT_USAGE
            }
        },
        Command::Verify {
            spec,
            profile,
            repo,
            strict,
            json,
        } => verify(
            spec.as_deref(),
            profile.as_deref(),
            repo.as_deref(),
            strict,
            json,
        ),
        Command::Status {
            spec,
            profile,
            repo,
            json,
            md,
            check,
        } => status(
            spec.as_deref(),
            profile.as_deref(),
            repo.as_deref(),
            json,
            md,
            check,
        ),
        Command::Scaffold {
            id,
            title,
            from_table,
        } => match from_table {
            None => {
                print!("{}", Workplan::skeleton(&id, &title));
                EXIT_GREEN
            }
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(text) => {
                    print!("{}", Workplan::from_markdown_table(&id, &title, &text));
                    EXIT_GREEN
                }
                Err(e) => {
                    eprintln!("error: cannot read {}: {e}", path.display());
                    EXIT_OPERATIONAL
                }
            },
        },
    }
}

/// Load the spec, or exit 70 (operational: the input is missing or unreadable).
fn load_spec(spec: Option<&Path>) -> Result<Workplan, i32> {
    let path = spec.unwrap_or(Path::new("plan-spec.toml"));
    Workplan::load(path).map_err(|e| {
        eprintln!("error: {e}");
        EXIT_OPERATIONAL
    })
}

/// Load the profile: explicit path must exist; the default file is
/// optional. Discovery order for the default: ./planspec.toml first,
/// then the resolution root's planspec.toml (the committed project
/// profile), then built-in defaults. Any problem with a present file
/// exits 70.
fn load_profile(profile: Option<&Path>, root: &Path) -> Result<Profile, i32> {
    let load = |path: &Path| -> Result<Profile, i32> {
        Profile::load(path).map_err(|e| {
            eprintln!("error: {e}");
            EXIT_OPERATIONAL
        })
    };
    match profile {
        Some(path) => load(path),
        None => {
            let cwd_default = Path::new("planspec.toml");
            if cwd_default.exists() {
                return load(cwd_default);
            }
            let root_default = root.join("planspec.toml");
            if root_default.exists() {
                return load(&root_default);
            }
            Ok(Profile::default())
        }
    }
}

/// Sweep mode: every COMMITTED `*.planspec.toml` in the repo, sorted.
/// Uses `git ls-files` (the index), so uncommitted scratch files can
/// neither abort the gate with parse noise nor fake it with extra rows.
/// Outside a git repository nothing is discovered; the caller then
/// reports "no spec found" (exit 70), which is honest for a bare run.
fn sweep_specs(root: &Path) -> Vec<PathBuf> {
    verdict::ls_specs(root)
}

/// Spec discovery shared by verify and status: an explicit --spec wins;
/// else the v0.1 default ./plan-spec.toml when present; else sweep the
/// repo for every committed *.planspec.toml (the bare-invocation mode).
fn discover_specs(root: &Path, spec: Option<&Path>) -> Result<Vec<PathBuf>, i32> {
    match spec {
        Some(path) => Ok(vec![path.to_path_buf()]),
        None => {
            if Path::new("plan-spec.toml").exists() {
                return Ok(vec![PathBuf::from("plan-spec.toml")]);
            }
            let found = sweep_specs(root);
            if found.is_empty() {
                eprintln!(
                    "error: no spec found: pass --spec, keep ./plan-spec.toml, \
                     or commit *.planspec.toml files to the repo"
                );
                return Err(EXIT_OPERATIONAL);
            }
            Ok(found)
        }
    }
}

/// Evidence resolution root. An explicit `--repo` wins; else the
/// directory of an explicit profile file — the profile declares its
/// project, so evidence resolves against that repo, never the CWD's.
/// With neither declaration the CWD's git repository root serves as
/// root (`git rev-parse --show-toplevel`), so a bare run from any
/// subdirectory sweeps and resolves against the same committed specs
/// as a run from the root; outside a repository the CWD itself is the
/// root (filesystem-only mode).
fn resolution_root(repo: Option<&Path>, profile: Option<&Path>) -> PathBuf {
    if let Some(repo) = repo {
        return repo.to_path_buf();
    }
    if let Some(profile) = profile {
        return match profile.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
    }
    match verdict::git_toplevel(Path::new(".")) {
        Some(toplevel) => toplevel,
        None => PathBuf::from("."),
    }
}

/// True when any task cites git evidence (commit, commits, git-message),
/// so a git repository is needed.
fn cites_commits(plan: &Workplan) -> bool {
    plan.task.iter().any(|task| {
        task.evidence.as_ref().is_some_and(|evidence| {
            evidence.commit.is_some()
                || !evidence.commits.is_empty()
                || evidence.git_message.is_some()
        })
    })
}

fn verify(
    spec: Option<&Path>,
    profile: Option<&Path>,
    repo: Option<&Path>,
    strict: bool,
    json: bool,
) -> i32 {
    let root = resolution_root(repo, profile);
    let specs = match discover_specs(&root, spec) {
        Ok(specs) => specs,
        Err(code) => return code,
    };
    let profile = match load_profile(profile, &root) {
        Ok(profile) => profile,
        Err(code) => return code,
    };
    let grammar = match profile.grammar() {
        Ok(grammar) => grammar,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_OPERATIONAL;
        }
    };

    let mut plans = Vec::new();
    for path in &specs {
        match load_spec(Some(path.as_path())) {
            Ok(plan) => plans.push((path.clone(), plan)),
            Err(code) => return code,
        }
    }
    if plans
        .iter()
        .any(|(_, plan)| cites_commits(plan))
        && let Err(e) = verdict::require_repo(&root)
    {
        eprintln!("error: {e}");
        return EXIT_OPERATIONAL;
    }

    let mut findings = Vec::new();
    let mut task_count = 0usize;
    for (path, plan) in &plans {
        let multi = specs.len() > 1;
        let label = move |mut finding: Finding| -> Finding {
            if multi {
                finding.message = format!("[{}] {}", path.display(), finding.message);
            }
            finding
        };
        task_count += plan.task.len();
        findings.extend(gates::check_schema(plan, &grammar).into_iter().map(&label));
        findings.extend(gates::check_dag(plan).into_iter().map(&label));
        findings.extend(gates::check_cites(plan).into_iter().map(&label));
        findings
            .extend(verdict::freshness_findings(&root, &profile, plan).into_iter().map(&label));
        if strict {
            // A Pending row with a human-declared note states why it has
            // no evidence (a declared skip, a clarification) and is not
            // red; a noteless Pending stays red until such a note is
            // declared. A scaffold-generated dedup note is NOT clearance:
            // a human must edit or replace it before the row passes.
            findings.extend(
                plan.task
                    .iter()
                    .filter(|task| {
                        !task
                            .evidence
                            .as_ref()
                            .is_some_and(|evidence| evidence.claims_done())
                            && (task.note.trim().is_empty() || task.note_generated)
                    })
                    .map(|task| {
                        let why = if task.note_generated {
                            "carries only a scaffold-generated note — replace it with a human note"
                        } else {
                            "or declare a note"
                        };
                        Finding::new(
                            "freshness",
                            format!("task '{}' cites no evidence (MISSING) — {why}", task.id),
                        )
                    })
                    .map(&label),
            );
            let verdicts = verdict::verdicts(&root, &profile, plan);
            findings.extend(verdict::phase_findings(plan, &verdicts).into_iter().map(label));
            findings
                .extend(verdict::phase_completeness_findings(plan).into_iter().map(&label));
            // Legacy free-text log cites still resolve but warn under --strict.
            for task in plan.task.iter().filter(|task| {
                task.evidence
                    .as_ref()
                    .is_some_and(|evidence| evidence.log.is_some() && evidence.file.is_none())
            }) {
                eprintln!(
                    "warning: task '{}' rests on a legacy free-text log cite — migrate it to file+contains",
                    task.id
                );
            }
        }
    }

    let green = findings.is_empty();
    if json {
        let report = VerifyReport {
            command: "verify",
            green,
            findings,
        };
        println!("{}", serde_json::to_string_pretty(&report).expect("report serializes"));
    } else if green {
        if specs.len() == 1 {
            println!("planspec verify: GREEN ({task_count} tasks)");
        } else {
            println!("planspec verify: GREEN ({} specs, {task_count} tasks)", specs.len());
        }
    } else {
        for finding in &findings {
            println!("{finding}");
        }
    }
    if green {
        EXIT_GREEN
    } else {
        EXIT_RED
    }
}

fn status(
    spec: Option<&Path>,
    profile: Option<&Path>,
    repo: Option<&Path>,
    json: bool,
    md: bool,
    check: bool,
) -> i32 {
    let root = resolution_root(repo, profile);
    let specs = match discover_specs(&root, spec) {
        Ok(specs) => specs,
        Err(code) => return code,
    };
    let profile = match load_profile(profile, &root) {
        Ok(profile) => profile,
        Err(code) => return code,
    };
    let mut plans = Vec::new();
    for path in &specs {
        match load_spec(Some(path.as_path())) {
            Ok(plan) => plans.push((path.clone(), plan)),
            Err(code) => return code,
        }
    }
    if plans
        .iter()
        .any(|(_, plan)| cites_commits(plan))
        && let Err(e) = verdict::require_repo(&root)
    {
        eprintln!("error: {e}");
        return EXIT_OPERATIONAL;
    }
    // One verdict list per spec, in discovery order.
    let derived: Vec<(PathBuf, Vec<TaskVerdict>)> = plans
        .iter()
        .map(|(path, plan)| (path.clone(), verdict::verdicts(&root, &profile, plan)))
        .collect();

    // Freshness mode is the one status invocation that is a gate: it
    // regenerates the --md ledger per spec and byte-compares it against
    // the committed <stem>.status.md artifact beside the spec (the
    // archspec-artifact pattern). A missing artifact, any ledger drift,
    // or a spec with zero task rows exits 1 — an empty table is never
    // "fresh".
    if check {
        let mut stale = false;
        let mut rows = 0usize;
        for ((path, plan), (_, tasks)) in plans.iter().zip(derived.iter()) {
            rows += plan.task.len();
            if plan.task.is_empty() {
                println!(
                    "status STALE: {} has no task rows — an empty ledger is not fresh",
                    path.display()
                );
                stale = true;
                continue;
            }
            let artifact = status_artifact(path);
            let rendered = render_md(plan, tasks);
            match std::fs::read_to_string(&artifact) {
                Ok(committed) if committed == rendered => {}
                Ok(_) => {
                    println!(
                        "status STALE: ledger differs from committed artifact {} — regenerate with `planspec status --md`",
                        artifact.display()
                    );
                    stale = true;
                }
                Err(_) => {
                    println!(
                        "status STALE: no committed ledger artifact at {} — commit it from `planspec status --md`",
                        artifact.display()
                    );
                    stale = true;
                }
            }
        }
        if stale {
            println!("status stale: ledger artifacts need regeneration");
            return EXIT_RED;
        }
        println!("status fresh: {rows} ledger rows verified against committed artifacts");
        return EXIT_GREEN;
    }

    if json {
        let all: Vec<TaskVerdict> = derived
            .iter()
            .flat_map(|(_, tasks)| tasks.iter().cloned())
            .collect();
        let report = StatusReport {
            command: "status",
            tasks: &all,
        };
        println!("{}", serde_json::to_string_pretty(&report).expect("report serializes"));
        return EXIT_GREEN;
    }

    for (path, plan) in &plans {
    if specs.len() > 1 {
        println!("== {}", path.display());
    }
    let tasks = verdict::verdicts(&root, &profile, plan);
    if md {
        // The canonical ledger table: one row per task and tombstone.
        print!("{}", render_md(plan, &tasks));
    } else {
        for task in &tasks {
        let note = task
            .note
            .as_deref()
            .map(|n| format!(" (note: {n})"))
            .unwrap_or_default();
        match task.state {
            verdict::TaskState::Done => println!("{} done", task.id),
            verdict::TaskState::Stale => {
                println!("{} STALE: {}", task.id, task.notes.join("; "));
            }
            verdict::TaskState::Pending => println!("{} pending{note}", task.id),
            verdict::TaskState::Parked => {
                println!("{} parked (id reserved; no split-into recorded)", task.id);
            }
            verdict::TaskState::Superseded => {
                println!(
                    "{} superseded (split-into: {})",
                    task.id,
                    task.cites.join(" + ").replace("split-into: ", "")
                );
            }
        }
    }
    // Status is a display, not a gate: it exits 0 even with stale tasks.
    // Loud warning keeps that contract visible; `verify` is the gate.
    let stale = tasks
        .iter()
        .filter(|task| task.state == verdict::TaskState::Stale)
        .count();
    if stale > 0 {
        eprintln!(
            "warning: {stale} task(s) STALE — status is not a gate; use `planspec verify` to gate"
        );
    }
    }
    }
    EXIT_GREEN
}

/// The canonical ledger table for one spec: header, one row per task,
/// one row per tombstone. `status --md` prints it; `status --check`
/// regenerates it and byte-compares against the committed artifact,
/// so both paths must render from this single source.
fn render_md(plan: &Workplan, tasks: &[TaskVerdict]) -> String {
    let escape = |text: &str| text.replace('|', "\\|").replace('\n', " ");
    let mut out = String::new();
    out.push_str("| phase | id | task | cite | verdict | note |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for task in &plan.task {
        let derived = tasks.iter().find(|v| v.id == task.id);
        let (verdict, cites, note) = match derived {
            Some(v) => (state_str(v.state), v.cites.join(" + "), v.note.clone()),
            None => ("pending".to_string(), String::new(), None),
        };
        let phase = match task.phase {
            Some(number) => match plan.phase.iter().find(|p| p.number == number) {
                Some(p) if p.title.is_empty() => format!("P{number}"),
                Some(p) => format!("P{} {}", p.number, p.title),
                None => format!("P{number}"),
            },
            None => "-".to_string(),
        };
        out.push_str(&format!(
            "| {phase} | {} | {} | {} | {verdict} | {} |\n",
            task.id,
            escape(&task.behavior),
            if cites.is_empty() { "-".to_string() } else { escape(&cites) },
            note.map(|n| escape(&n)).unwrap_or_default(),
        ));
    }
    for tombstone in &plan.tombstone {
        let derived = tasks.iter().find(|v| v.id == tombstone.id);
        let verdict = derived.map(|v| state_str(v.state)).unwrap_or("superseded".to_string());
        let cites = derived
            .map(|v| v.cites.join(" + "))
            .unwrap_or_else(|| "split-into: ".to_string() + &tombstone.split_into.join(", "));
        out.push_str(&format!(
            "| - | {} | tombstone | {} | {verdict} |  |\n",
            tombstone.id,
            escape(&cites),
        ));
    }
    out
}

/// Committed ledger artifact beside a spec: `foo.planspec.toml` maps to
/// `foo.status.md` (any other `*.toml` drops its suffix the same way).
fn status_artifact(spec: &Path) -> PathBuf {
    let name = spec
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = name
        .strip_suffix(".planspec.toml")
        .or_else(|| name.strip_suffix(".toml"))
        .unwrap_or(&name);
    spec.with_file_name(format!("{stem}.status.md"))
}

/// Lowercase verdict name for the ledger table.
fn state_str(state: verdict::TaskState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "pending".to_string())
}

#[derive(Serialize)]
struct VerifyReport {
    command: &'static str,
    green: bool,
    findings: Vec<Finding>,
}

#[derive(Serialize)]
struct StatusReport<'a> {
    command: &'static str,
    tasks: &'a [TaskVerdict],
}
