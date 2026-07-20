//! Configuration loading and defaults (SPEC §3.4).
//!
//! Check-time settings live in `.stele/config.toml` (committed). It is NOT an input
//! to `build`/the lock — it tunes `check`/`emit`, never the graph — so `build` must
//! not read it. An absent file yields all defaults; every key is optional, and any
//! unknown key (or an unknown `check.disable` class) is a §5.3 input error (exit 2)
//! naming the offender. Strictness is enforced by `deny_unknown_fields` on every
//! table plus the closed [`AssertionClass`] enum for the disable list.

use crate::model::{Result, SteleError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

/// The committed config, relative to the repo root.
const CONFIG_PATH: &str = ".stele/config.toml";

/// SPEC §3.4 defaults for the tables whose absent keys are not simply `None`.
const DEFAULT_CLAUDE_ROOT_TOKENS: u32 = 2000;
const DEFAULT_CODEX_CAP_BYTES: u32 = 32768;
const DEFAULT_EXHAUSTIVENESS_DEPTH: u32 = 1;
const DEFAULT_EXCLUDE: &[&str] = &["node_modules", "_build", "deps", "target"];

/// The parsed `.stele/config.toml` (SPEC §3.4). Missing tables fall back to
/// [`Config::default`]; unknown top-level keys are rejected (exit 2).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub budget: Budget,
    pub check: Check,
    pub exhaustiveness: Exhaustiveness,
    pub freshness: Freshness,
}

/// `[budget]` — the §4.4 caps (SPEC §3.4).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Budget {
    /// Token cap on the rendered root context (default 2000).
    pub claude_root: u32,
    /// Byte cap on the per-harness Codex output (default 32768).
    pub codex_cap: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            claude_root: DEFAULT_CLAUDE_ROOT_TOKENS,
            codex_cap: DEFAULT_CODEX_CAP_BYTES,
        }
    }
}

/// `[check]` — the independently-toggleable assertion knob (SPEC §3.4).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Check {
    /// Assertion classes to skip; `--only <class>` remains the run-exactly-one flag.
    pub disable: Vec<AssertionClass>,
}

/// `[exhaustiveness]` — the §4.3 walk controls (SPEC §3.4).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Exhaustiveness {
    /// How deep undocumented-subtree detection descends (default 1).
    pub depth: u32,
    /// Globs excluded from the walk (default `node_modules`, `_build`, `deps`, `target`).
    pub exclude: Vec<String>,
}

impl Default for Exhaustiveness {
    fn default() -> Self {
        Self {
            depth: DEFAULT_EXHAUSTIVENESS_DEPTH,
            exclude: DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// `[freshness]` — the §4.5 leash (SPEC §3.4). Both thresholds are `None` when
/// unset; per-node entries under `[freshness.node."<id>"]` override the global pair.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Freshness {
    pub churn_threshold: Option<u32>,
    pub enforced_leash: Option<u32>,
    /// Per-node overrides keyed by node id; ids are arbitrary and never rejected.
    pub node: BTreeMap<String, NodeFreshness>,
}

/// A `[freshness.node."<id>"]` override (SPEC §3.4).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NodeFreshness {
    pub churn_threshold: Option<u32>,
    pub enforced_leash: Option<u32>,
}

/// The six assertion classes (SPEC §4). A closed enum so an unknown name in
/// `check.disable` is rejected (exit 2) rather than silently ignored.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AssertionClass {
    Budget,
    Exhaustiveness,
    Freshness,
    Liveness,
    Referential,
    Structural,
}

/// Load `.stele/config.toml` (SPEC §3.4). An absent file yields all defaults; an
/// unknown key or unknown disable-class is an input error (exit 2); a genuine IO
/// failure is an internal error (exit 3). `build` never calls this.
pub fn load(root: &Path) -> Result<Config> {
    let path = root.join(CONFIG_PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(SteleError::internal(format!("read {CONFIG_PATH}: {e}"))),
    };
    toml::from_str(&text).map_err(|e| SteleError::input_msg(format!("{CONFIG_PATH}: {e}")))
}
