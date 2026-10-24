//! Import-extraction integration tests (SPEC §2.3 `imports`, §4.2 territory
//! attribution). Each test materializes a tiny two-node tree — two AGENTS.md
//! blocks over real source files — in a git repo, runs `stele build`, and reads
//! the resulting `extracted.imports` out of the lock. The invariants under test,
//! per language: a cross-node import is recorded, a same-node reference is
//! dropped, an external package is ignored, duplicates collapse, and the arrays
//! are deterministic across builds.

mod common;

use common::Fixture;
use std::collections::BTreeMap;
use stele::lock::{self, Lock};

const LOCK_PATH: &str = ".stele/graph.lock";

/// The system node's stele block: a bare root so every file has a territory owner.
const ROOT_BLOCK: &str = "# root\n\n```stele\nkind: system\npurpose: root\n```\n";

/// Build `fixture` and return `node id → extracted.imports` for every node.
fn imports_of(fixture: &Fixture) -> BTreeMap<String, Vec<String>> {
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "build failed: {}", build.combined());
    let lock: Lock = lock::parse_lock(&fixture.read(LOCK_PATH)).expect("lock parses");
    lock.nodes
        .into_iter()
        .map(|(id, node)| (id, node.extracted.imports))
        .collect()
}

/// A container-node stele block declaring the given id's directory as its own.
fn node_block(name: &str) -> String {
    format!("# {name}\n\n```stele\nkind: component\npurpose: {name}\n```\n")
}

// ─── Elixir ───────────────────────────────────────────────────────────────────

// Elixir: an `alias` to a module a sibling node defines is a cross-node import; a
// reference to a module the same node owns is dropped; an external module (no
// `defmodule` anywhere) is ignored; a duplicate reference collapses to one entry.
#[test]
fn elixir_records_cross_node_drops_same_node_ignores_external() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", ROOT_BLOCK);
    fixture.write("apps/web/AGENTS.md", &node_block("web"));
    fixture.write("packages/shared/AGENTS.md", &node_block("shared"));

    // shared defines AcmeShared.Money.
    fixture.write(
        "packages/shared/lib/money.ex",
        "defmodule AcmeShared.Money do\n  def new(c), do: c\nend\n",
    );
    // web defines two modules; one aliases the other (same node → dropped), both
    // alias shared's Money (cross-node, twice → deduped), plus an external.
    fixture.write(
        "apps/web/lib/charge.ex",
        "defmodule AcmeWeb.Charge do\n  alias AcmeShared.Money\n  alias AcmeWeb.Refund\n  require Logger\n  def f, do: Money.new(1)\nend\n",
    );
    fixture.write(
        "apps/web/lib/refund.ex",
        "defmodule AcmeWeb.Refund do\n  alias AcmeShared.Money\n  def g, do: Money.new(2)\nend\n",
    );
    // A tracked source gone from the working tree is skipped, never read.
    fixture.write("apps/web/lib/gone.ex", "");
    fixture.commit("elixir fixture");
    fixture.delete_file("apps/web/lib/gone.ex");

    let imports = imports_of(&fixture);
    assert_eq!(imports["apps/web"], vec!["packages/shared"]);
    assert!(
        imports["packages/shared"].is_empty(),
        "shared should import nothing: {:?}",
        imports["packages/shared"]
    );
}

// ─── TypeScript / JavaScript ──────────────────────────────────────────────────

// TS/JS: a relative import, `require` or string-literal dynamic `import()` crossing a node
// boundary resolves by extension- or index-probing and is recorded (a computed `import()`
// is ignored); a bare specifier naming no workspace package is external and ignored; a
// workspace package-name (or subpath) import resolves through `package.json`, an
// unreadable one naming nothing.
#[test]
fn ts_records_relative_cross_node_and_workspace_package() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", ROOT_BLOCK);
    fixture.write("web/AGENTS.md", &node_block("web"));
    fixture.write("shared/AGENTS.md", &node_block("shared"));

    fixture.write(
        "shared/package.json",
        "{ \"name\": \"@acme/shared\", \"main\": \"src/money.ts\" }\n",
    );
    fixture.write(
        "shared/src/money.ts",
        "export function money(c: number) {\n  return c;\n}\n",
    );
    // web imports shared every way (relative, workspace name, subpath, require → deduped
    // to one edge) plus an external package and same-node imports (dropped).
    fixture.write(
        "web/index.ts",
        "import { money } from \"../shared/src/money\";\nimport { money as m } from \"@acme/shared\";\nimport { money as s } from \"@acme/shared/src/money\";\nimport express from \"express\";\nimport { local } from \"./helper\";\nimport { lib } from \"./lib\";\nimport { gone } from \"./gone\";\nexport { money, m, s, express, local, lib, gone };\n",
    );
    fixture.write("web/helper.tsx", "export const local = 1;\n");
    fixture.write(
        "shared/src/lazy.ts",
        "export const lazy = () => import(\"../../web/helper\");\nexport const any = (p: string) => import(p);\n",
    );
    fixture.write("web/lib/index.ts", "export const lib = 1;\n");
    fixture.write(
        "web/legacy.js",
        "require(\"../shared/src/money\");\nrequire(\"\");\n",
    );
    fixture.write("web/package.json", "not json\n");
    fixture.write("gone/package.json", "");
    fixture.commit("ts fixture");
    fixture.delete_file("gone/package.json");

    let imports = imports_of(&fixture);
    assert_eq!(imports["web"], vec!["shared"]);
    assert_eq!(imports["shared"], vec!["web"]);
}

// ─── Rust ─────────────────────────────────────────────────────────────────────

// Rust: a `use` or `extern crate` whose head is another workspace member's crate name
// is a cross-node import (hyphens folded to underscores); `crate::`/`std::` never
// cross; duplicate `use`s of the same crate collapse; a virtual-workspace or unreadable
// `Cargo.toml` names no crate.
#[test]
fn rust_records_workspace_member_use_ignores_std_and_intra_crate() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", ROOT_BLOCK);
    fixture.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    fixture.write("crates/gone/Cargo.toml", "");
    fixture.write("crates/web/AGENTS.md", &node_block("web"));
    fixture.write("crates/shared/AGENTS.md", &node_block("shared"));

    fixture.write(
        "crates/shared/Cargo.toml",
        "[package]\nname = \"acme-shared\"\nversion = \"0.1.0\"\n",
    );
    fixture.write("crates/shared/src/lib.rs", "pub fn money() {}\n");
    fixture.write(
        "crates/web/Cargo.toml",
        "[package]\nname = \"acme-web\"\nversion = \"0.1.0\"\n",
    );
    // web uses acme_shared twice (deduped), plus std and its own crate (dropped).
    fixture.write(
        "crates/web/src/lib.rs",
        "extern crate acme_shared;\nuse acme_shared::money;\nuse acme_shared::money as m;\nuse std::collections::BTreeMap;\nuse crate::helper;\nmod helper {}\n",
    );
    fixture.commit("rust fixture");
    fixture.delete_file("crates/gone/Cargo.toml");

    let imports = imports_of(&fixture);
    assert_eq!(imports["crates/web"], vec!["crates/shared"]);
    assert!(imports["crates/shared"].is_empty());
}

// ─── Python ───────────────────────────────────────────────────────────────────

// Python: an `import`/`from`-`import` naming a dotted module another node defines
// is a cross-node import; a same-node module reference is dropped; a stdlib/pip
// module that maps to no tracked file is ignored; duplicates collapse. With no system
// node, a file outside every territory imports nothing.
#[test]
fn python_records_dotted_module_drops_same_node_ignores_stdlib() {
    let fixture = Fixture::bare();
    fixture.write("web/AGENTS.md", &node_block("web"));
    fixture.write("shared/AGENTS.md", &node_block("shared"));

    fixture.write("shared/money.py", "def money(c):\n    return c\n");
    // web imports shared.money three ways (deduped), a stdlib module (ignored), and a
    // sibling web module (same node → dropped).
    fixture.write(
        "web/app.py",
        "import shared.money\nimport shared.money as sm\nfrom shared.money import money\nimport os\nimport web.helper\n",
    );
    fixture.write("web/helper.py", "helper = 1\n");
    fixture.write("web/gone.py", "");
    // Outside every territory: misattributed to shared, top.py would make shared import web.
    fixture.write("top.py", "import web.helper\n");
    fixture.commit("python fixture");
    fixture.delete_file("web/gone.py");

    let imports = imports_of(&fixture);
    assert_eq!(imports["web"], vec!["shared"]);
    assert!(imports["shared"].is_empty());
}

// ─── determinism ──────────────────────────────────────────────────────────────

// Two consecutive builds of a multi-language tree produce byte-identical locks:
// extraction order is a pure function of the sorted tracked set (§3.2).
#[test]
fn extraction_is_deterministic_across_runs() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let first = fixture.read(LOCK_PATH);
    assert_eq!(fixture.run(&["build"]).code, 0);
    let second = fixture.read(LOCK_PATH);
    assert_eq!(first, second, "second build diverged from the first");
}

// The acme fixture's billing node imports exactly store + shared (EXAMPLE §6), and
// a container's test file importing a child component is attributed to the
// container (§4.2 territory, non-inheriting).
#[test]
fn acme_billing_imports_match_the_worked_example() {
    let fixture = Fixture::acme();
    let imports = imports_of(&fixture);
    assert_eq!(
        imports["apps/web/lib/billing"],
        vec!["apps/web/lib/store", "packages/shared"],
    );
    // The store node imports shared (subscription.ex aliases AcmeShared.Money).
    assert_eq!(imports["apps/web/lib/store"], vec!["packages/shared"]);
    // Leaf providers and the worker import nothing across a boundary.
    assert!(imports["packages/shared"].is_empty());
    assert!(imports["apps/worker"].is_empty());
}
