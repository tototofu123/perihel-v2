//! Runs the shared behavioural contract in `shared/dictionary-test-vectors.json`.
//!
//! The macOS (Swift) and Windows (C#) apps implement the correction pass
//! independently, and independent implementations drift. That file is the thing
//! that stops them: this test is the Rust side of the same contract, so a change
//! to correction semantics has to be made there first and then reflected in every
//! implementation.

use std::fs;
use std::path::PathBuf;

use perihel_core::dictionary::{Correction, Dictionary, DictionaryCorrector};
use serde::Deserialize;

// ---------------------------------------------------------------- file schema

#[derive(Deserialize)]
struct Vectors {
    version: u32,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    entries: Vec<Entry>,
    input: String,
    expected: String,
    #[serde(rename = "expectedCorrections", default)]
    expected_corrections: Vec<ExpectedHit>,
}

#[derive(Deserialize)]
struct Entry {
    /// "correction" rewrites text; "term" is a bias word handed to the STT engine.
    kind: String,
    #[serde(default)]
    hear: Option<String>,
    write: String,
    #[serde(rename = "isEnabled", default = "enabled_by_default")]
    is_enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Deserialize)]
struct ExpectedHit {
    to: String,
    count: usize,
}

// ------------------------------------------------------------------- plumbing

fn vectors_path() -> PathBuf {
    // crates/perihel-core -> workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("shared")
        .join("dictionary-test-vectors.json")
}

fn load() -> Vectors {
    let path = vectors_path();
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read contract {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("cannot parse contract {}: {e}", path.display()))
}

fn build(entries: &[Entry]) -> Dictionary {
    let mut words = Vec::new();
    let mut corrections = Vec::new();
    for e in entries {
        match e.kind.as_str() {
            "correction" => corrections.push(Correction {
                hear: e
                    .hear
                    .clone()
                    .unwrap_or_else(|| panic!("correction entry {:?} has no `hear`", e.write)),
                write: e.write.clone(),
                is_enabled: e.is_enabled,
            }),
            "term" => words.push(e.write.clone()),
            other => panic!("unknown entry kind {other:?}"),
        }
    }
    Dictionary { words, corrections }
}

/// Order-insensitive comparison: the contract does not pin the order in which
/// independent rules fire, only the total per replacement.
fn normalize(hits: Vec<(String, usize)>) -> Vec<(String, usize)> {
    let mut v = hits;
    v.sort();
    v
}

// ---------------------------------------------------------------------- tests

#[test]
fn contract_file_is_present_and_non_trivial() {
    let v = load();
    assert_eq!(v.version, 1, "unexpected contract version");
    assert!(v.cases.len() >= 15, "expected the full vector set, got {}", v.cases.len());
}

#[test]
fn every_vector_passes() {
    let v = load();
    let mut failures: Vec<String> = Vec::new();

    for case in &v.cases {
        let corrector = DictionaryCorrector::from_dict(build(&case.entries));
        let (got, counts) = corrector.correct_with_counts(&case.input);

        if got != case.expected {
            failures.push(format!(
                "[{}]\n    input:    {:?}\n    expected: {:?}\n    actual:   {:?}",
                case.name, case.input, case.expected, got
            ));
            continue;
        }

        let want = normalize(
            case.expected_corrections
                .iter()
                .map(|h| (h.to.clone(), h.count))
                .collect(),
        );
        let have = normalize(counts);
        if want != have {
            failures.push(format!(
                "[{}]\n    input:      {:?}\n    expected corrections: {want:?}\n    actual corrections:   {have:?}",
                case.name, case.input
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} contract vectors failed:\n  - {}",
        failures.len(),
        v.cases.len(),
        failures.join("\n  - ")
    );
}

/// A contract file that stops being exercised is worse than no contract at all,
/// so assert we are actually running cases rather than silently passing on an
/// empty list.
#[test]
fn vectors_actually_exercise_correction() {
    let v = load();
    let correcting: Vec<&Case> = v
        .cases
        .iter()
        .filter(|c| !c.expected_corrections.is_empty())
        .collect();
    assert!(
        correcting.len() >= 10,
        "only {} vectors assert a correction fired; the contract looks gutted",
        correcting.len()
    );
}