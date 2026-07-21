# stele — guide

The five-minute path from a bare repo to a CI-enforced agent-doc graph. The [SPEC](./SPEC.md) is the contract and covers every rule in depth; [EXAMPLE.md](./EXAMPLE.md) is a full worked example. This guide is just the happy path.

## 1 · Install

```sh
curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash
```

Pin a version with `… | bash -s v0.1.1`; prereleases with `… | channel=canary bash`. The binary lands in `~/.local/bin/stele`. From source: `cargo install --git https://github.com/jgeschwendt/stele stele-cli`.

## 2 · Scaffold

From the repo root (must be a git repository):

```sh
stele init
```

This scaffolds `AGENTS.md` skeletons — a `system` node at the root, `container` nodes for candidate directories — and stages them. Each skeleton is a plain markdown file opening with a fenced `stele` block:

````markdown
# payments

```stele
kind: container
purpose: Payment capture and refund flow. Stripe is the only processor.
commands:
  test: cargo test -p payments
invariants:
  - claim: every charge is idempotent — retries must reuse the idempotency key
    anchor: src/charge.rs#create_charge
hazards:
  - claim: refund webhooks arrive out of order; never assume capture precedes refund
    anchor: lm:webhook-dispatch
```

Anything below the block is yours — ordinary markdown, untouched by the engine.
````

Fill in `purpose` (what an agent can't derive from the tree), real `commands`, and the claims worth enforcing. Delete skeletons for directories that don't deserve a node — shallow is fine; the graph should follow interface boundaries, not mirror the tree.

## 3 · Anchor your claims

Every invariant or hazard needs an anchor tying it to code, one of:

- **`path#symbol`** — a definition in that file (`src/charge.rs#create_charge`), or a heading slug in a markdown file (`SPEC.md#decision-log`).
- **`lm:<slug>`** — a named landmark: put a `stele:landmark <slug>` comment on the definition in the source file. Survives renames and moves that `path#symbol` doesn't.

Anchors are what make claims checkable: when the anchored code changes, the claim goes stale and CI says so.

## 4 · Build and commit

```sh
stele build   # compiles authored sources → .stele/graph.lock (+ .stele/index/)
stele emit    # renders router regions into AGENTS.md, transpose indexes, the CLAUDE.md shim
```

Commit everything: the `AGENTS.md` files, `.stele/graph.lock`, `.stele/index/`. The lock is the canonical graph — queries and CI read it, never re-derive it.

`build` is the reconciliation point: it re-stamps each claim's `verified` mark against the current code. Run it locally when you've reviewed that a claim still holds; **CI never runs `stele build`** — that would launder staleness.

Paths the engine should never scan (fixtures, vendored trees) go in a `.steleignore` at the root — gitignore syntax.

## 5 · Enforce in CI

```yaml
stele:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v5
      with:
        fetch-depth: 0        # full history — freshness checks use blame
    - run: curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash -s v0.1.1
    - run: ~/.local/bin/stele check
    - run: ~/.local/bin/stele emit --check
```

`stele check` runs six assertion classes over lock vs. code and exits nonzero with findings:

| class | catches |
| --- | --- |
| referential | anchors pointing at symbols/landmarks/files that no longer exist |
| structural | declared dependencies the code doesn't have — and code dependencies never declared |
| exhaustiveness | directories no node covers — unrouted context an agent can't navigate to |
| budget | node docs exceeding their token budget |
| freshness | anchored code changed since the claim was last verified (`stele blame <node>/<slug>` shows who/when) |
| liveness | documented `commands:` that no longer resolve or run |

`stele emit --check` fails if committed AGENTS.md files diverge from what the lock would render. Useful flags: `check --only <class>`, `check --run-commands` (actually execute `commands:`), `check --freeze` (accept current structure as baseline), `--json` everywhere for the machine envelope.

## 6 · Query it

Agents (and you) read the graph instead of grepping docs:

```sh
stele root                       # the system node — start here
stele unfold payments --depth 2  # a node with its children
stele invariants --touching src/charge.rs   # claims governing a file you're editing
stele hazards                    # every active hazard
stele nodes --kind container
stele blame payments/idempotent-charges     # provenance of one claim
```

For MCP-capable harnesses, `stele serve` speaks JSON-RPC over stdio and exposes the same read verbs as tools:

```json
{ "mcpServers": { "stele": { "command": "stele", "args": ["serve"] } } }
```

## 7 · No engine? Nothing breaks

The emitted `AGENTS.md` files are complete, standard-compliant markdown — every harness that reads AGENTS.md gets the full graph content with zero tooling. The degradation ladder is deliberate: plain files → CLI queries → MCP. The engine adds verification and lazy navigation; it never becomes a reading dependency.

## 8 · Undercover mode

When you're the *only* stele user on a shared repo — a work, upstream, or open-source checkout whose collaborators must never see a stele artifact — run it privately:

```sh
stele init --undercover
```

Everything the engine writes stays out of version control, analogous to `CLAUDE.local.md`. Only the marker's presence selects the mode; without it every command is byte-for-byte normal.

| where | what |
| --- | --- |
| graph home | parent of `git rev-parse --git-common-dir` — the repo root (normal checkout) or the shared bare root (grove) |
| `<home>/.stele/tree/<dir>/AGENTS.md` | node sources — the overlay, mirroring tree paths (root node at `.stele/tree/AGENTS.md`) |
| `<home>/.stele/graph.lock` + `.stele/index/` | the private lock and transpose indexes |
| `<work>/CLAUDE.local.md` | the one materialized file — a relative `@`-import of the overlay root, auto-loaded by Claude Code (SPEC [§3.5](./SPEC.md#35-undercover-mode)) |
| `<common-dir>/info/exclude` | a managed block hiding `/.stele/` and `/CLAUDE.local.md`, so `git status` stays clean |

Author node blocks under `.stele/tree/`, then `stele build` + `stele emit` + `stele check` exactly as normal — every artifact lands at the home; queries and MCP resolve it from any worktree.

**Worktrees share one graph.** The home is the parent of the git *common* dir, so every linked worktree — and every sibling in a bare-root (grove) layout — reads the same private graph, which survives worktree churn. `emit` drops a `CLAUDE.local.md` in whichever worktree you run it from, and freshness measures against *that* worktree's HEAD.

**What's different.** There's no CI story — nothing is committed for a pipeline to gate, so `stele check` is yours to run locally. The degradation ladder trades its files rung for invisibility: collaborators see nothing, and your surface is CLI → MCP plus the one shim. `emit --claude-rules` is unavailable (the single shim is the only materialized file). And undercover cannot coexist with a tracked shared graph — a repo already carrying a committed `.stele/` or `stele`-block `AGENTS.md` refuses `init --undercover` (exit 2); mixing the two is a v2 concern.
