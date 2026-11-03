//! Evidence resolution: derives each task's state from the project
//! environment. Every kind must resolve for a claimed-done task; an
//! unresolvable cite is STALE and never passes silently.
//!
//! Kinds:
//! - commit: `cite` must be an ancestor of the profile's `trunk_ref`
//!   (`git merge-base --is-ancestor`, spawned with every GIT_* variable
//!   unset per the repo runner law).
//! - commits: every hash in the list must resolve like a commit cite;
//!   the first unresolvable one is named. An empty list is not a claim.
//! - git-message: the cite must EQUAL the full subject line of exactly
//!   one commit on the trunk ref's first-parent history
//!   (`git log --first-parent --format=%s`, anchored equality). Body
//!   text never counts. Zero or many subject matches are STALE; many
//!   name every matching hash. Off-first-parent duplicates are invisible
//!   by design (documented visibility limit).
//! - check: `cite` must appear in the profile's check-spec list.
//! - log: named file (may contain `{id}`) under `log_dir` whose contents
//!   contain the task id. This free-text form is the v0.1 legacy cite:
//!   it still resolves; `verify --strict` warns rows that rest on it.
//! - file + contains: one pair. `file` is the exact path from the
//!   project root (no `{id}` expansion); when the profile configures
//!   `log-roots`, the path must sit under one of them. `contains` is an
//!   exact literal the file must carry. One half without the other is
//!   refused by the cite gate before resolution.
//! - sentinel: the profile's `sentinel_glob` (may contain `{id}`) must
//!   match at least one file. An unset or empty glob makes a sentinel
//!   claim stale — it must never resolve, silent-DONE is illegal.
//!
//! Evidence resolves against the root the caller declares (the project
//! repo from the profile location or `--repo`), never the process CWD.
//!
//! The verdict vocabulary is a closed set: Done, Stale, Pending, Parked,
//! Superseded. Live tasks derive the first three; tombstone entries
//! derive the last two (a tombstone with children is Superseded — the
//! id was superseded by its split; a tombstone with no children recorded
//! yet is Parked — the id is reserved). A Pending row may declare a
//! `note` explaining its non-completion (a declared skip, a clarification);
//! `--strict` stays red on a noteless Pending until such a note is
//! declared. A note never lowers a resolving Done.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::config::Profile;
use crate::gates::{self, Finding};
use crate::model::{Task, Workplan};

/// Derived per-task state. Never declared by an author or an AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    /// Evidence present and every cite resolved.
    Done,
    /// Evidence present but at least one cite unresolvable.
    Stale,
    /// No evidence, or no claim of completion.
    Pending,
    /// Tombstone id reserved, no split recorded yet.
    Parked,
    /// Tombstone id superseded by its recorded split-into children.
    Superseded,
}

/// Derived verdict for one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskVerdict {
    pub id: String,
    pub state: TaskState,
    /// Every cite the row declares, rendered for the ledger table.
    pub cites: Vec<String>,
    /// The row's declared note, when present. It decorates a Pending
    /// fold; it never lowers a resolving Done.
    pub note: Option<String>,
    /// Why a cite did not resolve; empty when state is not `Stale`.
    pub notes: Vec<String>,
}

/// Environment for resolution: repo root plus the project profile.
#[derive(Debug)]
pub struct Env<'a> {
    root: PathBuf,
    profile: &'a Profile,
    /// Check list, read once. An error here makes every check cite stale.
    checks: Result<Vec<String>, String>,
}

impl<'a> Env<'a> {
    pub fn new(root: &Path, profile: &'a Profile) -> Self {
        let checks = profile
            .checks_at(root)
            .map_err(|e| format!("{e} (STALE)"));
        Self {
            root: root.to_path_buf(),
            profile,
            checks,
        }
    }

    /// Resolve one task's evidence into a verdict.
    pub fn resolve(&self, task: &Task) -> TaskVerdict {
        let note = (!task.note.trim().is_empty()).then(|| task.note.clone());
        let cites = self.render_cites(task);
        let pending = || TaskVerdict {
            id: task.id.clone(),
            state: TaskState::Pending,
            cites: cites.clone(),
            note: note.clone(),
            notes: Vec::new(),
        };
        let Some(evidence) = task.evidence.as_ref() else {
            return pending();
        };
        if !evidence.claims_done() {
            return pending();
        }
        let mut notes = Vec::new();
        if let Some(cite) = &evidence.commit
            && let Err(note) = self.resolve_commit(cite)
        {
            notes.push(note);
        }
        for cite in &evidence.commits {
            if let Err(note) = self.resolve_commit(cite) {
                notes.push(note);
                break; // the first unresolvable hash is named
            }
        }
        if let Some(needle) = &evidence.git_message
            && let Err(note) = self.resolve_git_message(needle)
        {
            notes.push(note);
        }
        if let Some(cite) = &evidence.check
            && let Err(note) = self.resolve_check(cite)
        {
            notes.push(note);
        }
        if let Some(name) = &evidence.log
            && let Err(note) = self.resolve_log(&task.id, name)
        {
            notes.push(note);
        }
        if let (Some(file), Some(needle)) = (&evidence.file, &evidence.contains)
            && let Err(note) = self.resolve_file_contains(file, needle)
        {
            notes.push(note);
        }
        if evidence.sentinel == Some(true)
            && let Err(note) = self.resolve_sentinel(&task.id)
        {
            notes.push(note);
        }
        if notes.is_empty() {
            TaskVerdict {
                id: task.id.clone(),
                state: TaskState::Done,
                cites,
                note,
                notes,
            }
        } else {
            TaskVerdict {
                id: task.id.clone(),
                state: TaskState::Stale,
                cites,
                note,
                notes,
            }
        }
    }

    /// Render every cite the row declares, for the ledger table.
    fn render_cites(&self, task: &Task) -> Vec<String> {
        let Some(evidence) = task.evidence.as_ref() else {
            return Vec::new();
        };
        let mut cites = Vec::new();
        if let Some(cite) = &evidence.commit {
            cites.push(format!("commit {cite}"));
        }
        if !evidence.commits.is_empty() {
            cites.push(format!("commits {}", evidence.commits.join(" + ")));
        }
        if let Some(needle) = &evidence.git_message {
            cites.push(format!("git-message: {needle}"));
        }
        if let Some(cite) = &evidence.check {
            cites.push(format!("check {cite}"));
        }
        if let Some(name) = &evidence.log {
            cites.push(format!("log {}", name.replace("{id}", &task.id)));
        }
        if let (Some(file), Some(needle)) = (&evidence.file, &evidence.contains) {
            cites.push(format!("{file} contains \"{needle}\""));
        }
        if evidence.sentinel == Some(true) {
            cites.push(format!(
                "sentinel {}",
                self.profile.sentinel_glob.replace("{id}", &task.id)
            ));
        }
        cites
    }

    fn resolve_commit(&self, cite: &str) -> Result<(), String> {
        if cite.starts_with('-') {
            return Err(format!(
                "commit cite '{cite}' is not a valid rev (STALE)"
            ));
        }
        // Taxonomy first: a missing repo and a missing rev are different
        // faults; conflated 128s hide which side is broken.
        match git_status(&self.root, &["rev-parse", "--git-dir"]) {
            Ok(0) => {}
            Ok(_) => {
                return Err(format!(
                    "'{}' is not a git repository (STALE)",
                    self.root.display()
                ));
            }
            Err(e) => return Err(format!("cannot run git: {e} (STALE)")),
        }
        match git_status(
            &self.root,
            &["rev-parse", "--verify", "--quiet", &format!("{cite}^{{commit}}")],
        ) {
            Ok(0) => {}
            Ok(1) => {
                return Err(format!(
                    "commit cite '{cite}' cannot be resolved in '{}': no such object (STALE)",
                    self.root.display()
                ));
            }
            Ok(code) => {
                return Err(format!("git rev-parse exited with code {code} (STALE)"));
            }
            Err(e) => return Err(format!("cannot run git: {e} (STALE)")),
        }
        match git_status(
            &self.root,
            &["merge-base", "--is-ancestor", cite, &self.profile.trunk_ref],
        ) {
            Ok(0) => Ok(()),
            Ok(1) => Err(format!(
                "commit cite '{cite}' is not an ancestor of '{}' (STALE)",
                self.profile.trunk_ref
            )),
            Ok(128) => Err(format!(
                "trunk ref '{}' cannot be resolved in '{}': no such ref (STALE)",
                self.profile.trunk_ref,
                self.root.display()
            )),
            Ok(code) => Err(format!(
                "git merge-base exited with code {code} (STALE)"
            )),
            Err(e) => Err(format!("cannot run git: {e} (STALE)")),
        }
    }

    /// Resolve a git-message cite: the cite must EQUAL the full subject
    /// line of exactly one commit on the trunk ref's first-parent history
    /// (`git log --format=%s`, anchored equality, fixed string — body
    /// text never counts). Zero subject matches and many subject matches
    /// are both STALE; many name every matching hash. Subject anchoring
    /// is what makes the cite deterministic: incidental body mentions
    /// can neither keep a rewritten landing green nor flip a healthy
    /// cite red. Documented visibility limit: commits OFF the first-parent
    /// chain (e.g. the second parent of a merge) are invisible here, so
    /// an off-trunk duplicate subject stays GREEN by design.
    fn resolve_git_message(&self, needle: &str) -> Result<(), String> {
        match git_stdout(
            &self.root,
            &[
                "log",
                "--first-parent",
                "--format=%h%x1f%s",
                // Revision first, then a bare `--`: with a tracked path
                // named like the ref (this repo has a `main/` directory),
                // a bare rev argument is ambiguous and git log exits 128.
                &self.profile.trunk_ref,
                "--",
            ],
        ) {
            Ok(lines) => {
                let matches: Vec<&str> = lines
                    .iter()
                    .filter(|line| {
                        line.split_once('\u{1f}')
                            .is_some_and(|(_, subject)| subject == needle)
                    })
                    .map(|line| line.split_once('\u{1f}').unwrap_or(("", "")).0)
                    .collect();
                if matches.len() == 1 {
                    Ok(())
                } else if matches.is_empty() {
                    Err(format!(
                        "git-message cite '{needle}' matches no commit subject on '{}' (STALE)",
                        self.profile.trunk_ref
                    ))
                } else {
                    Err(format!(
                        "git-message cite '{needle}' is the subject of {} commits ({}) on '{}': the subject must be unique (STALE)",
                        matches.len(),
                        matches.join(", "),
                        self.profile.trunk_ref
                    ))
                }
            }
            Err(128) => Err(format!(
                "trunk ref '{}' cannot be resolved in '{}': no such ref (STALE)",
                self.profile.trunk_ref,
                self.root.display()
            )),
            Err(code) => Err(format!("git log exited with code {code} (STALE)")),
        }
    }

    /// Resolve one structured log cite: exact path from the project root
    /// whose contents carry the exact literal. When the profile configures
    /// `log-roots`, the path must also sit under one of them (or under
    /// `log-dir`, always allowed); the roots are the allowlist that keeps
    /// path cites inside the declared log-cite surface.
    fn resolve_file_contains(&self, file: &str, needle: &str) -> Result<(), String> {
        if gates::cite_escapes_root(file) {
            return Err(format!(
                "file cite '{file}' escapes the project root (STALE)"
            ));
        }
        let path = self.root.join(file);
        if !self.profile.log_roots.is_empty() {
            let under = std::iter::once(&self.profile.log_dir)
                .chain(self.profile.log_roots.iter())
                .any(|root| path.starts_with(self.root.join(root)));
            if !under {
                return Err(format!(
                    "file cite '{file}' resolves outside the configured log-roots (STALE)"
                ));
            }
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "file cite '{file}' has no file at {} (STALE)",
                    path.display()
                ));
            }
            Err(e) => {
                return Err(format!(
                    "file cite '{file}' at {} is unreadable: {e} (STALE)",
                    path.display()
                ));
            }
        };
        if text.contains(needle) {
            Ok(())
        } else {
            Err(format!(
                "file cite '{file}' at {} does not contain the cited text (STALE)",
                path.display()
            ))
        }
    }

    fn resolve_check(&self, cite: &str) -> Result<(), String> {        match &self.checks {
            Err(note) => Err(format!(
                "check cite '{cite}' cannot resolve: {note}"
            )),
            Ok(checks) => {
                if checks.iter().any(|name| name == cite) {
                    Ok(())
                } else {
                    Err(format!(
                        "check cite '{cite}' is not in check-spec list (STALE)"
                    ))
                }
            }
        }
    }

    fn resolve_log(&self, task_id: &str, name: &str) -> Result<(), String> {
        if gates::cite_escapes_root(name) {
            return Err(format!(
                "log cite '{name}' escapes the project root (STALE)"
            ));
        }
        let path = self
            .root
            .join(&self.profile.log_dir)
            .join(name.replace("{id}", task_id));
        // A missing file and an unreadable one are different faults;
        // both stay STALE, but the message must say which.
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "log cite '{}' has no file at {} (STALE)",
                    name,
                    path.display()
                ));
            }
            Err(e) => {
                return Err(format!(
                    "log cite '{}' at {} is unreadable: {e} (STALE)",
                    name,
                    path.display()
                ));
            }
        };
        if text.contains(task_id) {
            Ok(())
        } else {
            Err(format!(
                "log cite '{}' at {} does not contain task id '{task_id}' (STALE)",
                name,
                path.display()
            ))
        }
    }

    fn resolve_sentinel(&self, task_id: &str) -> Result<(), String> {
        // A sentinel claim with no configured glob must never resolve:
        // root.join("") is the root directory, which would match itself
        // into a DONE with zero evidence. Silent-DONE is illegal.
        if self.profile.sentinel_glob.trim().is_empty() {
            return Err(
                "sentinel claim but profile sets no sentinel-glob: nothing can match (STALE)"
                    .to_string(),
            );
        }
        let pattern = self
            .root
            .join(self.profile.sentinel_glob.replace("{id}", task_id));
        let pattern = pattern.to_string_lossy().into_owned();
        let count = glob::glob(&pattern)
            .map_err(|e| format!("sentinel-glob '{pattern}' is not a valid glob: {e} (STALE)"))?
            .filter_map(Result::ok)
            // Directories must not count as sentinel evidence.
            .filter(|path| path.is_file())
            .count();
        if count >= 1 {
            Ok(())
        } else {
            Err(format!(
                "sentinel glob '{pattern}' matched no files (STALE)"
            ))
        }
    }
}

/// Refuse a resolution root that is not a git repository, so commit
/// evidence can never resolve against a repo the spec never declared.
pub fn require_repo(root: &Path) -> Result<(), String> {
    match git_status(root, &["rev-parse", "--git-dir"]) {
        Ok(0) => Ok(()),
        Ok(_) => Err(format!(
            "resolution root '{}' is not a git repository (name the project repo with --repo)",
            root.display()
        )),
        Err(e) => Err(format!("cannot run git to check '{}': {e}", root.display())),
    }
}

/// GIT_* variables the repo runner law unsets in full; stripped first,
/// then anything else GIT_* found in the parent environment.
const RUNNER_GIT_VARS: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

/// Remove every GIT_* environment variable from `command` so the
/// subprocess can never be pointed at an enclosing repo (runner law).
fn strip_git_env(command: &mut Command) {
    for key in RUNNER_GIT_VARS {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
}

/// Run git in `root` (GIT_* stripped, same law) and collect stdout lines.
/// The exit code is returned in the error channel so callers can
/// distinguish failure kinds (128 = unknown ref) without parsing text.
fn git_stdout(root: &Path, args: &[&str]) -> Result<Vec<String>, i32> {
    let mut command = Command::new("git");
    strip_git_env(&mut command);
    let out = command
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("git runs");
    if !out.status.success() {
        return Err(out.status.code().unwrap_or(-1));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .collect())
}

/// Run git in `root` with every GIT_* environment variable unset, so the
/// subprocess can never be pointed at an enclosing repo (runner law).
fn git_status(root: &Path, args: &[&str]) -> Result<i32, std::io::Error> {
    let mut command = Command::new("git");
    strip_git_env(&mut command);
    let status = command
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

/// Per-task verdicts for `planspec status`.
pub fn verdicts(root: &Path, profile: &Profile, plan: &Workplan) -> Vec<TaskVerdict> {
    let env = Env::new(root, profile);
    let mut verdicts: Vec<TaskVerdict> = plan.task.iter().map(|task| env.resolve(task)).collect();
    // Tombstones close the vocabulary: an id with recorded children is
    // Superseded by its split; an id with none recorded yet is Parked.
    verdicts.extend(plan.tombstone.iter().map(|tombstone| TaskVerdict {
        id: tombstone.id.clone(),
        state: if tombstone.split_into.is_empty() {
            TaskState::Parked
        } else {
            TaskState::Superseded
        },
        cites: if tombstone.split_into.is_empty() {
            Vec::new()
        } else {
            vec![format!("split-into: {}", tombstone.split_into.join(", "))]
        },
        note: None,
        notes: Vec::new(),
    }));
    verdicts
}

/// Strict phase gate: no task may show Done in a later declared phase
/// while an earlier declared phase still has unfinished (live, non-Done)
/// tasks. Declared `number` order is the only order the tool honours.
pub fn phase_findings(plan: &Workplan, verdicts: &[TaskVerdict]) -> Vec<Finding> {
    if plan.phase.is_empty() {
        return Vec::new();
    }
    let state_of = |id: &str| -> TaskState {
        verdicts
            .iter()
            .find(|v| v.id == id)
            .map(|v| v.state)
            .unwrap_or(TaskState::Pending)
    };
    let mut findings = Vec::new();
    let highest_done = plan
        .task
        .iter()
        .filter(|task| state_of(&task.id) == TaskState::Done)
        .filter_map(|task| task.phase)
        .max();
    let Some(limit) = highest_done else {
        return findings;
    };
    for phase in &plan.phase {
        if phase.number >= limit {
            continue;
        }
        let unfinished: Vec<&str> = plan
            .task
            .iter()
            .filter(|task| task.phase == Some(phase.number))
            .filter(|task| state_of(&task.id) != TaskState::Done)
            .map(|task| task.id.as_str())
            .collect();
        if !unfinished.is_empty() {
            findings.push(Finding::new(
                "phase",
                format!(
                    "phase P{} ({}) has unfinished tasks ({}) while a later phase has done work",
                    phase.number,
                    phase.title,
                    unfinished.join(", ")
                ),
            ));
        }
    }
    findings
}

/// Strict phase completeness: when a plan declares `[[phase]]` blocks,
/// every task must carry a `phase` number — an unassigned task escapes
/// the ordering gate silently. Removing all `[[phase]]` declarations is
/// the legal opt-out, but then the ordering gate itself is gone.
pub fn phase_completeness_findings(plan: &Workplan) -> Vec<Finding> {
    if plan.phase.is_empty() {
        return Vec::new();
    }
    plan.task
        .iter()
        .filter(|task| task.phase.is_none())
        .map(|task| {
            Finding::new(
                "phase",
                format!(
                    "task '{}' has no phase while the plan orders work (assign phase = <number>)",
                    task.id
                ),
            )
        })
        .collect()
}

/// Spec paths tracked in the git index (`git ls-files`), sorted.
/// Uncommitted scratch files are never in the index, so a bare run can
/// never be polluted by them: the committed set is the gate.
pub fn ls_specs(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = git_stdout(root, &["ls-files", "--", "*.planspec.toml"])
        .unwrap_or_default()
        .into_iter()
        .map(|rel| root.join(rel))
        .collect();
    found.sort();
    found
}

/// Resolve the repository root from `cwd` via `git rev-parse
/// --show-toplevel`. Used so a bare run from any subdirectory resolves
/// cites against the repo root, not a partial relative path.
pub fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    git_stdout(cwd, &["rev-parse", "--show-toplevel"])
        .ok()
        .and_then(|mut lines| lines.pop().map(PathBuf::from))
}

/// Freshness findings for the `verify` gate: one per stale task,
/// naming every unresolvable cite.
pub fn freshness_findings(
    root: &Path,
    profile: &Profile,
    plan: &Workplan,
) -> Vec<Finding> {
    verdicts(root, profile, plan)
        .into_iter()
        .filter(|verdict| verdict.state == TaskState::Stale)
        .map(|verdict| {
            Finding::new(
                "freshness",
                format!("task '{}': {}", verdict.id, verdict.notes.join("; ")),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Workplan;

    /// Spawn git in `dir` with every GIT_* variable unset (test law:
    /// fixtures must never touch the enclosing repo).
    fn git_in(dir: &Path, args: &[&str]) -> std::process::Output {
        let mut command = Command::new("git");
        strip_git_env(&mut command);
        let out = command
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn rev(dir: &Path, r: &str) -> String {
        String::from_utf8_lossy(&git_in(dir, &["rev-parse", r]).stdout)
            .trim()
            .to_string()
    }

    /// Repo with: `main` = seed + second, `side` = seed + side-branch.
    fn fixture_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tmpdir");
        let root = dir.path();
        git_in(root, &["init", "-b", "main", "."]);
        git_in(root, &["config", "user.email", "t@example.invalid"]);
        git_in(root, &["config", "user.name", "planspec tests"]);
        std::fs::write(root.join("seed"), "seed\n").unwrap();
        git_in(root, &["add", "seed"]);
        git_in(root, &["commit", "-m", "seed"]);
        git_in(root, &["branch", "side"]);
        std::fs::write(root.join("second"), "second\n").unwrap();
        git_in(root, &["add", "second"]);
        git_in(root, &["commit", "-m", "second"]);
        git_in(root, &["checkout", "-q", "side"]);
        std::fs::write(root.join("side"), "side\n").unwrap();
        git_in(root, &["add", "side"]);
        git_in(root, &["commit", "-m", "side"]);
        git_in(root, &["checkout", "-q", "main"]);
        dir
    }

    fn profile() -> Profile {
        Profile {
            trunk_ref: "main".to_string(),
            log_roots: Vec::new(),
            check_spec: PathBuf::from("checks.toml"),
            log_dir: PathBuf::from("logs"),
            sentinel_glob: "sentinel/{id}.done".to_string(),
            id_grammar: r"^[a-z][a-z0-9.-]*$".to_string(),
        }
    }

    fn task_with(evidence_toml: &str) -> Task {
        let text = format!(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n[[task]]\nid = \"task-a\"\n\
             behavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n{evidence_toml}"
        );
        let plan = Workplan::from_toml(&text).expect("fixture spec");
        plan.task[0].clone()
    }

    #[test]
    fn commit_ancestor_resolves() {
        let dir = fixture_repo();
        let seed = rev(dir.path(), "main~1");
        let task = task_with(&format!("[task.evidence]\ncommit = \"{seed}\"\n"));
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Done);
        assert!(verdict.notes.is_empty());
    }

    #[test]
    fn missing_commit_is_stale() {
        let dir = fixture_repo();
        let task = task_with("[task.evidence]\ncommit = \"deadbeef\"\n");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("no such object (STALE)"));
    }

    #[test]
    fn non_ancestor_commit_is_stale() {
        let dir = fixture_repo();
        let side = rev(dir.path(), "side");
        let task = task_with(&format!("[task.evidence]\ncommit = \"{side}\"\n"));
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("not an ancestor of 'main' (STALE)"));
    }

    #[test]
    fn check_cite_resolves_or_is_stale() {
        let dir = fixture_repo();
        std::fs::write(dir.path().join("checks.toml"), "checks = [\"clippy\"]\n").unwrap();
        let prof = profile();
        let env = Env::new(dir.path(), &prof);
        let ok = task_with("[task.evidence]\ncheck = \"clippy\"\n");
        assert_eq!(env.resolve(&ok).state, TaskState::Done);
        let bad = task_with("[task.evidence]\ncheck = \"unknown-check\"\n");
        let verdict = env.resolve(&bad);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("not in check-spec list (STALE)"));
    }

    #[test]
    fn check_cite_without_check_spec_file_is_stale() {
        let dir = fixture_repo();
        let task = task_with("[task.evidence]\ncheck = \"clippy\"\n");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("cannot resolve"));
    }

    #[test]
    fn log_cite_needs_file_containing_task_id() {
        let dir = fixture_repo();
        std::fs::create_dir(dir.path().join("logs")).unwrap();
        let task = task_with("[task.evidence]\nlog = \"{id}.md\"\n");
        // Missing file first.
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("no file"));
        // File without the task id stays stale.
        std::fs::write(dir.path().join("logs/task-a.md"), "nothing here\n").unwrap();
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("does not contain task id"));
        // File named for the task and carrying its id resolves.
        std::fs::write(dir.path().join("logs/task-a.md"), "task-a logged\n").unwrap();
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Done);
    }

    #[test]
    fn sentinel_glob_must_match() {
        let dir = fixture_repo();
        let task = task_with("[task.evidence]\nsentinel = true\n");
        let prof = profile();
        let env = Env::new(dir.path(), &prof);
        assert_eq!(env.resolve(&task).state, TaskState::Stale);
        std::fs::create_dir(dir.path().join("sentinel")).unwrap();
        std::fs::write(dir.path().join("sentinel/task-a.done"), "").unwrap();
        assert_eq!(env.resolve(&task).state, TaskState::Done);
    }

    #[test]
    fn no_evidence_is_pending_not_stale() {
        let dir = fixture_repo();
        let task = task_with("");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Pending);
        assert!(verdict.notes.is_empty());
    }

    #[test]
    fn git_subprocess_isolated_from_enclosing_repo() {
        // A poisoned GIT_DIR (as inside a hook) must not reach the
        // subprocess: resolution still uses the fixture repo.
        let dir = fixture_repo();
        let seed = rev(dir.path(), "main~1");
        let mut command = Command::new("git");
        command.env("GIT_DIR", "/nonexistent-poisoned-dir");
        strip_git_env(&mut command);
        let out = command
            .current_dir(dir.path())
            .args(["rev-parse", "--git-dir"])
            .output()
            .expect("git runs");
        assert!(out.status.success());
        let task = task_with(&format!("[task.evidence]\ncommit = \"{seed}\"\n"));
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Done);
    }

    #[test]
    fn findings_name_stale_tasks() {
        let dir = fixture_repo();
        let plan: Workplan = toml::from_str(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"b\"\ndepends-on = []\n\
             done-when = \"d\"\n[task.evidence]\ncheck = \"ghost\"\n",
        )
        .unwrap();
        let findings = freshness_findings(dir.path(), &profile(), &plan);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "freshness");
        assert!(findings[0].message.starts_with("task 'task-a':"));
        assert!(findings[0].message.contains("(STALE)"));
    }

    // B1: evidence must resolve against the declared repo, never another.
    #[test]
    fn commit_from_foreign_repo_is_stale() {
        let mine = fixture_repo();
        let theirs = fixture_repo();
        // A commit that exists only in the foreign repo.
        std::fs::write(theirs.path().join("foreign-only"), "only there\n").unwrap();
        git_in(theirs.path(), &["add", "foreign-only"]);
        git_in(theirs.path(), &["commit", "-m", "foreign-only"]);
        let foreign = rev(theirs.path(), "main");
        let task = task_with(&format!("[task.evidence]\ncommit = \"{foreign}\"\n"));
        // Resolution root is `mine`: the foreign commit is unknown there.
        let verdict = Env::new(mine.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("no such object (STALE)"));
    }

    #[test]
    fn require_repo_refuses_non_repo_root() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let err = require_repo(dir.path()).expect_err("plain dir is not a repo");
        assert!(err.contains("not a git repository"), "got: {err}");
        let repo = fixture_repo();
        assert!(require_repo(repo.path()).is_ok());
    }

    // B2: a claim with no configured glob must be red, never silent-DONE.
    #[test]
    fn sentinel_claim_without_glob_is_stale() {
        let dir = fixture_repo();
        let task = task_with("[task.evidence]\nsentinel = true\n");
        let prof = Profile {
            sentinel_glob: String::new(),
            ..profile()
        };
        let verdict = Env::new(dir.path(), &prof).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("no sentinel-glob"), "got: {:?}", verdict.notes);
    }

    // B2: glob hits on directories are not sentinel evidence.
    #[test]
    fn sentinel_glob_matching_a_directory_is_stale() {
        let dir = fixture_repo();
        let task = task_with("[task.evidence]\nsentinel = true\n");
        let prof = profile();
        let env = Env::new(dir.path(), &prof);
        std::fs::create_dir_all(dir.path().join("sentinel/task-a.done")).unwrap();
        let verdict = env.resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(verdict.notes[0].contains("matched no files"));
        std::fs::remove_dir(dir.path().join("sentinel/task-a.done")).unwrap();
        std::fs::write(dir.path().join("sentinel/task-a.done"), "").unwrap();
        assert_eq!(env.resolve(&task).state, TaskState::Done);
    }

    // m1: 'not a repo' and 'bad ref' carry distinct messages.
    #[test]
    fn commit_taxonomy_distinguishes_no_repo_from_bad_ref() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let task = task_with("[task.evidence]\ncommit = \"deadbeef\"\n");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(
            verdict.notes[0].contains("is not a git repository"),
            "got: {:?}",
            verdict.notes
        );

        let repo = fixture_repo();
        let seed = rev(repo.path(), "main~1");
        let task = task_with(&format!("[task.evidence]\ncommit = \"{seed}\"\n"));
        let prof = Profile {
            trunk_ref: "no-such-ref".to_string(),
            ..profile()
        };
        let verdict = Env::new(repo.path(), &prof).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(
            verdict.notes[0].contains("no such ref"),
            "got: {:?}",
            verdict.notes
        );
        assert!(
            !verdict.notes[0].contains("no such object"),
            "taxonomy must not conflate: {:?}",
            verdict.notes
        );
    }

    // m2: an unreadable log file is reported as unreadable, not missing.
    #[test]
    fn unreadable_log_is_not_reported_as_missing() {
        let dir = fixture_repo();
        std::fs::create_dir_all(dir.path().join("logs/task-a.md")).unwrap();
        let task = task_with("[task.evidence]\nlog = \"{id}.md\"\n");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(
            verdict.notes[0].contains("unreadable"),
            "got: {:?}",
            verdict.notes
        );
        assert!(
            !verdict.notes[0].contains("no file"),
            "got: {:?}",
            verdict.notes
        );
    }

    // m5: a log cite reaching outside the root is refused, not read.
    #[test]
    fn log_cite_escaping_root_is_stale_without_reading() {
        let dir = fixture_repo();
        std::fs::write(dir.path().join("escape.md"), "task-a\n").unwrap();
        let task = task_with("[task.evidence]\nlog = \"../escape.md\"\n");
        let verdict = Env::new(dir.path(), &profile()).resolve(&task);
        assert_eq!(verdict.state, TaskState::Stale);
        assert!(
            verdict.notes[0].contains("escapes the project root"),
            "got: {:?}",
            verdict.notes
        );
    }
}
