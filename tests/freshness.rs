//! The freshness class (SPEC §4.5) and `stele blame` (§5.1), driven through the real
//! binary on scratch fixtures. Freshness recomputes each resolved claim's bound-region
//! digest from the working tree and stales prose-only claims on an AST change, while
//! exempting `enforced_by`-backed claims (their guard is the proof) and ignoring
//! formatting/comment churn (the AST drops it). Parser-less anchors fall to a
//! commit-count fallback thresholded per node. `blame` walks history to the staling
//! commit. Every case builds a clean baseline, mutates WITHOUT a rebuild (so the lock's
//! `verified` watermark is the point of comparison), then asserts `check`/`blame`.

mod common;

use common::Fixture;

/// The acme `refund-cap` claim (prose-only, `※ ` anchor into Elixir `changeset/2`),
/// mutated so the digested region drifts without touching the claim — the §8.4 setup,
/// left uncommitted-of-lock (build ran at the clean baseline, never re-stamped).
fn drifted_refund_cap() -> Fixture {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // A semantic edit INSIDE changeset/2; the landmark line above it does not move, so
    // the rebuilt lock still byte-matches (resolved unchanged) and check reaches §4.5.
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "|> cast(attrs, @castable_fields)",
        "|> cast(attrs, [:amount_cents, :charge_id, :reason])",
    );
    fixture.commit("loosen cap for partial captures");
    fixture
}

// §4.5 primary signal: an AST-region digest drift stales a prose-only claim, naming the
// bound region and the staling commit (EXAMPLE 8.4).
#[test]
fn digest_drift_names_region_and_staling_commit() {
    let fixture = drifted_refund_cap();
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("freshness"), "{out}");
    assert!(
        out.contains("AST digest of enclosing region changed"),
        "{out}"
    );
    // The bound region is the immediately-following Elixir def, named with its arity.
    assert!(out.contains("region changeset/2"), "{out}");
    // The staling commit is identified by subject, with a `stele blame` pointer.
    assert!(out.contains("loosen cap for partial captures"), "{out}");
    assert!(out.contains("stele blame billing/refund-cap"), "{out}");
}

// §4.5 exemption: an `enforced_by`-backed claim is NOT staled by a digest change — its
// guard, run by the same CI, is the freshness proof (EXAMPLE 8.4 note 2). Mutating
// billing-idempotency's bound region (`create/2`) in place must leave check clean.
#[test]
fn enforced_by_backed_claim_is_exempt_from_digest_staling() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // An in-place AST change (adds an argument) inside `create/2`, the region bound by
    // the enforced_by-backed `billing-idempotency` landmark. No line shifts, so other
    // landmarks keep their resolved lines and the byte-compare still passes.
    fixture.replace(
        "apps/web/lib/billing/charge.ex",
        "|> Money.normalize()",
        "|> Money.normalize(account_id)",
    );
    fixture.commit("change the enforced idempotency region");

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 0, "{out}");
    assert!(!out.contains("freshness"), "{out}");
}

// §4.5 stability: a formatting/comment change inside the digested region does NOT fire —
// the AST digest ignores comments and whitespace (EXAMPLE 8.4 note 1).
#[test]
fn formatting_change_does_not_stale() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // A comment added INSIDE changeset/2 — below the landmark, so nothing shifts — is
    // dropped from the structural digest, leaving the region digest unchanged.
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "    |> validate_refund_cap()",
        "    # a note that must not stale the claim\n    |> validate_refund_cap()",
    );
    fixture.commit("comment inside the changeset region");

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 0, "{out}");
    assert!(!out.contains("freshness"), "{out}");

    // A later semantic edit stales the claim; the walk steps past the comment-only commit
    // (its digest still matches) and names the semantic one.
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "|> cast(attrs, @castable_fields)",
        "|> cast(attrs, [:amount_cents])",
    );
    fixture.commit("narrow the cast");
    let out = fixture.run(&["check"]).combined();
    assert!(
        out.contains("staling commit: ") && out.contains("\"narrow the cast\""),
        "{out}"
    );
}

// §4.5 fallback: a parser-less anchor (no bundled parser → no digest) stales on a churn
// count — commits touching the anchored file since verified.sha, past the per-config
// threshold. Region granularity is the whole file (documented v1 limitation).
#[test]
fn parserless_churn_fallback_fires_past_threshold() {
    let fixture = Fixture::bare();
    fixture.write(
        "AGENTS.md",
        "# churn-fixture\n\n```stele\n\
         kind: system\n\
         purpose: parser-less churn fallback fixture\n\
         invariants:\n\
         \x20 - claim: the notes stay accurate to the code\n\
         \x20   anchor: ※ notes-mark\n\
         ```\n",
    );
    // `.txt` has no bundled parser, so the claim's digest is null and freshness must
    // fall to the churn count. The landmark sits on line 1; later edits append below it,
    // so `resolved` stays notes.txt:1 and the byte-compare survives.
    fixture.write("notes.txt", "# ※ notes-mark\nline one\nline two\n");
    fixture.write(".stele/config.toml", "[freshness]\nchurn_threshold = 1\n");
    fixture.commit("import churn fixture");

    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // Two commits touching the anchored file — the first is AT the threshold of 1 (fresh),
    // the second over it.
    fixture.append("notes.txt", "line three\n");
    fixture.commit("edit notes 1");
    assert_eq!(fixture.run(&["check"]).code, 0);
    fixture.append("notes.txt", "line four\n");
    fixture.commit("edit notes 2");

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("freshness"), "{out}");
    assert!(out.contains("notes.txt"), "{out}");
    assert!(out.contains("2 commits"), "{out}");
    // `blame` reports the same churn count for the parser-less anchor.
    let blame = fixture.run(&["blame", "/notes-mark"]).combined();
    assert!(
        blame.contains("notes.txt touched by 2 commit(s)"),
        "{blame}"
    );
}

// The churn count reads the anchored path literally: `:notes.txt` is no pathspec magic
// (its edit counts) and `[ab].txt` no glob (an `a.txt`-only edit does not).
#[test]
fn churn_count_reads_the_anchored_path_literally() {
    for (file, edited, stale) in [
        (":notes.txt", ":notes.txt", true),
        ("[ab].txt", "a.txt", false),
    ] {
        let fixture = Fixture::bare();
        fixture.write(
            "AGENTS.md",
            "# churn\n\n```stele\nkind: system\ninvariants:\n  - claim: notes hold\n    anchor: ※ notes-mark\n```\n",
        );
        fixture.write(file, "# ※ notes-mark\n");
        fixture.write("a.txt", "");
        fixture.write(".stele/config.toml", "[freshness]\nchurn_threshold = 0\n");
        fixture.commit("import");
        assert_eq!(fixture.run(&["build"]).code, 0);
        fixture.commit("lock");
        fixture.append(edited, "edit\n");
        fixture.commit("edit");
        let check = fixture.run(&["check", "--only", "freshness"]);
        assert_eq!(check.code == 1, stale, "{file}: {}", check.combined());
    }
}

// §4.5 fallback disabled: with neither `freshness.churn_threshold` nor (for the
// `enforced_by`-backed claim) `freshness.enforced_leash` configured, a parser-less anchor
// never stales however much its file churns (unset threshold → fallback off).
#[test]
fn parserless_churn_disabled_without_a_threshold() {
    let fixture = Fixture::bare();
    fixture.write(
        "AGENTS.md",
        "# churn-fixture\n\n```stele\n\
         kind: system\n\
         purpose: parser-less churn fallback fixture\n\
         invariants:\n\
         \x20 - claim: the notes stay accurate to the code\n\
         \x20   anchor: ※ notes-mark\n\
         \x20 - claim: the guarded notes stay accurate\n\
         \x20   anchor: ※ notes-guard\n\
         \x20   enforced_by: notes.txt\n\
         ```\n",
    );
    fixture.write("notes.txt", "# ※ notes-mark\n# ※ notes-guard\nline one\n");
    fixture.commit("import churn fixture (no config)");

    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");
    fixture.append("notes.txt", "line two\n");
    fixture.commit("edit notes");

    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// §5.1 `stele blame`: an up-to-date claim on a clean tree reports fresh (exit 0). An
// uncommitted edit then stales it with no staling commit (`null` under `--json`) — unless the
// watermark names no commit; a lock
// stamped over that dirty tree, with the edit reverted, diverges from no committed
// version and no working-tree edit — the honest answer is "history unavailable".
#[test]
fn blame_reports_up_to_date_on_clean_tree() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    let blame = fixture.run(&["blame", "billing/refund-cap"]);
    let out = blame.combined();
    assert_eq!(blame.code, 0, "{out}");
    assert!(out.contains("up to date"), "{out}");
    assert!(out.contains("changeset/2"), "{out}");

    let refund = "apps/web/lib/billing/refund.ex";
    let clean = fixture.read(refund);
    fixture.replace(refund, "@castable_fields)", "[:amount_cents])");
    let out = fixture.run(&["blame", "billing/refund-cap"]).combined();
    assert!(out.contains("(uncommitted working-tree change)"), "{out}");
    let json = fixture
        .run(&["blame", "billing/refund-cap", "--json"])
        .stdout;
    assert!(json.contains("\"staling_commit\":null"), "{json}");
    // A watermark naming no commit (a hand-edited `"sha": ""`) is history unavailable, never
    // the working-tree edit an empty `..HEAD` range would suggest.
    let lock = fixture.read(".stele/graph.lock");
    let mut shas = lock.split("\"sha\": \"");
    let head = shas.next().unwrap().to_string();
    fixture.write(
        ".stele/graph.lock",
        &shas.fold(head, |out, rest| out + "\"sha\": \"" + &rest[40..]),
    );
    let out = fixture.run(&["blame", "billing/refund-cap"]).combined();
    assert!(
        out.contains("history unavailable (shallow clone?)"),
        "{out}"
    );

    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.write(refund, &clean);
    let out = fixture.run(&["blame", "billing/refund-cap"]).combined();
    assert!(
        out.contains("STALE") && out.contains("history unavailable"),
        "{out}"
    );
}

// §5.1 `stele blame`: a drifted claim reports STALE and names the staling commit; the
// abbreviated node-id resolves the same claim as the full id.
#[test]
fn blame_reports_staling_commit_on_drift() {
    let fixture = drifted_refund_cap();
    let blame = fixture.run(&["blame", "billing/refund-cap"]);
    let out = blame.combined();
    assert_eq!(blame.code, 0, "{out}");
    assert!(out.contains("STALE"), "{out}");
    assert!(out.contains("loosen cap for partial captures"), "{out}");
}

// §5.1 `stele blame`: an address naming no claim, or an abbreviation a twin `billing`
// node makes ambiguous, is an input error (exit 2); a claim whose anchor stopped
// resolving since the last build says so.
#[test]
fn blame_on_unknown_claim_is_exit_2() {
    let fixture = Fixture::acme();
    fixture.write(
        "packages/billing/AGENTS.md",
        "# billing\n\n```stele\nkind: container\npurpose: twin final segment\n```\n",
    );
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    let blame = fixture.run(&["blame", "store/nonexistent"]);
    assert_eq!(blame.code, 2, "{}", blame.combined());
    let blame = fixture.run(&["blame", "billing/refund-cap"]);
    assert_eq!(blame.code, 2, "{}", blame.combined());
    assert!(
        blame.combined().contains("is ambiguous"),
        "{}",
        blame.combined()
    );

    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "※ refund-cap",
        "※ refund-cap-v2",
    );
    let out = fixture
        .run(&["blame", "apps/web/lib/billing/refund-cap"])
        .combined();
    assert!(out.contains("anchor no longer resolves"), "{out}");
}

// P5 (F11/F12): in a depth-1 shallow clone the watermark commit is unreachable, so churn
// is uncountable. `check` must surface an honest freshness finding — "history unavailable
// (shallow clone?)" — never silently pass a possibly-staled parser-less claim (F11), and
// `blame` must say the same rather than reporting "0 commit(s)". A parser-backed claim's
// digest drift likewise names no staling commit. Before any commit (unborn HEAD) nothing
// is watermarked, so freshness has nothing to compare and `blame` says "not verified".
#[test]
fn shallow_clone_reports_history_unavailable_not_silent_pass() {
    let src = Fixture::bare();
    // A parser-less claim: a landmark in a .txt file (no bundled parser → churn fallback),
    // with churn_threshold 0 so any post-watermark churn would trip it.
    src.write(".stele/config.toml", "[freshness]\nchurn_threshold = 0\n");
    src.write("notes.txt", "intro\n# ※ doc-rule\nmore\n");
    src.write(
        "code.ts",
        "// ※ code-rule\nexport function f() { return 1; }\n",
    );
    src.write(
        "AGENTS.md",
        "# proj\n\n```stele\nkind: system\npurpose: shallow-clone probe\ninvariants:\n\
         \x20 - claim: the documented rule holds\n    anchor: ※ doc-rule\n\
         \x20 - claim: the code rule holds\n    anchor: ※ code-rule\n```\n\n\
         <!-- @stele -->\n<!-- @end -->\n",
    );
    src.stage_all();
    assert_eq!(src.run(&["build"]).code, 0); // unborn HEAD: verified stays null
    assert_eq!(src.run(&["check"]).code, 0);
    let unborn = src.run(&["blame", "/doc-rule"]).combined();
    assert!(
        unborn.contains("not verified in the committed lock"),
        "{unborn}"
    );
    src.commit("c1: author the node + config");
    assert_eq!(src.run(&["build"]).code, 0); // stamps verified.sha = c1

    // Churn the anchored files and commit (the lock rides along); HEAD is now c2, but the
    // lock's watermark still points at c1.
    src.write("notes.txt", "intro\n# ※ doc-rule\nmore\nEDIT\n");
    src.write(
        "code.ts",
        "// ※ code-rule\nexport function f() { return 2; }\n",
    );
    src.commit("c2: churn the anchored file");

    // depth-1 clone: only c2 is present, so the c1 watermark is unreachable.
    let clone = src.shallow_clone();
    let check = clone.run(&["check"]);
    let out = check.combined();
    assert_eq!(
        check.code, 1,
        "shallow clone must not silently pass:\n{out}"
    );
    assert!(
        out.contains("history unavailable (shallow clone?)"),
        "check must name the honest failure:\n{out}"
    );
    assert!(out.contains("staling commit: history unavailable"), "{out}");
    let json = clone.run(&["blame", "/code-rule", "--json"]).stdout;
    assert!(
        json.contains("{\"status\":\"history-unavailable\"}"),
        "{json}"
    );

    let blame = clone.run(&["blame", "/doc-rule"]);
    let bout = blame.combined();
    assert!(
        bout.contains("history unavailable"),
        "blame must not report a false churn count:\n{bout}"
    );
    assert!(
        !bout.contains("touched by 0 commit"),
        "blame must not read the unreachable watermark as zero churn:\n{bout}"
    );
}

// §4.5 binding through a wrapper: a landmark above `export function a` or a decorated
// `def a` digests the wrapper, named `a`, so editing the export modifier or the decorator
// stales the claim and editing the next definition `b` does not.
#[test]
fn landmark_above_an_export_or_decorator_binds_the_wrapped_definition() {
    for (file, src, wrapper, edited) in [
        (
            "m.ts",
            "// ※ cap-a\nexport function a() { return 1; }\n\nfunction b() { return 2; }\n",
            "export function",
            "export default function",
        ),
        (
            "m.py",
            "# ※ cap-a\n@staticmethod\ndef a(): return 1\n\ndef b(): return 2\n",
            "@staticmethod",
            "@classmethod",
        ),
    ] {
        let fixture = Fixture::bare();
        fixture.write(
            "AGENTS.md",
            "# p\n\n```stele\nkind: system\npurpose: wrapper probe\ninvariants:\n\
             \x20 - claim: a is capped\n    anchor: ※ cap-a\n```\n",
        );
        fixture.write(file, src);
        fixture.commit("import");
        assert_eq!(fixture.run(&["build"]).code, 0);
        fixture.commit("baseline graph.lock");

        fixture.replace(file, "return 2", "return 9");
        fixture.commit("edit b");
        let check = fixture.run(&["check"]);
        assert_eq!(check.code, 0, "{file}: {}", check.combined());

        fixture.replace(file, wrapper, edited);
        fixture.commit("edit the wrapper");
        let check = fixture.run(&["check"]);
        let out = check.combined();
        assert_eq!(check.code, 1, "{file}: {out}");
        assert!(out.contains("region a now digests"), "{file}: {out}");
    }
}
