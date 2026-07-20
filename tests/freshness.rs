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

/// The acme `refund-cap` claim (prose-only, `lm:` anchor into Elixir `changeset/2`),
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
         \x20   anchor: lm:notes-mark\n\
         ```\n",
    );
    // `.txt` has no bundled parser, so the claim's digest is null and freshness must
    // fall to the churn count. The landmark sits on line 1; later edits append below it,
    // so `resolved` stays notes.txt:1 and the byte-compare survives.
    fixture.write(
        "notes.txt",
        "# stele:landmark notes-mark\nline one\nline two\n",
    );
    fixture.write(".stele/config.toml", "[freshness]\nchurn_threshold = 1\n");
    fixture.commit("import churn fixture");

    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // Two commits touching the anchored file — one over the threshold of 1.
    fixture.append("notes.txt", "line three\n");
    fixture.commit("edit notes 1");
    fixture.append("notes.txt", "line four\n");
    fixture.commit("edit notes 2");

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("freshness"), "{out}");
    assert!(out.contains("notes.txt"), "{out}");
    assert!(out.contains("2 commits"), "{out}");
}

// §4.5 fallback disabled: with no `freshness.churn_threshold` configured, a parser-less
// anchor never stales however much its file churns (unset threshold → fallback off).
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
         \x20   anchor: lm:notes-mark\n\
         ```\n",
    );
    fixture.write("notes.txt", "# stele:landmark notes-mark\nline one\n");
    fixture.commit("import churn fixture (no config)");

    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");
    fixture.append("notes.txt", "line two\n");
    fixture.commit("edit notes");

    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// §5.1 `stele blame`: an up-to-date claim on a clean tree reports fresh (exit 0).
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

// §5.1 `stele blame`: an address naming no claim is an input error (exit 2).
#[test]
fn blame_on_unknown_claim_is_exit_2() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    let blame = fixture.run(&["blame", "billing/nonexistent"]);
    assert_eq!(blame.code, 2, "{}", blame.combined());
}

// P5 (F11/F12): in a depth-1 shallow clone the watermark commit is unreachable, so churn
// is uncountable. `check` must surface an honest freshness finding — "history unavailable
// (shallow clone?)" — never silently pass a possibly-staled parser-less claim (F11), and
// `blame` must say the same rather than reporting "0 commit(s)".
#[test]
fn shallow_clone_reports_history_unavailable_not_silent_pass() {
    let src = Fixture::bare();
    // A parser-less claim: a landmark in a .txt file (no bundled parser → churn fallback),
    // with churn_threshold 0 so any post-watermark churn would trip it.
    src.write(".stele/config.toml", "[freshness]\nchurn_threshold = 0\n");
    src.write("notes.txt", "intro\n# stele:landmark doc-rule\nmore\n");
    src.write(
        "AGENTS.md",
        "# proj\n\n```stele\nkind: system\npurpose: shallow-clone probe\ninvariants:\n\
         \x20 - claim: the documented rule holds\n    anchor: lm:doc-rule\n```\n\n\
         <!-- stele:begin router -->\n<!-- stele:end -->\n",
    );
    src.commit("c1: author the node + config");
    assert_eq!(src.run(&["build"]).code, 0); // stamps verified.sha = c1

    // Churn the anchored file and commit (the lock rides along); HEAD is now c2, but the
    // lock's watermark still points at c1.
    src.write(
        "notes.txt",
        "intro\n# stele:landmark doc-rule\nmore\nEDIT\n",
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
