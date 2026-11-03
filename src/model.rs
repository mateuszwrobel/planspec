//! Workplan spec model: one TOML file per plan.
//!
//! Shape (as TOML):
//!
//! ```toml
//! [plan]
//! id = "plan-1"
//! title = "..."
//! source-cites = ["..."]
//!
//! [[phase]]           # optional ordering declarations; numbers order phases
//! number = 0
//! title = "foundation gate"
//!
//! [[task]]
//! id = "task-1"
//! behavior = "..."
//! depends-on = []
//! done-when = "..."
//! phase = 0           # optional; must name a declared [[phase]] number
//! note = "..."        # optional; declares an annotation (Pending-with-note)
//! [task.evidence]     # every key optional; absence = missing
//! commit = "abc123"   # one commit cite
//! commits = ["a1b2", "c3d4"]  # multi-commit cite: ALL must be ancestors
//! git-message = "landed by message"  # exactly-one match on the trunk history
//! check = "clippy"
//! log = "task-1.md"   # legacy free-text form (warns under --strict)
//! file = "logs/task-1.md"       # structured log cite: exact path ...
//! contains = "task-1"           # ... plus exact substring
//! sentinel = true
//!
//! [[tombstone]]         # decomposition: ids stay stable
//! id = "task-0"
//! split-into = ["task-2", "task-3"]
//! ```
//!
//! v0.1 specs parse unchanged: every v0.2 addition is optional.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One workplan spec file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Workplan {
    pub plan: Plan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phase: Vec<Phase>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task: Vec<Task>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstone: Vec<Tombstone>,
}

/// The plan header block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub source_cites: Vec<String>,
}

/// One declared phase. `number` is what tasks cite and what orders the
/// phases (integer order, nothing else); `title` is prose. Declaring a
/// phase is the only ordering the tool honours: no implicit ordering is
/// invented beyond the declared numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Phase {
    pub number: i64,
    #[serde(default)]
    pub title: String,
}

/// One unit of work in the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub behavior: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub done_when: String,
    /// Declared phase number; must name a `[[phase]]` number. Absent means
    /// the task joins no phase ordering.
    #[serde(default)]
    pub phase: Option<i64>,
    /// Author annotation folded into the verdict: a task citing no
    /// completion evidence stays Pending, and this note is what
    /// `--strict` demands for an annotated Pending (a declared-skip or
    /// a done-with-clarification row). It never lowers a resolving Done.
    #[serde(default)]
    pub note: String,
    /// Set by `scaffold --from-table` when `note` records an automatic
    /// dedup fold rather than a human annotation. Under `--strict` a
    /// generated note is NOT clearance: a human must edit or replace it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub note_generated: bool,
    #[serde(default)]
    pub evidence: Option<Evidence>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Evidence a task cites to claim completion. Every key is optional;
/// absence means missing. `sentinel = true` resolves through the project
/// profile's `sentinel_glob`; a value of `false` is not a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Evidence {
    #[serde(default)]
    pub commit: Option<String>,
    /// Multi-commit cite: every hash must be an ancestor of the trunk ref;
    /// the first unresolvable one is named. An empty list is not a claim.
    #[serde(default)]
    pub commits: Vec<String>,
    /// Landed-by-message cite: resolved via `git log --grep` over the
    /// trunk ref's first-parent history. Exactly one match resolves;
    /// zero or many are stale, and many name the match list.
    #[serde(default)]
    pub git_message: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    /// Legacy free-text log cite: a file name (may contain `{id}`) under
    /// the profile's `log-dir` whose contents contain the task id.
    /// Still resolves; `--strict` warns it is legacy.
    #[serde(default)]
    pub log: Option<String>,
    /// Structured log cite, one pair: `file` is the exact path from the
    /// project root (no `{id}` expansion), `contains` the exact literal
    /// the file must carry. One without the other is a cite failure.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub contains: Option<String>,
    #[serde(default)]
    pub sentinel: Option<bool>,
}

impl Evidence {
    /// True when this block claims completion through at least one kind.
    pub fn claims_done(&self) -> bool {
        self.commit.is_some()
            || !self.commits.is_empty()
            || self.git_message.is_some()
            || self.check.is_some()
            || self.log.is_some()
            || self.file.is_some()
            || self.contains.is_some()
            || self.sentinel == Some(true)
    }
}

/// A decomposed task: the id stays stable and records its children.
/// `split-into` names current tasks only; a tombstone-to-tombstone
/// chain is not representable — a split that was itself decomposed
/// must record its final child tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Tombstone {
    pub id: String,
    #[serde(default)]
    pub split_into: Vec<String>,
}

impl Workplan {
    /// Parse a workplan spec file. Any TOML or shape problem is an
    /// `Invalid` error (CLI exit 70), never a gate failure.
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let text = std::fs::read_to_string(path).map_err(|e| SpecError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Self::from_toml(&text)
    }

    /// Parse workplan spec text.
    pub fn from_toml(text: &str) -> Result<Self, SpecError> {
        toml::from_str(text).map_err(|e| SpecError::Parse(e.to_string()))
    }

    /// Schema-valid skeleton for `planspec scaffold`.
    pub fn skeleton(id: &str, title: &str) -> String {
        let plan = Workplan {
            plan: Plan {
                id: id.to_string(),
                title: title.to_string(),
                source_cites: Vec::new(),
            },
            phase: Vec::new(),
            task: Vec::new(),
            tombstone: Vec::new(),
        };
        let body =
            toml::to_string_pretty(&plan).expect("skeleton always serializes");
        format!(
            "# planspec workplan scaffold — add [[task]] blocks with\n\
             # [task.evidence] cites (commit / commits / git-message / check /\n\
             # file+contains / sentinel), optional phase = <number> backed by\n\
             # [[phase]] declarations, and optional note = \"...\" annotations.\n{body}"
        )
    }

    /// Row-conversion scaffold (`planspec scaffold --from-table`): each
    /// markdown pipe-table data row becomes one `[[task]]` block.
    /// - id: the first cell with `**` emphasis stripped; an empty id cell
    ///   mints `row-<n>`, where n is the row's 1-based ordinal among data
    ///   rows of the input.
    /// - behavior: the second cell (or the remaining cells joined when the
    ///   row carries only the id); done-when: the last cell.
    /// - identical rows (emphasis and whitespace stripped from every cell)
    ///   deduplicate to the first occurrence, and that task's note records
    ///   how many copies the table carried.
    /// - no evidence is invented: every row lands Pending for a human to
    ///   cite.
    pub fn from_markdown_table(id: &str, title: &str, text: &str) -> String {
        let mut tasks: Vec<Task> = Vec::new();
        // (normalized row, task index, copies seen so far)
        let mut seen: Vec<(String, usize, usize)> = Vec::new();
        let mut ordinal = 0usize;
        let lines: Vec<&str> = text.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                continue;
            }
            let cells: Vec<String> = trimmed
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().replace("**", ""))
                .collect();
            // Separator rows (|---|:--:|) carry no data.
            if cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) {
                continue;
            }
            // A row directly above a separator line is that table's header.
            let followed_by_separator = lines.get(i + 1).is_some_and(|next| {
                let next = next.trim();
                next.starts_with('|')
                    && next
                        .trim_matches('|')
                        .split('|')
                        .all(|c| !c.trim().is_empty() && c.trim().chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
            });
            if followed_by_separator {
                continue;
            }

            let normalized = cells.join("|");
            if let Some(hit) = seen.iter().position(|(row, _, _)| *row == normalized) {
                seen[hit].2 += 1;
                let (index, copies) = (seen[hit].1, seen[hit].2);
                tasks[index].note = format!(
                    "deduplicated: identical table row appeared {copies} times"
                );
                tasks[index].note_generated = true;
                continue;
            }
            seen.push((normalized, tasks.len(), 1));

            ordinal += 1;
            let task_id = cells
                .first()
                .filter(|c| !c.trim().is_empty())
                .map(|c| c.trim().to_string())
                .unwrap_or_else(|| format!("row-{ordinal}"));
            let behavior = cells
                .get(1)
                .cloned()
                .unwrap_or_else(|| cells[1..].join(" · "));
            let done_when = cells.last().cloned().unwrap_or_default();
            tasks.push(Task {
                id: task_id,
                behavior: if behavior.trim().is_empty() {
                    "declare the behavior".to_string()
                } else {
                    behavior
                },
                depends_on: Vec::new(),
                done_when: if done_when.trim().is_empty() {
                    "declare the completion proof".to_string()
                } else {
                    done_when
                },
                phase: None,
                note: String::new(),
                note_generated: false,
                evidence: None,
            });
        }

        let plan = Workplan {
            plan: Plan {
                id: id.to_string(),
                title: title.to_string(),
                source_cites: Vec::new(),
            },
            phase: Vec::new(),
            task: tasks,
            tombstone: Vec::new(),
        };
        let body =
            toml::to_string_pretty(&plan).expect("scaffold always serializes");
        format!(
            "# planspec scaffold from markdown table — ids minted as row-<n>\n\
             # where a row carried none; identical rows deduplicated with a note.\n\
             # No evidence is invented: cite each task before gating.\n{body}"
        )
    }
}

/// Load/parse failures for specs and profiles. Both map to CLI exit 70.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("spec invalid: {0}")]
    Parse(String),
    #[error("profile invalid: {0}")]
    Profile(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[plan]
id = "plan-1"
title = "Demo plan"
source-cites = ["spec:RFC-1"]

[[phase]]
number = 0
title = "foundation gate"

[[task]]
id = "task-1"
behavior = "reads the spec"
depends-on = []
done-when = "verify exits 0"
phase = 0
[task.evidence]
commit = "abc1234"
check = "clippy"
log = "task-1.md"
sentinel = true

[[task]]
id = "task-2"
behavior = "depends on task-1"
depends-on = ["task-1"]
done-when = "status shows done"
note = "declared-skip: zero new doors"

[[tombstone]]
id = "task-0"
split-into = ["task-1", "task-2"]
"#;

    #[test]
    fn parses_full_spec() {
        let plan = Workplan::from_toml(FULL).expect("valid spec");
        assert_eq!(plan.plan.id, "plan-1");
        assert_eq!(plan.phase.len(), 1);
        assert_eq!(plan.phase[0].number, 0);
        assert_eq!(plan.task.len(), 2);
        assert_eq!(plan.task[0].depends_on, Vec::<String>::new());
        assert_eq!(plan.task[1].depends_on, ["task-1"]);
        assert_eq!(plan.task[0].phase, Some(0));
        assert_eq!(plan.task[1].note, "declared-skip: zero new doors");
        assert_eq!(
            plan.task[0].evidence.as_ref().map(|e| e.commit.as_deref()),
            Some(Some("abc1234"))
        );
        assert_eq!(plan.tombstone[0].split_into, ["task-1", "task-2"]);
        assert!(plan.task[0].evidence.as_ref().unwrap().claims_done());
        assert!(!plan.task[1].evidence.as_ref().is_some_and(|e| e.claims_done()));
    }

    #[test]
    fn parses_v02_cite_shapes() {
        let plan = Workplan::from_toml(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n[[task]]\nid = \"t1\"\n\
             behavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommits = [\"a1\", \"b2\"]\n\
             git-message = \"law lands\"\nfile = \"logs/t1.md\"\ncontains = \"t1\"\n",
        )
        .expect("v0.2 spec parses");
        let evidence = plan.task[0].evidence.as_ref().unwrap();
        assert_eq!(evidence.commits, ["a1", "b2"]);
        assert_eq!(evidence.git_message.as_deref(), Some("law lands"));
        assert_eq!(evidence.file.as_deref(), Some("logs/t1.md"));
        assert_eq!(evidence.contains.as_deref(), Some("t1"));
        assert!(evidence.claims_done());
    }

    // Backward compatibility: a v0.1 spec parses unchanged, with no
    // phase/note/new-cite fields.
    #[test]
    fn v01_spec_parses_unchanged() {
        let plan = Workplan::from_toml(
            "[plan]\nid = \"p\"\ntitle = \"t\"\nsource-cites = []\n[[task]]\n\
             id = \"t1\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"abc1234\"\ncheck = \"clippy\"\n\
             log = \"t1.md\"\nsentinel = true\n[[tombstone]]\nid = \"t0\"\n\
             split-into = [\"t1\"]\n",
        )
        .expect("v0.1 spec parses in v0.2");
        assert!(plan.phase.is_empty());
        assert_eq!(plan.task[0].phase, None);
        assert!(plan.task[0].note.is_empty());
        let evidence = plan.task[0].evidence.as_ref().unwrap();
        assert!(evidence.commits.is_empty());
        assert_eq!(evidence.git_message, None);
        assert_eq!(evidence.file, None);
    }

    #[test]
    fn absent_evidence_is_none() {
        let plan = Workplan::from_toml(FULL).expect("valid spec");
        assert!(plan.task[1].evidence.is_none());
    }

    #[test]
    fn sentinel_false_is_not_a_claim() {
        let evidence: Evidence = toml::from_str("sentinel = false").unwrap();
        assert!(!evidence.claims_done());
    }

    #[test]
    fn empty_commits_list_is_not_a_claim() {
        let evidence: Evidence = toml::from_str("commits = []").unwrap();
        assert!(!evidence.claims_done());
    }

    #[test]
    fn unknown_field_is_invalid() {
        let bad = "[plan]\nid = \"p\"\ntitle = \"t\"\nbogus = 1\n";
        assert!(matches!(
            Workplan::from_toml(bad),
            Err(SpecError::Parse(_))
        ));
    }

    #[test]
    fn missing_required_field_is_invalid() {
        let bad = "[plan]\nid = \"p\"\n";
        assert!(matches!(
            Workplan::from_toml(bad),
            Err(SpecError::Parse(_))
        ));
    }

    #[test]
    fn malformed_toml_is_invalid() {
        assert!(matches!(
            Workplan::from_toml("[plan\nid = 1"),
            Err(SpecError::Parse(_))
        ));
    }

    #[test]
    fn skeleton_is_schema_valid() {
        let text = Workplan::skeleton("plan-x", "X");
        let plan = Workplan::from_toml(&text).expect("skeleton parses");
        assert_eq!(plan.plan.id, "plan-x");
        assert_eq!(plan.plan.title, "X");
        assert!(plan.task.is_empty());
    }

    #[test]
    fn from_table_mints_ids_and_dedups() {
        let table = "| id | rows | ONE behavior |\n|---|---|---|\n\
                     | UI-1 | A1 | one behavior |\n\
                     | | gate | foundation lands |\n\
                     | **PC-29** | — | seat the PO |\n\
                     | UI-1 | A1 | one behavior |\n";
        let text = Workplan::from_markdown_table("plan-t", "T", table);
        let plan = Workplan::from_toml(&text).expect("scaffold parses");
        assert_eq!(plan.task.len(), 3, "the identical row must dedup: {plan:?}");
        assert_eq!(plan.task[0].id, "UI-1");
        assert_eq!(plan.task[1].id, "row-2", "minted id names the data-row ordinal");
        assert_eq!(plan.task[2].id, "PC-29", "emphasis is stripped from the id");
        assert_eq!(
            plan.task[0].note,
            "deduplicated: identical table row appeared 2 times"
        );
        assert!(plan.task[0].note_generated, "a dedup fold is a generated note, not a human one");
        assert!(!plan.task[1].note_generated, "a noteless row carries no generated flag");
    }
}
