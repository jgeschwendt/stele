//! Referential (§4.1): claim anchors resolve, landmark slugs are unique, `decided_by`
//! names a live ADR, `enforced_by` names a tracked file, and claim-glyph comments resolve.

use super::{Context, Finding};
use crate::config::AssertionClass;
use crate::model::{
    Claim, ClaimAnchor, ClaimLookup, LANDMARK_ANCHOR_PREFIX, Node, Resolution, Result, normalize_id,
};
use std::collections::{BTreeMap, BTreeSet};

/// The claim comment token (§2.5) — the glyph plus its single space — quoted ahead of
/// an address in the §4.1(e) dangling-binding messages.
const CLAIM_COMMENT_TOKEN: &str = "⊨ ";
/// The lowercased ADR status that fails a `decided_by` target (§2.6/§4.1).
const SUPERSEDED_STATUS: &str = "superseded";

/// The referential class (§4.1): (a) every claim anchor resolves; (b) every
/// referenced-or-declared landmark slug has slug-match cardinality exactly 1; (c)
/// every `decided_by` names an existing, non-superseded ADR, and each ADR id one file; (d) every `enforced_by`
/// names a VCS-tracked file; (e) every claim-glyph comment resolves to a declared
/// claim.
pub(super) fn referential(ctx: &Context) -> Result<Vec<Finding>> {
    let graph = ctx.graph;
    let mut findings = Vec::new();

    // The landmark slugs whose cardinality (b) must hold: those a claim anchor
    // REFERENCES plus those a landmark comment DECLARES (EXAMPLE 8.3b shows
    // both). A set keeps the union unique and ordered.
    let mut slugs: BTreeSet<String> = graph.anchors.landmarks.keys().cloned().collect();

    for node in &graph.nodes {
        for claim in node.invariants.iter().chain(node.hazards.iter()) {
            if let Some(slug) = claim.anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
                slugs.insert(slug.to_string());
            }

            // (a) the anchor resolves (unresolved = 0, ambiguous = >1; §2.4).
            match claim.resolution {
                Resolution::Unresolved => findings.push(anchor_unresolved(node, claim)),
                Resolution::Ambiguous => findings.push(anchor_ambiguous(node, claim)),
                Resolution::Pending | Resolution::Resolved => {}
            }

            // (d) the enforced_by artifact is a VCS-tracked file.
            if let Some(artifact) = &claim.enforced_by
                && !ctx.tracks(artifact)?
            {
                findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("enforced_by {artifact} is not a VCS-tracked file"),
                ));
            }
        }

        // (c) every decided_by resolves to an existing, non-superseded ADR. The
        // authored token is `§ <NNNN>` (§2.6) and the index id is path-derived
        // (`<adrdir>/<NNNN>`, §3.2), so the number is what matches — one ADR
        // directory is detected per repo, so the tail is unambiguous.
        for target in &node.edges.decided_by {
            let number = crate::model::decision_number(target).unwrap_or_default();
            match graph
                .adrs
                .iter()
                .find(|adr| adr.id.rsplit('/').next() == Some(number))
            {
                None => findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("decided_by target {target} resolves to no ADR in the index"),
                )),
                Some(adr) if adr.status == SUPERSEDED_STATUS => findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("decided_by target {target} names a superseded ADR"),
                )),
                Some(_) => {}
            }
        }
    }

    // (c) a referenced ADR id names one file: twins (parallel branches each adding
    // 0003) leave `§ 0003` ambiguous, and the lock keeps only one of them. Unreferenced
    // twins (log4brains' same-day `20240101-*`) are ambiguous to no one.
    let referenced: BTreeSet<&str> = graph
        .nodes
        .iter()
        .flat_map(|node| &node.edges.decided_by)
        .filter_map(|target| crate::model::decision_number(target).ok())
        .collect();
    let mut adr_files: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for adr in graph.adrs.iter().filter(|adr| {
        adr.id
            .rsplit('/')
            .next()
            .is_some_and(|n| referenced.contains(n))
    }) {
        adr_files.entry(&adr.id).or_default().push(&adr.path);
    }
    for (id, paths) in adr_files.iter().filter(|(_, paths)| paths.len() > 1) {
        findings.push(Finding::error(
            AssertionClass::Referential,
            None,
            format!(
                "ADR {id} is declared by {} files: {}",
                paths.len(),
                paths.join(", ")
            ),
        ));
    }

    // (b) slug-match cardinality exactly 1 (§2.5). A referenced slug with 0
    // occurrences is already reported by (a); only >1 is a cardinality finding.
    for slug in &slugs {
        let occurrences = graph
            .anchors
            .landmarks
            .get(slug)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if occurrences.len() > 1 {
            let mut finding = Finding::error(
                AssertionClass::Referential,
                None,
                format!(
                    "landmark {LANDMARK_ANCHOR_PREFIX}{slug} has slug-match cardinality {}",
                    occurrences.len()
                ),
            );
            for occ in occurrences {
                finding = finding.location(occ.file.clone(), occ.line, None);
            }
            findings.push(finding);
        }
    }

    // (e) every claim-glyph comment resolves to a declared claim.
    for anchor in &graph.anchors.claims {
        match graph.resolve_claim(&anchor.addr) {
            ClaimLookup::Found(_) => {}
            ClaimLookup::Ambiguous => findings.push(claim_comment_dangling(
                anchor,
                format!(
                    "{CLAIM_COMMENT_TOKEN}{} is an ambiguous abbreviation (matches multiple nodes)",
                    anchor.addr
                ),
            )),
            ClaimLookup::NotFound => findings.push(claim_comment_dangling(
                anchor,
                format!(
                    "{CLAIM_COMMENT_TOKEN}{} resolves to no declared claim",
                    anchor.addr
                ),
            )),
        }
    }

    Ok(findings)
}

/// A §4.1(a) unresolved-anchor finding (0 occurrences/definitions). The message form
/// matches the EXAMPLE 8.3a gallery for a landmark anchor; a `<path>#<symbol>` anchor gets the
/// analogous "0 definitions" phrasing.
fn anchor_unresolved(node: &Node, claim: &Claim) -> Finding {
    let message = match claim.anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
        Some(slug) => format!(
            "anchor {} unresolved (0 occurrences of \"{LANDMARK_ANCHOR_PREFIX}{slug}\")",
            claim.anchor
        ),
        None => match split_symbol(&claim.anchor) {
            // Never read (anchors::resolve_symbol), so no definition count to report.
            (path, _) if normalize_id(path).is_err() => format!(
                "anchor {} unresolved ({path} is not a repo-root-relative path, §2.4)",
                claim.anchor
            ),
            (path, symbol) => format!(
                "anchor {} unresolved (0 definitions of \"{symbol}\" in {path})",
                claim.anchor
            ),
        },
    };
    Finding::error(AssertionClass::Referential, Some(node.id.clone()), message)
        .detail(unanchored(claim))
}

/// A §4.1(a) ambiguous-anchor finding (>1 definitions; only `<path>#<symbol>` anchors
/// can be ambiguous, §2.4).
fn anchor_ambiguous(node: &Node, claim: &Claim) -> Finding {
    let (path, symbol) = split_symbol(&claim.anchor);
    Finding::error(
        AssertionClass::Referential,
        Some(node.id.clone()),
        format!(
            "anchor {} ambiguous (>1 definitions of \"{symbol}\" in {path})",
            claim.anchor
        ),
    )
    .detail(unanchored(claim))
}

/// A §4.1(e) dangling claim-comment finding, anchored at the comment's `file:line`.
fn claim_comment_dangling(anchor: &ClaimAnchor, message: String) -> Finding {
    Finding::error(AssertionClass::Referential, None, message).location(
        anchor.file.clone(),
        anchor.line,
        None,
    )
}

/// The "claim … is now unanchored" detail line (EXAMPLE 8.3a).
fn unanchored(claim: &Claim) -> String {
    format!(
        "claim \"{}\" is now unanchored — provenance broken",
        crate::emit::flat(&claim.text)
    )
}

/// Split a `<path>#<symbol>` anchor into its parts for §4.1(a) messages; a `#`-free
/// anchor (never produced for this arm) degrades to an empty symbol.
fn split_symbol(anchor: &str) -> (&str, &str) {
    anchor.rsplit_once('#').unwrap_or((anchor, ""))
}
