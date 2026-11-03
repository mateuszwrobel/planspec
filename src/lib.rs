//! planspec — workplane verifier.
//!
//! planspec reads work-decomposition specs (workplans, subplans/tasks,
//! decomposition tombstones) plus a project profile, resolves the evidence
//! each task cites, and reports verdicts. It is read-only: it starts no
//! processes, orchestrates nothing, and never lets an AI judge completion.
//! Task status is DERIVED from evidence, never declared.

pub mod config;
pub mod gates;
pub mod help;
pub mod model;
pub mod verdict;
