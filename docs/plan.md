# Plan · glyph notation (0.3.0)

Status: in progress 2026-09-17 (phase 2) · owner: jlg · execution: opus agents per phase, session model reviews and gates.

## The change

One grammar, four glyphs plus an ASCII region marker. Slug lexeme, `path#symbol`, cardinality, node ids, ADR files, the `stele` fence and every file name stay as they are.

| slot       | was                                   | becomes                            |
| ---------- | ------------------------------------- | ---------------------------------- |
| landmark   | `// stele:landmark refund-cap`        | `// ※ refund-cap`                  |
| claim      | `// stele:claim billing/refund-cap`   | `// ⊨ billing/refund-cap`          |
| anchor:    | `lm:refund-cap`                       | `※ refund-cap`                     |
| decided_by | `adr/0007`                            | `§ 0007`                           |
| region     | `<!-- stele:begin router … -->` / `<!-- stele:end -->` | `<!-- @stele [name] -->` / `<!-- @end -->` |

Rules pinned now, so no phase re-decides them:

- **Token shape, everywhere.** A glyph token is the glyph, one ASCII space, the payload: `※ refund-cap`, `⊨ billing/refund-cap`, `§ 0007`. The same bytes in a comment, in `anchor:`, and in `decided_by:`, so the node file quotes the code verbatim. In comments the payload ends at whitespace or end of line. Payload is a slug (landmark) or `<node-id>/<slug>` (claim). Anything else after the glyph is prose and is ignored, never an error: `※` is a common annotation mark in CJK comments, so a non-matching `※` must be silent. A typo in a landmark that a node file references still fails referentially (cardinality 0, exit 1). Today's exit-2 on a malformed landmark slug goes away; the anchor field keeps exit 2.
- **`§ NNNN`** is the authored decision reference; it resolves against the detected ADR directory. The lock keeps the path-derived id `<adrdir>/<NNNN>` so `doc/adr` and `docs/adr` repos keep working.
- **`@stele`** takes an optional region name; absent means `router`. `@end` is bare. One region per file, no nesting, as before.
- **`✻`** stays reserved for human notes. `stele:` disappears from the grammar entirely; the undercover `info/exclude` fence (`# stele:begin undercover` … `# stele:end undercover`) is an internal managed block, not notation, and is unchanged so existing installs self-heal.
- **Hard cut, no dual-read.** `LOCK_VERSION` 1 → 2. A v1 lock fails `check`/`emit` with `run stele migrate, then stele build`. `stele migrate` rewrites the old grammar to the new in place, idempotent, over the same scan scope as `build` (tracked files, `.steleignore`, overlay in undercover). Version 0.2.0 → 0.3.0.
- **CLI addresses stay ASCII.** `stele blame billing/refund-cap`, `stele node billing`: no glyph is ever typed into a shell. Output prints the glyph forms.

## Phases

Chiastic: spec, then the core, then outward. Each phase ends with the lefthook chain green (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build && stele check && stele emit --check`) and one commit on main, on jlg's word.

1. **SPEC.** §2.4 anchor field, §2.5 tokens and the silent-prose rule, §2.6 `§NNNN`, §3.1 region markers, §3.2 lock v2, §5.1 `migrate` verb and the v1-lock error text, §8 decision-log entry "glyph notation" (rationale: zero collision without a fence; ※ renders in every CJK-capable font; ⊨ is "the region satisfies the claim", ⊢ is the test's job; ✻ kept for humans; `stele:` sigil entry superseded). research/claims.md: no new claim, ※/§ East-Asian-width "ambiguous" noted in the log entry. Agent: opus. Review: session.
2. **Engine core.** `src/model.rs` (`LANDMARK_ANCHOR_PREFIX` → `※ `, `derive_slug`, decided_by `§ ` parsing, ADR lock id unchanged), `src/anchors.rs` (glyph tokens, silent non-match, byte-level match on the UTF-8 sequence), `src/parse.rs` (`@stele [name]` / `@end`), `src/lock.rs` (`LOCK_VERSION = 2`), `src/emit.rs` (anchor tables, engine line unchanged), `src/assert.rs` + `src/cli.rs` + `src/serve.rs` (messages, `init` scaffolds the new region, `blame` output), all unit tests. `tests/fixtures/acme` rewritten to the new grammar; every integration test and `tests/gallery.rs` expectation updated. Green means the whole suite. Agent: opus, one brief. Review: session, with a fresh-eyes probe on the silent-prose rule (a fixture file carrying `※ 注意` and `※ note: x` must build clean).
3. **`stele migrate`.** New verb in `src/cli.rs` with the old tokens in a `legacy` module: comments (`stele:landmark x` → `※ x`, `stele:claim a` → `⊨ a`), node blocks (`anchor: lm:x` → `※ x`, `decided_by: [adr/0007]` → `[§ 0007]`), region markers (name preserved when not `router`). Idempotent, reports files touched, refuses nothing but warns on a dirty tree. Tests: a v1 acme copy migrates to byte-equality with the v2 fixture; a second run is a no-op; undercover overlay migrates. Agent: opus. Review: session.
4. **Self-host + docs outward.** `stele migrate` on this repo, `build`, `emit`, gate. EXAMPLE.md rewritten in the new grammar (docs/notation-sample.md is the seed), GUIDE.md, README.md (draft number, release version), docs/index.html (draft number, version strings), root AGENTS.md prose, `src/AGENTS.md` and siblings. Delete docs/notation-draft.md and docs/notation-sample.md: their content lives in SPEC §8 and EXAMPLE. Agent: opus. Review: session, cold whole-file read of EXAMPLE and SPEC.
5. **Release + downstream.** Tag v0.3.0 (release workflow ships the binary), reinstall locally (`cp` to `stele.new` then `mv -f`, never over the running binary). Then per downstream repo, on its trunk: `stele migrate && stele build && stele emit`, gate, commit — one word from jlg covers the batch. Repos: bridge, grove, jlg.io, routines, sandman, spawn, threads, tldraft, typescript, visor. Branch worktrees pick it up on merge; a branch that emits before merging gets a lock-version error pointing at `migrate`.

## Not in this change

- No dual-read of the old grammar. `migrate` is the bridge.
- No change to `.claude/rules` emission, the CLAUDE.md shims, or the undercover exclude fence.
- No second region kind; `@stele <name>` merely leaves room.

## Verification that closes it

`rg 'stele:(landmark|claim|begin|end)|\blm:' --glob '!docs/plan.md' --glob '!src/cli.rs'` returns nothing outside the `legacy` module; the acme fixture builds with a `※ 注意` prose line present; every downstream repo is green on `stele check && stele emit --check` with the 0.3.0 binary.
