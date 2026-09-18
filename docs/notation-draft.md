# Notation draft — footnote-style ASCII markers

Premise: every stele token today is a word (`stele:landmark`, `stele:claim`, `lm:`, `stele:begin router`).
Replace them with one bracket grammar borrowed from markdown footnotes: `[^id]` references, `[^id]:` defines.
Greppability survives because `[^` is rare in code and the markdown scan already ignores prose (HTML comments only).

## A · one sigil, disambiguated by shape (the lean)

```elixir
# [^refund-cap]                      landmark — declares a stable address (slug, no slash)
def changeset(refund, attrs) do

# [^billing/refund-cap]              claim binding — back-reference to a declared claim (has a slash)
defp cap(amount, charge) do
```

```yaml
invariants:
  - claim: a refund never exceeds its original charge
    anchor: ^refund-cap              # was lm:refund-cap  — caret is the landmark namespace
  - claim: money is integer cents
    anchor: money.ts#MoneyType       # unchanged — `#` already means symbol
edges:
  decided_by: [^adr/0007]            # was adr/0007 — same footnote shape, resolves to adr/0007-*.md
```

```markdown
<!-- [^router] -->                   was <!-- stele:begin router -->
…generated…
<!-- [^router]: -->                  was <!-- stele:end -->   (definition-colon closes; or keep [/router])
```

Rule of shape: a footnote id with no `/` is a landmark, with a `/` is a claim address, with an `adr/` head is a decision.
Root-node claims are `[^/refund-cap]` (system id is `/`, so the slash is still there).

## B · sigil per concept (the alternative pole)

```elixir
# ~refund-cap                        landmark  (~ = "here")
# =billing/refund-cap                claim     (= = "this realizes")
```
```yaml
    anchor: ~refund-cap
    decided_by: [~adr/0007]
```
```markdown
<!-- ~router -->  …  <!-- /router -->
```
Fewer bytes, but `~` and `=` collide with real code in comments (`# = 5` style notes, `~` in Ruby/Perl docs), so B needs a fence like `#~` or a trailing `:`, at which point A is simpler.

## What stays fixed either way

slug lexeme `[a-z0-9]+(-[a-z0-9]+)*` · cardinality 1 · `<path>#<symbol>` · node ids · ADR `NNNN-*.md` · the `stele` fenced block · file names.

## Migration shape (when you pick)

`stele build` accepts both grammars for one minor version (old tokens warn), `stele emit --notation` rewrites comments and blocks in place, SPEC §2.5/§3.1 rewritten, lock version bumps.

## Option field · 50 grammars

Columns: the landmark comment body, the claim-binding comment body, the `anchor:` value, the region open/close. `decided_by` takes the landmark wrapper around `adr/0007` in every row. Slug lexeme, `path#symbol` and cardinality are constant.

| #  | family            | landmark              | claim                       | anchor:            | region open / close                | trap                        |
| -- | ----------------- | --------------------- | --------------------------- | ------------------ | ---------------------------------- | --------------------------- |
| 1  | footnote          | `[^refund-cap]`       | `[^billing/refund-cap]`     | `^refund-cap`      | `[^router]` / `[^router]:`         | none known                  |
| 2  | footnote, def-colon | `[^refund-cap]`     | `[^billing/refund-cap]:`    | `^refund-cap`      | `[^router]` / `[^/router]`         | colon easy to drop          |
| 3  | footnote + wiki   | `[^refund-cap]`       | `[[billing/refund-cap]]`    | `^refund-cap`      | `[[router]]` / `[[/router]]`       | two shapes to learn         |
| 4  | wiki link         | `[[refund-cap]]`      | `[[billing/refund-cap]]`    | `[[refund-cap]]`   | `[[router]]` / `[[/router]]`       | Obsidian prose collides     |
| 5  | wiki, kinded      | `[[lm:refund-cap]]`   | `[[claim:billing/refund-cap]]` | `lm:refund-cap` | `[[router]]` / `[[/router]]`       | verbose                     |
| 6  | link ref          | `[refund-cap]`        | `[billing/refund-cap]`      | `[refund-cap]`     | `[router]` / `[/router]`           | any bracketed word          |
| 7  | link ref, def     | `[refund-cap]:`       | `[billing/refund-cap]:`     | `[refund-cap]`     | `[router]:` / `[/router]:`         | md link defs in comments    |
| 8  | roam block        | `((refund-cap))`      | `((billing/refund-cap))`    | `((refund-cap))`   | `((router))` / `((/router))`       | Lisp comments               |
| 9  | mustache          | `{{refund-cap}}`      | `{{billing/refund-cap}}`    | `{{refund-cap}}`   | `{{router}}` / `{{/router}}`       | template files              |
| 10 | braces            | `{refund-cap}`        | `{billing/refund-cap}`      | `{refund-cap}`     | `{router}` / `{/router}`           | every C-family comment      |
| 11 | brace caret       | `{^refund-cap}`       | `{^billing/refund-cap}`     | `^refund-cap`      | `{^router}` / `{^/router}`         | none known                  |
| 12 | xml tag           | `<refund-cap/>`       | `<billing/refund-cap/>`     | `<refund-cap>`     | `<router>` / `</router>`           | HTML/JSX comments           |
| 13 | angle             | `<refund-cap>`        | `<billing/refund-cap>`      | `<refund-cap>`     | `<router>` / `</router>`           | generics in comments        |
| 14 | angle caret       | `<^refund-cap>`       | `<^billing/refund-cap>`     | `^refund-cap`      | `<^router>` / `<^/router>`         | none known                  |
| 15 | arrows            | `<- refund-cap`       | `-> billing/refund-cap`     | `<-refund-cap`     | `-> router` / `<- router`          | prose arrows                |
| 16 | fat arrows        | `=> refund-cap`       | `<= billing/refund-cap`     | `=>refund-cap`     | `=> router` / `<= router`          | comparison in prose         |
| 17 | yaml anchor       | `&refund-cap`         | `*billing/refund-cap`       | `*refund-cap`      | `&router` / `*router`              | `*` bullet lists            |
| 18 | yaml anchor, wrapped | `[&refund-cap]`    | `[*billing/refund-cap]`     | `*refund-cap`      | `[&router]` / `[*router]`          | none known                  |
| 19 | caret bare        | `^refund-cap`         | `^billing/refund-cap`       | `^refund-cap`      | `^router` / `^/router`             | regex in comments           |
| 20 | tilde             | `~refund-cap`         | `~billing/refund-cap`       | `~refund-cap`      | `~router` / `~/router`             | Ruby/Perl docs, `~/` paths  |
| 21 | tilde + equals    | `~refund-cap`         | `=billing/refund-cap`       | `~refund-cap`      | `~router` / `~/router`             | `# = 5`-style notes         |
| 22 | at                | `@refund-cap`         | `@billing/refund-cap`       | `@refund-cap`      | `@router` / `@/router`             | JSDoc, decorators           |
| 23 | at, javadoc verbs | `@lm refund-cap`      | `@claim billing/refund-cap` | `lm:refund-cap`    | `@region router` / `@end`          | verbose, but idiomatic      |
| 24 | double at         | `@@refund-cap`        | `@@billing/refund-cap`      | `@@refund-cap`     | `@@router` / `@@/router`           | Ruby class vars             |
| 25 | hash              | `#refund-cap`         | `#billing/refund-cap`       | `#refund-cap`      | `#router` / `#/router`             | `#` comment langs, `#` symbol |
| 26 | double hash       | `##refund-cap`        | `##billing/refund-cap`      | `##refund-cap`     | `##router` / `##/router`           | md headings in comments     |
| 27 | css               | `.refund-cap`         | `/billing/refund-cap`       | `.refund-cap`      | `.router` / `/router`              | file paths, ellipses        |
| 28 | dollar            | `$refund-cap`         | `$billing/refund-cap`       | `$refund-cap`      | `$router` / `$/router`             | shell vars                  |
| 29 | percent           | `%refund-cap`         | `%billing/refund-cap`       | `%refund-cap`      | `%router` / `%/router`             | format strings              |
| 30 | bang              | `!refund-cap`         | `!billing/refund-cap`       | `!refund-cap`      | `!router` / `!/router`             | negation, shebangs          |
| 31 | question          | `?refund-cap`         | `?billing/refund-cap`       | `?refund-cap`      | `?router` / `?/router`             | ternaries                   |
| 32 | ampersand         | `&refund-cap`         | `&billing/refund-cap`       | `&refund-cap`      | `&router` / `&/router`             | references in C/Rust        |
| 33 | pipe fence        | `\|refund-cap\|`      | `\|billing/refund-cap\|`    | `\|refund-cap\|`   | `\|router\|` / `\|/router\|`       | tables, absolute value      |
| 34 | double colon      | `::refund-cap`        | `::billing/refund-cap`      | `::refund-cap`     | `::router` / `::/router`           | C++/Rust paths              |
| 35 | label             | `refund-cap:`         | `billing/refund-cap:`       | `refund-cap`       | `router:` / `/router:`             | any word-colon in prose     |
| 36 | double label      | `refund-cap::`        | `billing/refund-cap::`      | `refund-cap`       | `router::` / `/router::`           | rare, but reads as C++      |
| 37 | trailing caret    | `refund-cap^`         | `billing/refund-cap^`       | `refund-cap^`      | `router^` / `/router^`             | none known, odd to read     |
| 38 | backtick          | `` `refund-cap` ``    | `` `billing/refund-cap` ``  | `refund-cap`       | `` `router` `` / `` `/router` ``   | every code span in comments |
| 39 | star emphasis     | `*refund-cap*`        | `*billing/refund-cap*`      | `refund-cap`       | `*router*` / `*/router*`           | md emphasis, `*/` closes C  |
| 40 | short verbs       | `lm refund-cap`       | `cl billing/refund-cap`     | `lm:refund-cap`    | `rg router` / `rg end`             | not greppable               |
| 41 | stele short       | `st:refund-cap`       | `st:billing/refund-cap`     | `st:refund-cap`    | `st:router` / `st:/router`         | none known                  |
| 42 | stele verbs (now) | `stele:landmark refund-cap` | `stele:claim billing/refund-cap` | `lm:refund-cap` | `stele:begin router` / `stele:end` | wordy                     |
| 43 | stele terse       | `stele:refund-cap`    | `stele:billing/refund-cap`  | `refund-cap`       | `stele:router` / `stele:/router`   | none known                  |
| 44 | s/c letters       | `s:refund-cap`        | `c:billing/refund-cap`      | `s:refund-cap`     | `r:router` / `r:end`               | Windows drive letters       |
| 45 | bracket verbs     | `[lm refund-cap]`     | `[claim billing/refund-cap]` | `lm:refund-cap`   | `[router]` / `[/router]`           | verbose                     |
| 46 | single vs double  | `[refund-cap]`        | `[[billing/refund-cap]]`    | `[refund-cap]`     | `[[router]]` / `[[/router]]`       | bracket depth is the kind   |
| 47 | paren caret       | `(^refund-cap)`       | `(^billing/refund-cap)`     | `^refund-cap`      | `(^router)` / `(^/router)`         | none known                  |
| 48 | at-wrapped        | `@refund-cap@`        | `@billing/refund-cap@`      | `refund-cap`       | `@router@` / `@/router@`           | none known, noisy           |
| 49 | caret-wrapped     | `^refund-cap^`        | `^billing/refund-cap^`      | `refund-cap`       | `^router^` / `^/router^`           | none known, noisy           |
| 50 | dashes            | `-- refund-cap --`    | `-- billing/refund-cap --`  | `refund-cap`       | `-- router --` / `-- /router --`   | SQL/Lua comments            |

Reading the field: rows 1, 11, 14, 18, 41, 43, 47 have no known collision; of those, 1 is the only one whose shape already exists in a spec people know. Rows 6, 10, 12, 13, 25 collide with ordinary comment text and would need cardinality errors to catch false landmarks.

## Glyph grammar (non-ASCII)

Zero collision, one glyph per concept, no fence. Costs: typing (snippet), not self-naming (root AGENTS.md teaches it), ASCII-only hooks downstream. `✻` stays the human-notes glyph.

| slot       | glyph | why                                  | written                                    | alternates      |
| ---------- | ----- | ------------------------------------ | ------------------------------------------ | --------------- |
| landmark   | ※     | reference mark, "note this spot"     | `// ※ refund-cap`                          | ⌖ ◎             |
| claim      | ⊢     | turnstile, "this code asserts it"    | `// ⊢ billing/refund-cap`                  | ‡ ⊨             |
| anchor:    | ※     | same glyph as the comment it targets | `anchor: ※refund-cap`                      |                 |
| decided_by | §     | section sign, "per the record"       | `decided_by: [§0007]`                      |                 |
| region     | ⟦ ⟧   | semantic brackets, engine-owned      | `<!-- ⟦router⟧ --> … <!-- ⟦/router⟧ -->`   | ⌈ ⌉ ⌊ ⌋ · « »   |

`※ : refund-cap` → `※ refund-cap`: the glyph is the sigil, the colon is noise.

## Glyph option field · 40 grammars

Same columns as the ASCII field. `anchor:` takes the landmark glyph, no space. `decided_by` shows its own wrapper. `✻` is excluded everywhere (human notes).

| #  | family              | landmark        | claim                   | anchor:          | decided_by   | region open / close        | note                              |
| -- | ------------------- | --------------- | ----------------------- | ---------------- | ------------ | -------------------------- | --------------------------------- |
| 1  | reference + turnstile | `※ refund-cap` | `⊢ billing/refund-cap` | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | the lean                          |
| 2  | reference only      | `※ refund-cap`  | `※ billing/refund-cap`  | `※refund-cap`    | `※adr/0007`  | `※router` / `※/router`     | one glyph, slash rule decides     |
| 3  | reference + dagger  | `※ refund-cap`  | `‡ billing/refund-cap`  | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | footnote ladder: ※ then ‡         |
| 4  | dagger ladder       | `† refund-cap`  | `‡ billing/refund-cap`  | `†refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | classic footnote order            |
| 5  | reference + models  | `※ refund-cap`  | `⊨ billing/refund-cap`  | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ⊨ "code satisfies the claim"      |
| 6  | reference + therefore | `※ refund-cap` | `∴ billing/refund-cap` | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ∴ reads as consequence            |
| 7  | position + turnstile | `⌖ refund-cap` | `⊢ billing/refund-cap`  | `⌖refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ⌖ is literally "position"         |
| 8  | position only       | `⌖ refund-cap`  | `⌖ billing/refund-cap`  | `⌖refund-cap`    | `⌖adr/0007`  | `⌖router` / `⌖/router`     | one glyph                         |
| 9  | bullseye + turnstile | `◎ refund-cap` | `⊢ billing/refund-cap`  | `◎refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ◎ renders wide in some fonts      |
| 10 | target + check      | `⊙ refund-cap`  | `✓ billing/refund-cap`  | `⊙refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ✓ risks emoji fallback            |
| 11 | pilcrow + section   | `¶ refund-cap`  | `§ billing/refund-cap`  | `¶refund-cap`    | `§adr/0007`  | `⟦router⟧` / `⟦/router⟧`   | § doubles as claim and adr        |
| 12 | section + pilcrow   | `§ refund-cap`  | `¶ billing/refund-cap`  | `§refund-cap`    | `§adr/0007`  | `⟦router⟧` / `⟦/router⟧`   | § collides with adr               |
| 13 | asterism            | `⁂ refund-cap`  | `⊢ billing/refund-cap`  | `⁂refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ⁂ is a break mark, wide           |
| 14 | lozenge             | `◊ refund-cap`  | `⊢ billing/refund-cap`  | `◊refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ◊ is neutral, no meaning          |
| 15 | diamond ladder      | `◇ refund-cap`  | `◆ billing/refund-cap`  | `◇refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | hollow declares, solid binds      |
| 16 | circle ladder       | `○ refund-cap`  | `● billing/refund-cap`  | `○refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | same idea, rounder                |
| 17 | square ladder       | `□ refund-cap`  | `■ billing/refund-cap`  | `□refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | checkbox connotation              |
| 18 | triangle            | `▸ refund-cap`  | `◂ billing/refund-cap`  | `▸refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | direction: out to code, back to doc |
| 19 | arrows              | `→ refund-cap`  | `← billing/refund-cap`  | `→refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | arrows appear in prose            |
| 20 | double arrows       | `⇒ refund-cap`  | `⇐ billing/refund-cap`  | `⇒refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | logic-flavored                    |
| 21 | maps-to             | `↦ refund-cap`  | `⊢ billing/refund-cap`  | `↦refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ↦ "maps here"                     |
| 22 | anchor glyph        | `⚓ refund-cap` | `⊢ billing/refund-cap`  | `⚓refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ⚓ is emoji-presentation, rejected |
| 23 | flag                | `⚑ refund-cap`  | `⊢ billing/refund-cap`  | `⚑refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ⚑ emoji fallback on some systems  |
| 24 | place of interest   | `⌘ refund-cap`  | `⊢ billing/refund-cap`  | `⌘refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | reads as the Mac key              |
| 25 | viewdata square     | `⌗ refund-cap`  | `⊢ billing/refund-cap`  | `⌗refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | a hash that is not `#`            |
| 26 | angle quotes        | `« refund-cap »` | `‹ billing/refund-cap ›` | `«refund-cap»` | `§0007`      | `«router»` / `«/router»`   | prose quotes in some locales      |
| 27 | math brackets       | `⟨refund-cap⟩`  | `⟦billing/refund-cap⟧`  | `⟨refund-cap⟩`   | `§0007`      | `⟪router⟫` / `⟪/router⟫`   | bracket weight is the kind        |
| 28 | corner brackets     | `「refund-cap」` | `『billing/refund-cap』` | `「refund-cap」` | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | CJK width, wide in mono           |
| 29 | ceiling / floor     | `⌈refund-cap⌉`  | `⌊billing/refund-cap⌋`  | `⌈refund-cap⌉`   | `§0007`      | `⌈router⌉` / `⌊router⌋`    | region reads open/close naturally |
| 30 | tortoise shell      | `〔refund-cap〕` | `⊢ billing/refund-cap`  | `〔refund-cap〕`  | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | CJK width                         |
| 31 | reference, wrapped  | `⟨※refund-cap⟩` | `⟨⊢billing/refund-cap⟩` | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | fence is redundant with a glyph   |
| 32 | reference + colon   | `※: refund-cap` | `⊢: billing/refund-cap` | `※refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | your sample; colon is noise       |
| 33 | reference trailing  | `refund-cap ※`  | `billing/refund-cap ⊢`  | `refund-cap※`    | `0007§`      | `router⟧` / `/router⟧`     | postfix, odd to grep              |
| 34 | interrobang         | `‽ refund-cap`  | `⊢ billing/refund-cap`  | `‽refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | reads as a question               |
| 35 | tie / dotted        | `⁀ refund-cap`  | `⁝ billing/refund-cap`  | `⁀refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | too faint at small sizes          |
| 36 | box drawing         | `├ refund-cap`  | `┤ billing/refund-cap`  | `├refund-cap`    | `§0007`      | `┌router┐` / `└router┘`    | collides with tree diagrams       |
| 37 | greek               | `λ refund-cap`  | `Φ billing/refund-cap`  | `λrefund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | identifiers in math code          |
| 38 | currency-free       | `¤ refund-cap`  | `⊢ billing/refund-cap`  | `¤refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | ¤ is the "no currency" glyph      |
| 39 | degree              | `° refund-cap`  | `⊢ billing/refund-cap`  | `°refund-cap`    | `§0007`      | `⟦router⟧` / `⟦/router⟧`   | tiny, collides with units         |
| 40 | full ladder         | `※ refund-cap`  | `‡ billing/refund-cap`  | `※refund-cap`    | `§0007`      | `¶router` / `¶/router`     | all four from typography          |

Reading the field: 1, 3, 5, 7, 15, 29, 40 are clean and distinguishable at mono width. 10, 22, 23 fail the no-emoji rule on fallback fonts. 26, 28, 30 are double-width in monospace. 2 and 8 are the one-glyph purists, paying with the slash rule.

## Most accurate glyph per slot

| slot       | glyph | Unicode name                      | why                                                                    | runner-up |
| ---------- | ----- | --------------------------------- | ---------------------------------------------------------------------- | --------- |
| landmark   | ⌖     | POSITION INDICATOR                | a landmark is a named position in code; the glyph's literal name       | ※         |
| claim      | ⊨     | TRUE (models)                     | the region satisfies the claim; ⊢ "proves" is `enforced_by`'s job      | ⊢         |
| anchor:    | ⌖     | same as landmark                  | the field names the landmark, so it carries its glyph                  | →         |
| decided_by | №     | NUMERO SIGN                       | an ADR is a numbered record; § is a section of text                    | §         |
| region     | ⟦ ⟧   | MATHEMATICAL WHITE SQUARE BRACKETS | denotation brackets: the rendering computed from the graph            | ⌈ ⌉ ⌊ ⌋   |

```
# ⌖ refund-cap            # ⊨ billing/refund-cap
anchor: ⌖refund-cap       decided_by: [№0007]
<!-- ⟦router⟧ --> … <!-- ⟦/router⟧ -->
```

Caveats: ⌖ is the rarest in mono fonts (JetBrains Mono, Cascadia, SF Mono carry it). ※ wins if the notation should read as footnotes, at the cost of naming the note rather than the place.
