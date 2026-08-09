//! Optional text → facts + entities extraction, the layer that makes the graph
//! self-build.
//!
//! The Agent Memory SDK is *bring-your-own-links*: [`crate::MemoryService::remember`]
//! only stores the links the caller supplies, so a graph is only ever as rich as
//! what the caller wires by hand. This module adds the missing commodity on top:
//! an [`Extractor`] turns a paragraph of raw text into atomic facts, each tagged
//! with the salient topics it mentions. [`crate::MemoryService::remember_extracted`]
//! then stores those facts and wires the fact↔entity graph automatically, so
//! `why()` has something to traverse without any manual `relate()`.
//!
//! Mirroring the [`crate::embedder`] pattern, the plug-point is dependency-free
//! (bring your own LLM by implementing [`Extractor`]) while a batteries-included
//! `OllamaExtractor` backend lives behind the `extract` feature.

/// One extracted, graph-ready fact: a self-contained sentence plus the salient
/// topics it concerns. The topics become shared graph hubs, so two facts about
/// the same topic are reachable from one another even with no textual overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFact {
    /// The atomic, standalone fact (pronouns resolved, dates absolute).
    pub text: String,
    /// Salient topics the fact concerns — short canonical lowercase noun
    /// phrases (e.g. `"adoption"`, `"charity race"`). 1-4 is typical.
    pub entities: Vec<String>,
}

/// One extracted entity→entity edge: `subject -[predicate]-> object`.
///
/// Where [`ExtractedFact::entities`] only says "this fact concerns these
/// topics", a relation says *how two topics relate*. It is what turns the
/// bipartite fact↔topic graph into a genuine knowledge graph: from
/// "Bruno Durand is Theo Durand's father" the wiring produces the edge
/// `bruno durand -[father of]-> theo durand`, so a later walk can answer
/// "who is Theo's father" without any fact mentioning both names again.
///
/// `subject` and `object` are canonicalized exactly like
/// [`ExtractedFact::entities`] (trimmed, lowercased), so they resolve to the
/// SAME entity hub as the topics — the hub id is content-addressed, so this
/// holds across separate calls and across sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedRelation {
    /// Canonical lowercase entity the edge points *from*.
    pub subject: String,
    /// The edge label (e.g. `"father of"`, `"sister of"`, `"works at"`).
    pub predicate: String,
    /// Canonical lowercase entity the edge points *to*.
    pub object: String,
}

/// One extracted entity attribute: `entity.key = value`.
///
/// Attributes are what make "Theo Durand is 15" answerable by a *filter*
/// rather than a similarity search: the pair is merged into the entity hub's
/// `ColumnStore` metadata, so `recall_where` can select on `age >= 15`.
///
/// `value` deliberately keeps its JSON type. `recall_where` comparisons are
/// TYPE-STRICT with no coercion, so an age extracted as the string `"15"`
/// would silently never match a numeric filter — the extraction contract
/// therefore demands numbers stay numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedAttribute {
    /// Canonical lowercase entity the attribute belongs to.
    pub entity: String,
    /// Attribute name, used verbatim as the metadata (`ColumnStore`) field.
    pub key: String,
    /// Attribute value, type preserved (a number stays a JSON number).
    pub value: serde_json::Value,
}

/// Everything one passage yields: the atomic facts, the entity→entity edges
/// between the topics they mention, and the attributes those entities carry.
///
/// [`Extractor::extract`] returns only the `facts` half; a backend that can
/// also read relations and attributes overrides [`Extractor::extract_graph`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extraction {
    /// The atomic, standalone facts, each with the topics it concerns.
    pub facts: Vec<ExtractedFact>,
    /// Typed edges between entities mentioned in the passage.
    pub relations: Vec<ExtractedRelation>,
    /// Attributes attached to entities mentioned in the passage.
    pub attributes: Vec<ExtractedAttribute>,
}

// --- Orienting a kinship triple, whichever form states it ---------------------
//
// A copule ("X est le pere de Y") hangs the relation on its own grammatical
// subject, so subject-of-sentence and subject-of-triple coincide. Three other
// forms hang it on the OTHER one, and a model mirrors all three:
//
//   possessive  "X a une soeur, Y"        → Y carries "soeur de", toward X
//   genitive    "la soeur de X est Y"     → the same reading, reversed order
//               "X's sister is Y"
//   plural      "X a deux soeurs, Y et Z" → the same, with SEVERAL carriers
//
// A mirrored kinship triple is worse than a missing edge — `entity("Camille
// Durand")` then answers, with the same confidence as any true edge, that
// Camille is Theo's *brother*. The prompt asks for the right direction; this
// pass guarantees it for the constructions that get it wrong, whatever backend
// produced the triple.
//
// Two invariants make the pass safe to run over a backend that may be
// hallucinating, and every rule below is bounded so as to keep them: it never
// invents an edge and never drops one. It only ever re-points triples the
// extractor already returned, between endpoints it already named.

/// Kinship nouns a passage can hang a relation on, folded (no accents, no
/// ligature) and singular — every matcher below tolerates a plural `s`. Doubling
/// as the
/// predicate whitelist: a triple is only ever re-pointed when its label is one
/// of these, so a non-kinship edge between the same two people is left alone.
///
/// Alliances (`"beau-frere"`, `"godmother"`, `"petit-fils"`) sit here beside
/// the blood ties because they behave identically: grammatically they are the
/// same possessive, and the converse of one is simply whichever OTHER label of
/// this table the extractor put on the same pair — see [`reorient`]. Listing
/// the noun is therefore the whole of the work; nothing below special-cases it.
///
/// Deliberately NOT here: `"partner"` / `"compagnon"`. "X has a partner, Y" is
/// a business relation as often as a family one, and a wrong re-point is worse
/// than none at all.
/// Every noun is paired with the CANONICAL noun of the relation it denotes.
///
/// The orientation pass decides direction by asking whether the predicate names
/// the same kinship the passage named. Asking that of the SPELLING made two
/// words for one relation read as each other's converse, and stored the edge
/// backwards (#1754) — across languages (`"sister"` vs `"soeur"`), and just as
/// much within one (`"mari"` vs `"epoux"`, `"gendre"` vs `"beau-fils"`, both
/// listed here). Comparing canonical forms asks it of the MEANING instead, so a
/// synonym orients like the word it stands for and only a real converse flips.
///
/// Where one French noun covers two English ones — `"beau-pere"` is both the
/// father-in-law and the stepfather — every spelling folds onto that single
/// canonical. Deliberate: the pass has no "unrelated" branch, so anything not
/// judged identical is treated as the converse. Reading two step/in-law words
/// as ONE relation leaves an edge unturned; reading them as converses points it
/// the wrong way, and this pass already holds that a missing correction beats a
/// wrong one.
const KINSHIP_NOUNS: &[(&str, &str)] = &[
    // Blood ties, fr. — each its own canonical.
    ("pere", "pere"),
    ("mere", "mere"),
    ("frere", "frere"),
    ("soeur", "soeur"),
    ("fils", "fils"),
    ("fille", "fille"),
    ("oncle", "oncle"),
    ("tante", "tante"),
    ("cousin", "cousin"),
    ("cousine", "cousine"),
    ("neveu", "neveu"),
    ("niece", "niece"),
    ("grand-pere", "grand-pere"),
    ("grand-mere", "grand-mere"),
    ("grand-oncle", "grand-oncle"),
    ("grand-tante", "grand-tante"),
    ("arriere-grand-pere", "arriere-grand-pere"),
    ("arriere-grand-mere", "arriere-grand-mere"),
    ("petit-fils", "petit-fils"),
    ("petite-fille", "petite-fille"),
    // Alliances and step-family, fr. — `gendre`/`bru` and `mari`/`femme` are
    // synonyms of the nouns they fold onto, not converses of them.
    ("beau-pere", "beau-pere"),
    ("belle-mere", "belle-mere"),
    ("beau-frere", "beau-frere"),
    ("belle-soeur", "belle-soeur"),
    ("beau-fils", "beau-fils"),
    ("belle-fille", "belle-fille"),
    ("gendre", "beau-fils"),
    ("bru", "belle-fille"),
    ("demi-frere", "demi-frere"),
    ("demi-soeur", "demi-soeur"),
    ("parrain", "parrain"),
    ("marraine", "marraine"),
    ("filleul", "filleul"),
    ("filleule", "filleule"),
    ("epoux", "epoux"),
    ("epouse", "epouse"),
    ("mari", "epoux"),
    ("femme", "epouse"),
    // Blood ties, en. — folded onto their French twin.
    ("father", "pere"),
    ("mother", "mere"),
    ("brother", "frere"),
    ("sister", "soeur"),
    ("son", "fils"),
    ("daughter", "fille"),
    ("uncle", "oncle"),
    ("aunt", "tante"),
    ("nephew", "neveu"),
    ("grandfather", "grand-pere"),
    ("grandmother", "grand-mere"),
    ("grandson", "petit-fils"),
    ("granddaughter", "petite-fille"),
    // Alliances and step-family, en. — folded onto their French twin.
    ("husband", "epoux"),
    ("wife", "epouse"),
    ("father-in-law", "beau-pere"),
    ("mother-in-law", "belle-mere"),
    ("brother-in-law", "beau-frere"),
    ("sister-in-law", "belle-soeur"),
    ("son-in-law", "beau-fils"),
    ("daughter-in-law", "belle-fille"),
    ("stepfather", "beau-pere"),
    ("stepmother", "belle-mere"),
    ("stepbrother", "beau-frere"),
    ("stepsister", "belle-soeur"),
    ("half-brother", "demi-frere"),
    ("half-sister", "demi-soeur"),
    ("godfather", "parrain"),
    ("godmother", "marraine"),
    ("godson", "filleul"),
    ("goddaughter", "filleule"),
];

/// What precedes the kinship noun when the sentence hangs the relation on the
/// person it introduces rather than on its own subject. The trailing space is
/// load-bearing: without it `" a un "` would also fire on `"a une"`.
///
/// The counting determiners are what make a plural construction readable at
/// all: `"a deux soeurs, Camille et Lea"` matches no singular marker.
const POSSESSIVE_MARKERS: &[&str] = &[
    " a un ",
    " a une ",
    " a pour ",
    " a des ",
    " a deux ",
    " a trois ",
    " a quatre ",
    " has a ",
    " has an ",
    " has two ",
    " has three ",
    " has four ",
];

/// What sits between a kinship noun and the name of whoever HOLDS it in a
/// genitive: `"la soeur DE Theo"`, `"the sister OF Theo"`.
const GENITIVE_LINKS: &[&str] = &[" de ", " d'", " of "];

/// The clitic the English genitive marks its holder with, holder first:
/// `"Theo's sister is Camille"`.
const SAXON_MARKER: &str = "'s ";

/// The copula that closes a genitive and introduces its carrier:
/// `"la soeur de Theo EST Camille"`.
const GENITIVE_COPULAS: &[&str] = &[" est ", " sont ", " is ", " are "];

/// Articles a copula may put in front of the name it introduces. Stepping over
/// one is what lets the carrier still be *required* to sit right after the
/// copula, which is the whole of the genitive's safety.
const LEADING_ARTICLES: &[&str] = &["le ", "la ", "les ", "l'", "the "];

/// What may join two carriers of one construction: `"Camille Durand ET Lea
/// Durand"`. Longest first, so `", et "` is never read as `", "` followed by
/// something that is not a name — which would end the walk one carrier early.
const ENUMERATION_SEPARATORS: &[&str] = &[", et ", ", and ", " et ", " and ", " & ", ", "];

/// Diacritics and ligatures folded to ASCII, so `"sœur"`, `"soeur"` and
/// `"Sœur"` are one token — the passage and the model's label rarely agree on
/// accents, and the whole pass hinges on matching one against the other.
const FOLDINGS: &[(char, &str)] = &[
    ('à', "a"),
    ('â', "a"),
    ('ä', "a"),
    ('é', "e"),
    ('è', "e"),
    ('ê', "e"),
    ('ë', "e"),
    ('î', "i"),
    ('ï', "i"),
    ('ô', "o"),
    ('ö', "o"),
    ('ù', "u"),
    ('û', "u"),
    ('ü', "u"),
    ('ç', "c"),
    ('œ', "oe"),
    ('æ', "ae"),
    // A typographic apostrophe, so `"Theo’s"` and `"d’Theo"` reach the same
    // matchers as their ASCII spellings — most editors substitute it silently.
    ('\u{2019}', "'"),
];

/// Lowercase `text` and fold its diacritics away. Every offset produced from
/// the result indexes the *folded* string, never the original.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        match FOLDINGS.iter().find(|(from, _)| *from == ch) {
            Some((_, to)) => folded.push_str(to),
            None => folded.push(ch),
        }
    }
    folded
}

/// A kinship relation the passage states: every one of `bearers` carries
/// `noun`, and `holder` is the one they carry it toward.
struct Kinship {
    noun: &'static str,
    holder: String,
    bearers: Vec<String>,
}

/// How many bytes `word` occupies at the start of `rest` when it is written
/// there as a whole word, a plural `s` included. `None` when it is not.
///
/// `"soeurette"` therefore never reads as `"soeur"`, and — the case that
/// matters — `"brother-in-law"` never reads as `"brother"`: a hyphen CONTINUES
/// a compound noun, so it bars the match exactly like a letter. Without that,
/// the pass would recognise the bare noun, then treat the sentence's own label
/// as the *converse* of it and point the edge precisely the wrong way. A
/// missing edge would have been the better outcome.
fn word_prefix_len(rest: &str, word: &str) -> Option<usize> {
    let tail = rest.strip_prefix(word)?;
    let (tail, plural) = match tail.strip_prefix('s') {
        Some(shorter) => (shorter, 1),
        None => (tail, 0),
    };
    let glued = |ch: char| ch.is_alphanumeric() || ch == '-';
    (!tail.starts_with(glued)).then_some(word.len() + plural)
}

/// Whether `head` ENDS on `word` as a whole word, a plural `s` included — the
/// mirror of [`word_prefix_len`], for the genitive, where the noun precedes its
/// link instead of following a marker.
fn ends_with_word(head: &str, word: &str) -> bool {
    ends_exactly(head, word)
        || head
            .strip_suffix('s')
            .is_some_and(|singular| ends_exactly(singular, word))
}

/// `head` ends on `word` with no letter and no hyphen glued in front of it, so
/// `"la belle-soeur"` is never read as ending on `"soeur"`.
fn ends_exactly(head: &str, word: &str) -> bool {
    head.strip_suffix(word)
        .is_some_and(|lead| !lead.ends_with(|ch: char| ch.is_alphanumeric() || ch == '-'))
}

/// The kinship noun written at the start of `text`, and how many bytes it
/// occupies there.
/// The length is that of the spelling actually written; the noun returned is
/// its CANONICAL form, so a caller compares meanings and never spellings.
fn noun_at(text: &str) -> Option<(&'static str, usize)> {
    KINSHIP_NOUNS.iter().find_map(|(spelling, canonical)| {
        word_prefix_len(text, spelling).map(|len| (*canonical, len))
    })
}

/// The kinship noun `head` ends on, in its canonical form.
fn noun_before(head: &str) -> Option<&'static str> {
    KINSHIP_NOUNS
        .iter()
        .find(|(spelling, _)| ends_with_word(head, spelling))
        .map(|(_, canonical)| *canonical)
}

/// The text left once the first of `prefixes` that `text` starts with is
/// stepped over.
fn strip_any<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| text.strip_prefix(prefix))
}

/// Every distinct entity the triples name, deduplicated.
fn endpoint_names(relations: &[ExtractedRelation]) -> Vec<String> {
    let mut names: Vec<String> = relations
        .iter()
        .flat_map(|relation| [relation.subject.clone(), relation.object.clone()])
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The endpoint named closest to the left of the noun: the person who HAS the
/// relative.
fn holder_of(before: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| before.rfind(&fold(name)).map(|at| (at, name)))
        .max_by_key(|(at, _)| *at)
        .map(|(_, name)| name.clone())
}

/// The longest endpoint name written exactly at the start of `text`. Longest,
/// so `"theo durand"` wins over a bare `"theo"` that also happens to be an
/// endpoint — the shorter one would leave `" durand"` in front of the walk and
/// silently truncate the enumeration.
fn name_at(text: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter(|name| text.starts_with(&fold(name)))
        .max_by_key(|name| name.len())
        .cloned()
}

/// The longest endpoint name `head` ENDS on — who a clitic belongs to.
fn name_before(head: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter(|name| head.ends_with(&fold(name)))
        .max_by_key(|name| name.len())
        .cloned()
}

/// The first endpoint named anywhere in `text`, and where its mention begins.
fn first_name(text: &str, names: &[String]) -> Option<(usize, String)> {
    names
        .iter()
        .filter_map(|name| text.find(&fold(name)).map(|at| (at, name)))
        .min_by_key(|(at, name)| (*at, std::cmp::Reverse(name.len())))
        .map(|(at, name)| (at, name.clone()))
}

/// `first`, plus every further endpoint the SAME enumeration lists after it.
///
/// The walk stops at the first thing that is not a separator followed by an
/// endpoint. That bound is what keeps a following sentence from contributing a
/// carrier — re-pointing "Bruno est le pere de Theo" as though Bruno were a
/// sister of Theo is far worse than the edge it would have added.
fn enumeration_from(text: &str, first: String, names: &[String]) -> Vec<String> {
    let mut rest = &text[fold(&first).len()..];
    let mut bearers = vec![first];
    while let Some((name, tail)) = next_enumerated(rest, names) {
        bearers.push(name);
        rest = tail;
    }
    bearers
}

/// Verbs that mark the name before them as the SUBJECT of a new clause
/// rather than another item in a list.
///
/// `", et "` and `" et "` are enumeration separators AND the way French joins
/// two clauses, so the separator alone cannot tell "a sister, Camille, and
/// Lea" from "a sister, Camille, and Bruno IS the father of Marie". What
/// separates them is what follows the name: an item is followed by another
/// separator or by the end of its clause, a subject is followed by a verb.
const CLAUSE_VERBS: &[&str] = &[
    " est ",
    " sont ",
    " etait ",
    " etaient ",
    " a ",
    " ont ",
    " avait ",
    " avaient ",
    " is ",
    " are ",
    " was ",
    " were ",
    " has ",
    " have ",
    " had ",
];

/// The next endpoint of an enumeration, and what follows its mention.
///
/// Returns `None` when the name opens a new clause — bounding the walk at the
/// sentence is not enough, because a sentence holds several clauses. Letting
/// one through re-points an edge the passage states CORRECTLY: "Bruno est le
/// pere de Marie" became "Marie est le pere de Bruno", a confident falsehood
/// where there had been none. Strictly worse than the edge the walk exists to
/// add.
fn next_enumerated<'a>(rest: &'a str, names: &[String]) -> Option<(String, &'a str)> {
    let tail = strip_any(rest, ENUMERATION_SEPARATORS)?;
    let name = name_at(tail, names)?;
    let cut = fold(&name).len();
    let after = &tail[cut..];
    if CLAUSE_VERBS.iter().any(|verb| after.starts_with(verb)) {
        return None;
    }
    Some((name, after))
}

/// The carriers a possessive introduces: the first endpoint named after the
/// noun, plus the rest of its enumeration.
fn bearers_after(after: &str, names: &[String]) -> Vec<String> {
    match first_name(after, names) {
        Some((at, first)) => enumeration_from(&after[at..], first, names),
        None => Vec::new(),
    }
}

/// The carriers written RIGHT at the start of `text`, one article tolerated.
/// Requiring them there is what keeps a genitive from reaching across a clause
/// it does not own.
fn bearers_at(text: &str, names: &[String]) -> Vec<String> {
    [Some(text), strip_any(text, LEADING_ARTICLES)]
        .into_iter()
        .flatten()
        .find_map(|text| name_at(text, names).map(|first| enumeration_from(text, first, names)))
        .unwrap_or_default()
}

/// The earliest possessive construction in `folded`: `"X a une soeur, Y"`.
fn find_possessive(folded: &str, names: &[String]) -> Option<Kinship> {
    let (start, noun, end) = POSSESSIVE_MARKERS
        .iter()
        .filter_map(|marker| folded.find(marker).map(|at| at + marker.len()))
        .filter_map(|start| {
            let (noun, len) = noun_at(folded.get(start..)?)?;
            Some((start, noun, start + len))
        })
        .min_by_key(|(start, _, _)| *start)?;
    Some(Kinship {
        noun,
        holder: holder_of(folded.get(..start)?, names)?,
        bearers: bearers_after(folded.get(end..)?, names),
    })
}

/// The earliest genitive construction in `folded`, either word order.
fn find_genitive(folded: &str, names: &[String]) -> Option<Kinship> {
    find_of_genitive(folded, names).or_else(|| find_saxon_genitive(folded, names))
}

/// `"<noun> de <holder> est <bearer>"` — the French genitive and its English
/// `"of"` twin, scanned left to right so the earliest reading wins.
fn find_of_genitive(folded: &str, names: &[String]) -> Option<Kinship> {
    let mut links: Vec<(usize, usize)> = GENITIVE_LINKS
        .iter()
        .flat_map(|link| folded.match_indices(link).map(|(at, m)| (at, m.len())))
        .collect();
    links.sort_unstable();
    links
        .into_iter()
        .find_map(|(at, len)| of_genitive_at(folded, at, len, names))
}

/// One `"<noun> de <holder> est <bearer>"` reading, anchored on the link at
/// `at`.
///
/// Every step has to hold exactly — the noun ENDS where the link starts, the
/// holder STARTS where it ends, and the copula follows the holder's name
/// immediately. That is what keeps a copule out: "Camille est la soeur de Theo"
/// carries the very same `"<noun> de <holder>"` fragment and is already right,
/// but its "est" sits on the wrong side of the noun, so nothing follows the
/// holder and the reading is rejected rather than mirrored.
fn of_genitive_at(folded: &str, at: usize, len: usize, names: &[String]) -> Option<Kinship> {
    let noun = noun_before(folded.get(..at)?)?;
    let after_link = folded.get(at + len..)?;
    let holder = name_at(after_link, names)?;
    let after_holder = after_link.get(fold(&holder).len()..)?;
    let bearers = bearers_at(strip_any(after_holder, GENITIVE_COPULAS)?, names);
    Some(Kinship {
        noun,
        holder,
        bearers,
    })
}

/// `"<holder>'s <noun> is <bearer>"` — the English genitive, holder first.
fn find_saxon_genitive(folded: &str, names: &[String]) -> Option<Kinship> {
    folded
        .match_indices(SAXON_MARKER)
        .find_map(|(at, marker)| saxon_genitive_at(folded, at, marker.len(), names))
}

/// One `"<holder>'s <noun> is <bearer>"` reading, anchored on the clitic at
/// `at`. As tight as [`of_genitive_at`]: the holder's name must end on the
/// clitic, the noun must start right after it, and the copula must follow it.
fn saxon_genitive_at(folded: &str, at: usize, len: usize, names: &[String]) -> Option<Kinship> {
    let holder = name_before(folded.get(..at)?, names)?;
    let (noun, noun_len) = noun_at(folded.get(at + len..)?)?;
    let after_noun = folded.get(at + len + noun_len..)?;
    let bearers = bearers_at(strip_any(after_noun, GENITIVE_COPULAS)?, names);
    Some(Kinship {
        noun,
        holder,
        bearers,
    })
}

/// The kinship relation the passage states, in whichever form states it. The
/// possessive is tried first: its marker is the most specific, so a sentence
/// that could be read both ways reads as the possessive it is.
fn find_kinship(folded: &str, names: &[String]) -> Option<Kinship> {
    find_possessive(folded, names).or_else(|| find_genitive(folded, names))
}

/// The head word of a predicate label, folded: `"sœur de"` → `"soeur"`.
fn predicate_stem(predicate: &str) -> String {
    fold(predicate)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The kinship noun a predicate label names, plural tolerated (`"sœurs de"` →
/// `"soeur"`). `None` for any non-kinship label, which is what leaves an
/// unrelated edge between the same two people untouched.
fn predicate_noun(predicate: &str) -> Option<&'static str> {
    let stem = predicate_stem(predicate);
    KINSHIP_NOUNS
        .iter()
        .find(|(spelling, _)| word_prefix_len(&stem, spelling) == Some(stem.len()))
        .map(|(_, canonical)| *canonical)
}

/// Whether the triple runs between exactly these two entities, either way round.
fn joins(relation: &ExtractedRelation, one: &str, other: &str) -> bool {
    (relation.subject == one && relation.object == other)
        || (relation.subject == other && relation.object == one)
}

/// Point one triple the way the passage states it.
///
/// The triple built on the RELATION the passage named belongs to the person
/// that noun introduced; any *other* kinship relation over the same pair is its
/// converse and therefore runs the other way. That single rule is also all an
/// alliance ever needs: list `"beau-frere"` in the table and its converse is
/// whatever else the extractor labelled the pair with. Anything else is
/// untouched.
///
/// "Same relation" is decided on the CANONICAL noun, never on the spelling —
/// otherwise `"sister"` and `"soeur"`, or `"mari"` and `"epoux"`, read as each
/// other's converse and the edge is stored backwards (#1754).
fn reorient(relation: &mut ExtractedRelation, noun: &str, holder: &str, bearer: &str) {
    let Some(stem) = predicate_noun(&relation.predicate) else {
        return;
    };
    if !joins(relation, holder, bearer) {
        return;
    }
    let (subject, object) = if stem == noun {
        (bearer, holder)
    } else {
        (holder, bearer)
    };
    relation.subject = subject.to_string();
    relation.object = object.to_string();
}

/// Re-point the kinship triples the passage states, so each label sits on the
/// person who actually carries it.
///
/// A no-op unless the passage contains a possessive or a genitive naming a
/// kinship noun AND both sides of it resolve to entities the triples already
/// mention — the pass never invents an edge, never drops one, and never touches
/// a copule. A construction naming several carriers re-points the triple of
/// each; one that names a carrier no triple mentions simply has no triple to
/// re-point, since synthesising the edge would break the never-invent rule that
/// makes this pass safe over a hallucinating backend.
pub(crate) fn orient_kinship(passage: &str, relations: &mut [ExtractedRelation]) {
    let folded = fold(passage);
    let names = endpoint_names(relations);
    let Some(kinship) = find_kinship(&folded, &names) else {
        return;
    };
    for bearer in &kinship.bearers {
        if *bearer == kinship.holder {
            continue;
        }
        for relation in relations.iter_mut() {
            reorient(relation, kinship.noun, &kinship.holder, bearer);
        }
    }
}

/// Failure produced by an [`Extractor`] backend (e.g. a network-backed model
/// that cannot be reached, or output that cannot be parsed into facts).
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// The extraction backend (network, subprocess, …) returned an error.
    #[error("extraction backend error: {0}")]
    Backend(String),
    /// The backend produced output that could not be parsed into facts.
    #[error("could not parse facts from extractor output: {0}")]
    Parse(String),
}

/// Turns a passage of raw text into atomic, graph-ready facts.
///
/// Implement this to plug in any model — a local LLM, a hosted API, or a
/// deterministic rule set — and feed the result straight into
/// [`crate::MemoryService::remember_extracted`].
pub trait Extractor {
    /// Extract the atomic facts a reader would remember from `text`.
    ///
    /// # Errors
    /// Returns [`ExtractError`] if the backend fails or its output cannot be
    /// parsed into facts.
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError>;

    /// Extract the facts *and* the entity→entity edges and entity attributes
    /// the passage states.
    ///
    /// Defaults to [`Self::extract`] with no relations and no attributes, so
    /// every backend written against the fact-only contract keeps compiling
    /// and keeps working — it simply builds the bipartite fact↔topic graph it
    /// always did. A backend that can read structure overrides this.
    ///
    /// # Errors
    /// Returns [`ExtractError`] if the backend fails or its output cannot be
    /// parsed.
    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        Ok(Extraction {
            facts: self.extract(text)?,
            ..Extraction::default()
        })
    }
}

/// Forward [`Extractor`] through an [`Arc`](std::sync::Arc), so a shared `Arc<dyn Extractor>`
/// (e.g. one held by the MCP server) satisfies the `X: Extractor` bound on
/// [`crate::MemoryService::remember_extracted`].
///
/// Both methods are forwarded. Forwarding only `extract` would silently route
/// every `Arc`-held backend — which is *every* backend the MCP server and the
/// bindings use — through the fact-only default, discarding the relations and
/// attributes the inner extractor actually produced.
impl<T: Extractor + ?Sized> Extractor for std::sync::Arc<T> {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        (**self).extract(text)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        (**self).extract_graph(text)
    }
}

/// A shared, object-safe extractor. The MCP server and the language bindings
/// hold one of these (an `Option`), so the extraction tool can be attached at
/// runtime without the type being generic.
pub type DynExtractor = std::sync::Arc<dyn Extractor + Send + Sync>;

// --- Always-available backend: the outline a passage states -------------------
//
// The twin of `HashEmbedder` on this side of the crate, and for the same
// reason: without a dependency-free choice, every contract
// `remember_extracted` publishes is reachable only through a network call, so
// no binding can exercise it and no test can prove it. Deliberately NOT behind
// `extract` — that feature exists to pull in the HTTP client, which this
// backend does not need.

/// Deterministic, network-free extractor: it reads the structure a passage
/// STATES instead of inferring it.
///
/// A generative backend guesses which facts a paragraph holds; this one is
/// told. Each non-blank line of the passage is one directive:
///
/// | line | yields |
/// |---|---|
/// | `edge: <subject> \| <predicate> \| <object>` | one [`ExtractedRelation`] |
/// | `attr: <entity> \| <key> \| <json value>` | one [`ExtractedAttribute`] |
/// | `fact: <text> \| <topic>, <topic>` | one [`ExtractedFact`] |
/// | anything else | one [`ExtractedFact`], no topics |
///
/// Entity names are canonicalized (trimmed, lowercased) exactly as
/// [`ExtractedFact::entities`] are, so they resolve to the SAME
/// content-addressed hubs a generative backend's would — the two backends can
/// write into one graph.
///
/// Its purpose matches [`crate::HashEmbedder`]'s: reproducible tests and
/// offline behavior. It reads no natural language, so a caller holding only
/// prose wants a generative backend. What it offers instead is the one thing a
/// model cannot: the graph is exactly the one the caller wrote down — up to
/// [`orient_kinship`], the repointing pass EVERY backend's relations go
/// through, which can flip a triple whose predicate is a kinship noun the
/// passage also states possessively.
///
/// A malformed directive is an [`ExtractError::Parse`], never a silently
/// dropped line: a graph that quietly loses half of what it was handed is
/// worse than one that refuses.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutlineExtractor;

/// The `|`-separated fields of one directive body, trimmed.
fn directive_fields(rest: &str) -> Vec<&str> {
    rest.split('|').map(str::trim).collect()
}

/// The error a directive carrying the wrong number of fields deserves.
fn wrong_field_count(kind: &str, expected: usize, given: usize) -> ExtractError {
    ExtractError::Parse(format!(
        "`{kind}:` takes {expected} `|`-separated fields, {given} given"
    ))
}

/// `edge: <subject> | <predicate> | <object>`.
fn parse_edge(rest: &str) -> Result<ExtractedRelation, ExtractError> {
    let fields = directive_fields(rest);
    let [subject, predicate, object] = fields[..] else {
        return Err(wrong_field_count("edge", 3, fields.len()));
    };
    if subject.is_empty() || predicate.is_empty() || object.is_empty() {
        return Err(ExtractError::Parse(
            "`edge:` takes a non-blank subject, predicate and object".to_owned(),
        ));
    }
    Ok(ExtractedRelation {
        subject: crate::service::canonical_entity_name(subject),
        predicate: predicate.to_owned(),
        object: crate::service::canonical_entity_name(object),
    })
}

/// `attr: <entity> | <key> | <json value>`.
fn parse_attr(rest: &str) -> Result<ExtractedAttribute, ExtractError> {
    let fields = directive_fields(rest);
    let [entity, key, value] = fields[..] else {
        return Err(wrong_field_count("attr", 3, fields.len()));
    };
    if entity.is_empty() || key.is_empty() {
        return Err(ExtractError::Parse(
            "`attr:` takes a non-blank entity and key".to_owned(),
        ));
    }
    // Parsed as JSON, not stored as text, because `recall_where` comparisons
    // are type-strict: an age handed over as `"15"` would never match a
    // numeric filter (see [`ExtractedAttribute::value`]).
    let value = serde_json::from_str(value)
        .map_err(|err| ExtractError::Parse(format!("`attr:` value is not JSON: {err}")))?;
    Ok(ExtractedAttribute {
        entity: crate::service::canonical_entity_name(entity),
        key: key.to_owned(),
        value,
    })
}

/// `fact: <text> | <topic>, <topic>`, and the fallback for any other line.
fn parse_fact(body: &str) -> Result<ExtractedFact, ExtractError> {
    let (text, topics) = body.split_once('|').unwrap_or((body, ""));
    let text = text.trim();
    if text.is_empty() {
        return Err(ExtractError::Parse(
            "a fact line takes a non-blank text".to_owned(),
        ));
    }
    Ok(ExtractedFact {
        text: text.to_owned(),
        entities: topics
            .split(',')
            .map(crate::service::canonical_entity_name)
            .filter(|topic| !topic.is_empty())
            .collect(),
    })
}

impl Extractor for OutlineExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let mut extraction = Extraction::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("edge:") {
                extraction.relations.push(parse_edge(rest)?);
            } else if let Some(rest) = line.strip_prefix("attr:") {
                extraction.attributes.push(parse_attr(rest)?);
            } else {
                extraction
                    .facts
                    .push(parse_fact(line.strip_prefix("fact:").unwrap_or(line))?);
            }
        }
        Ok(extraction)
    }
}

/// What a caller must do to honour a requested extraction backend.
///
/// Returned by [`select_extractor`], which is the single place that knows which
/// backend names exist. Splitting the answer into these three shapes is what
/// lets the dependency-free backends be selected in **any** build: only the
/// [`Self::NeedsRemoteConfig`] arm requires an optional dependency and the URL
/// and model that go with it, and only that arm's construction is feature-gated.
pub enum ExtractorSelection {
    /// No extraction. Tools that need an extractor answer "not configured".
    Disabled,
    /// Ready to use as-is: needs no configuration, no network, no optional
    /// dependency. Attach it and the graph builds.
    Ready(DynExtractor),
    /// A network-backed backend the caller must build itself, because only the
    /// caller knows its URL and model. Carries the backend's name so the caller
    /// can dispatch without re-parsing the string.
    NeedsRemoteConfig(&'static str),
}

/// Hand-written because [`DynExtractor`] is a trait object and the trait does
/// not require `Debug` — a backend is identified by its shape here, never by
/// dumping its innards (an HTTP-backed one holds a URL, and a panic message is
/// not the place for it).
impl std::fmt::Debug for ExtractorSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::Ready(_) => f.write_str("Ready(<extractor>)"),
            Self::NeedsRemoteConfig(name) => write!(f, "NeedsRemoteConfig({name})"),
        }
    }
}

/// Resolve an extraction backend name to what the caller must do about it.
///
/// # Why this exists, and why it is in the library rather than the binary
///
/// The selection used to live inside the daemon's `#[cfg(feature = "extract")]`
/// block. That gate is what made [`OutlineExtractor`] unreachable from the MCP
/// server (#1734): the extractor needs no dependency and is linked into every
/// build, but the only code that could *choose* it was compiled away unless an
/// unrelated HTTP feature was on. Two of the twenty published tools were dead by
/// default as a result — `remember_extracted` refused outright, and `entity`
/// answered `found: false` for every name, entity hubs being born only of
/// extraction.
///
/// Living here rather than in `main.rs` also means the daemon and the tests
/// exercise the **same** function: a test can select `outline` and drive the
/// real server with the result, instead of proving a seam written for the test.
///
/// # Errors
/// A human-readable message naming the accepted forms, for an unknown backend.
pub fn select_extractor(backend: &str) -> Result<ExtractorSelection, String> {
    match backend {
        // No `#[cfg]` here, and that absence IS the fix: this arm must survive
        // in a build without any HTTP feature, which is exactly the build the
        // published binary ships.
        "outline" => Ok(ExtractorSelection::Ready(std::sync::Arc::new(
            OutlineExtractor,
        ))),
        "ollama" => Ok(ExtractorSelection::NeedsRemoteConfig("ollama")),
        // A protocol, not a vendor — see [`crate::select_embedder`]'s own
        // `openai` arm. The two roles accept the same names on purpose: an
        // operator who learned one has learned the other.
        "openai" => Ok(ExtractorSelection::NeedsRemoteConfig("openai")),
        "none" | "" => Ok(ExtractorSelection::Disabled),
        other => Err(format!(
            "unknown extraction backend '{other}' (expected 'outline' for the \
             offline deterministic reader, 'ollama' for a local generative \
             model, 'openai' for any OpenAI-compatible server — oMLX, \
             llama.cpp, LM Studio, vLLM or a hosted provider, selected by URL \
             rather than by name — or 'none')"
        )),
    }
}

// --- Optional batteries-included backend: a local Ollama generative model -----
//
// Enabled with `--features extract`. The default build omits this backend (and
// its HTTP dependency) so the shipped binary stays tiny and fully offline. Like
// the Ollama embedder, it calls a model the user already runs locally, so the
// text never leaves the machine.

/// Default Ollama base URL for the generative extraction endpoint.
#[cfg(feature = "extract")]
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Per-request timeout. Generation is far slower and more stall-prone than an
/// embedding call, so a wedged model fails the call instead of hanging forever.
#[cfg(feature = "extract")]
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// Ceiling on establishing the TCP connection to Ollama. Short on purpose: a
/// local daemon accepts at once or is not running, and `ureq`'s 30 s default
/// would be paid once per replay.
#[cfg(feature = "extract")]
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Ceiling on writing the request (prompt upload). Unlike the read bound, this
/// one is applied to the socket at connect time and is genuinely in force.
#[cfg(feature = "extract")]
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Ceiling on how many tokens one extraction call may generate.
///
/// Unbounded, the real graph-extraction prompt measured 3 933 completion
/// tokens for a twelve-word sentence — 1 min 59 s spent generating JSON to
/// store one fact (#1846). The same call capped at 600 tokens measured
/// 14.9 s. 512 sits just under that, comfortably above what any realistic
/// sentence's worth of triples needs, and turns the worst case into a
/// bounded one instead of a tuning knob callers have to discover by timing
/// out.
#[cfg(feature = "extract")]
const MAX_GENERATION_TOKENS: u32 = 512;

/// The knobs that actually configure the extractor, named in its failures.
///
/// **Not** the embedder's variables. `main.rs`'s `build_ollama_extractor` reads
/// `VELESDB_MEMORY_EXTRACTOR_URL`/`_MODEL`; telling a user to set
/// `VELESDB_MEMORY_OLLAMA_URL` here would send them to edit a setting this code
/// path never consults — an "actionable" message that is actively wrong. There
/// is no offline fallback to offer either: extraction is opt-in, and running
/// without it is simply not passing an extractor.
#[cfg(feature = "extract")]
const EXTRACT_LEVERS: crate::http_retry::FailureLevers<'static> =
    crate::http_retry::FailureLevers {
        url_var: "VELESDB_MEMORY_EXTRACTOR_URL",
        model_var: "VELESDB_MEMORY_EXTRACTOR_MODEL",
        fallback: None,
    };

/// How one generation attempt failed — transport and body failures may be
/// replayed, a complete response is the server's final word.
#[cfg(feature = "extract")]
enum GenerateCall {
    /// The request never completed. Boxed: `ureq::Error::Status` carries a
    /// whole `Response`.
    Transport(Box<ureq::Error>),
    /// Headers arrived but the body did not read back in full.
    Body(std::io::Error),
}

/// Replay policy for one generation attempt.
#[cfg(feature = "extract")]
fn generate_is_retryable(err: &GenerateCall) -> bool {
    match err {
        GenerateCall::Transport(inner) => crate::http_retry::is_retryable(inner),
        GenerateCall::Body(inner) => crate::http_retry::io_is_retryable(inner),
    }
}

/// Turn a failed generation into a message that names the endpoint, the model,
/// how many attempts were spent, and the variables that change the outcome.
#[cfg(feature = "extract")]
fn describe_generate_failure(url: &str, model: &str, err: &GenerateCall, attempts: u32) -> String {
    let cause = match err {
        GenerateCall::Transport(inner) => inner.to_string(),
        GenerateCall::Body(inner) => format!("reading the response failed: {inner}"),
    };
    crate::http_retry::actionable_ollama_failure(
        "generate",
        url,
        model,
        attempts,
        &cause,
        &EXTRACT_LEVERS,
    )
}

/// Extracts facts through a local Ollama `/api/generate` endpoint, keeping the
/// model — and therefore the source text — on the user's own machine.
///
/// The caller picks the generative model (Ollama has no universal default for
/// generation); `temperature` is pinned to `0` and `think` disabled for stable,
/// reproducible output.
#[cfg(feature = "extract")]
#[derive(Debug, Clone)]
pub struct OllamaExtractor {
    base_url: String,
    model: String,
    agent: ureq::Agent,
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// Build an extractor targeting `model` on the Ollama server at `base_url`
    /// (e.g. [`DEFAULT_OLLAMA_URL`]).
    ///
    /// The agent is bounded on four axes, not one. See
    /// [`crate::embedder`]'s `embed_agent` for why `timeout_read` is
    /// subordinate to the global `timeout` in `ureq` and must not be read as a
    /// per-read guarantee; `timeout_connect` and `timeout_write` are the two
    /// that actually bite. The connect bound matters most here: `ureq`'s own
    /// default is 30 s, which for a `localhost` daemon is 15x too long — and
    /// with replays, that idle wait would be paid three times over.
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let timeout = std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_write(WRITE_TIMEOUT)
            .timeout_read(timeout)
            .timeout(timeout)
            .build();
        Self {
            base_url: base_url.into(),
            model: model.into(),
            agent,
        }
    }
}

/// Read a model's reply as the flat fact list [`Extractor::extract`] promises.
///
/// A free function because every generative backend produces the same reply
/// and reads it the same way — only the transport differs. Leaving a copy in
/// each `impl` would let two backends drift on what counts as a valid answer.
#[cfg(feature = "extract")]
fn facts_from_reply(reply: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
    let raw =
        json_slice::<Vec<RawFact>>(reply).ok_or_else(|| ExtractError::Parse(truncate(reply)))?;
    Ok(raw.into_iter().filter_map(RawFact::into_fact).collect())
}

/// [`facts_from_reply`]'s counterpart for [`Extractor::extract_graph`].
#[cfg(feature = "extract")]
fn extraction_from_reply(reply: &str) -> Result<Extraction, ExtractError> {
    let raw = json_slice_object::<RawExtraction>(reply)
        .ok_or_else(|| ExtractError::Parse(truncate(reply)))?;
    Ok(raw.into_extraction())
}

#[cfg(feature = "extract")]
impl Extractor for OllamaExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        facts_from_reply(&self.generate(&build_prompt(text))?)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        extraction_from_reply(&self.generate(&build_graph_prompt(text))?)
    }
}

/// Extracts through any **OpenAI-compatible** `/v1/chat/completions` endpoint.
///
/// A sibling of [`OllamaExtractor`], not a layer over it — the same shape the
/// embedding role takes. The prompt stays here, on the role side: it is what
/// this crate wants said, not something the protocol knows about.
#[cfg(feature = "extract")]
#[derive(Debug)]
pub struct OpenAiExtractor {
    client: crate::http_client::HttpJsonClient,
    model: String,
}

#[cfg(feature = "extract")]
impl OpenAiExtractor {
    /// Build an extractor targeting `model` on the server at `base_url`
    /// (origin and port, no path).
    ///
    /// Bounded on the same four axes as [`OllamaExtractor::new`], with the
    /// same generous [`REQUEST_TIMEOUT_SECS`]: generation is slow wherever it
    /// runs, and the ceiling belongs to the role, not to the transport.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        auth: crate::http_client::Auth,
    ) -> Self {
        let timeout = std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_write(WRITE_TIMEOUT)
            .timeout_read(timeout)
            .timeout(timeout)
            .build();
        Self {
            client: crate::http_client::HttpJsonClient::new(
                crate::openai::base_url(&base_url.into()),
                auth,
                agent,
            ),
            model: model.into(),
        }
    }

    /// POST one prompt and return the assistant's reply.
    fn generate(&self, prompt: &str) -> Result<String, ExtractError> {
        let body = crate::openai::chat_body(&self.model, prompt, MAX_GENERATION_TOKENS);
        let payload = self
            .client
            .post_json(crate::openai::CHAT_COMPLETIONS_PATH, &body)
            .map_err(|failure| {
                ExtractError::Backend(crate::http_retry::actionable_openai_failure(
                    "chat/completions",
                    &failure.url,
                    &self.model,
                    failure.attempts,
                    &failure.cause,
                    Some(
                        "use the offline deterministic reader with \
                         VELESDB_MEMORY_EXTRACTOR=outline",
                    ),
                ))
            })?;
        crate::openai::parse_chat_response(&payload).map_err(ExtractError::Backend)
    }
}

#[cfg(feature = "extract")]
impl Extractor for OpenAiExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        facts_from_reply(&self.generate(&build_prompt(text))?)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        extraction_from_reply(&self.generate(&build_graph_prompt(text))?)
    }
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// POST one prompt to Ollama's `/api/generate` and return the trimmed reply,
    /// replaying the call when the failure is transient.
    ///
    /// Same defect, same repair as the embedder: this extractor also holds one
    /// `ureq::Agent`, so it also hands out pooled keep-alive connections that
    /// Ollama may have closed, and `ureq` will not replay a POST with a body.
    /// The whole attempt — POST and body read — is inside the closure so a
    /// truncated response is replayed rather than surfacing as a parse error.
    fn generate(&self, prompt: &str) -> Result<String, ExtractError> {
        let url = format!("{}/api/generate", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "think": false,
            // Extraction models are large — the one this crate documents as an
            // example is 21.9 GB — so an unload between calls is the dominant
            // cost, not the generation. Shares the embedder's knob so one
            // setting governs every Ollama call the daemon makes.
            "keep_alive": crate::embedder::keep_alive(),
            "options": { "temperature": 0, "num_predict": MAX_GENERATION_TOKENS },
        })
        .to_string();
        let attempt = || {
            let response = self
                .agent
                .post(&url)
                .set("Content-Type", "application/json")
                .send_string(&body)
                .map_err(|err| GenerateCall::Transport(Box::new(err)))?;
            response.into_string().map_err(GenerateCall::Body)
        };

        let payload = crate::http_retry::with_retry(
            &crate::http_retry::HTTP_RETRIES,
            generate_is_retryable,
            attempt,
        )
        .map_err(|(err, attempts)| {
            ExtractError::Backend(describe_generate_failure(&url, &self.model, &err, attempts))
        })?;
        parse_generate_response(&payload)
    }
}

/// The strict JSON contract the extraction prompt asks the model to honour.
#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawFact {
    fact: String,
    #[serde(default)]
    entities: Vec<String>,
}

#[cfg(feature = "extract")]
impl RawFact {
    /// Keep a fact only if it has text; trim and lowercase its topics, dropping
    /// blanks and duplicates so the same topic recurs as the same graph hub.
    fn into_fact(self) -> Option<ExtractedFact> {
        let text = self.fact.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mut entities: Vec<String> = self
            .entities
            .into_iter()
            .map(|entity| entity.trim().to_lowercase())
            .filter(|entity| !entity.is_empty())
            .collect();
        entities.sort_unstable();
        entities.dedup();
        Some(ExtractedFact { text, entities })
    }
}

/// Canonical form of an entity name: trimmed and lowercased. The one place
/// the rule lives, so a name arriving as a topic, as a relation endpoint, or
/// as an attribute owner always resolves to the SAME entity hub.
#[cfg(feature = "extract")]
fn canonical_entity(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The strict JSON contract the *graph* extraction prompt asks for.
#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawExtraction {
    #[serde(default)]
    facts: Vec<RawFact>,
    #[serde(default)]
    relations: Vec<RawRelation>,
    #[serde(default)]
    attributes: Vec<RawAttribute>,
}

#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawRelation {
    subject: String,
    predicate: String,
    object: String,
}

#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawAttribute {
    entity: String,
    key: String,
    value: serde_json::Value,
}

#[cfg(feature = "extract")]
impl RawExtraction {
    /// Canonicalize and drop the unusable: a relation missing an endpoint or a
    /// label, an attribute missing an owner or a name. A malformed item is
    /// skipped rather than failing the whole passage — one bad triple must not
    /// cost the caller every good fact in the same reply.
    fn into_extraction(self) -> Extraction {
        Extraction {
            facts: self
                .facts
                .into_iter()
                .filter_map(RawFact::into_fact)
                .collect(),
            relations: self
                .relations
                .into_iter()
                .filter_map(RawRelation::into_relation)
                .collect(),
            attributes: self
                .attributes
                .into_iter()
                .filter_map(RawAttribute::into_attribute)
                .collect(),
        }
    }
}

#[cfg(feature = "extract")]
impl RawRelation {
    fn into_relation(self) -> Option<ExtractedRelation> {
        let subject = canonical_entity(&self.subject);
        let object = canonical_entity(&self.object);
        let predicate = self.predicate.trim().to_string();
        // A self-loop carries no information and would sit in the graph as a
        // permanent dead end, so it is dropped alongside the incomplete ones.
        if subject.is_empty() || object.is_empty() || predicate.is_empty() || subject == object {
            return None;
        }
        Some(ExtractedRelation {
            subject,
            predicate,
            object,
        })
    }
}

#[cfg(feature = "extract")]
impl RawAttribute {
    fn into_attribute(self) -> Option<ExtractedAttribute> {
        let entity = canonical_entity(&self.entity);
        let key = self.key.trim().to_string();
        // A null value is the model saying "not stated"; storing it would make
        // an absent attribute look like a known-empty one.
        if entity.is_empty() || key.is_empty() || self.value.is_null() {
            return None;
        }
        Some(ExtractedAttribute {
            entity,
            key,
            value: self.value,
        })
    }
}

/// Build the *graph* extraction prompt: the passage plus a strict JSON
/// contract covering facts, entity→entity edges, and entity attributes.
///
/// The contract insists numbers stay JSON numbers. `recall_where` compares
/// type-strictly, so an age emitted as `"15"` would never match `age >= 15` —
/// no error, just a silent miss, which is the worst possible failure mode for
/// a memory system.
#[cfg(feature = "extract")]
fn build_graph_prompt(text: &str) -> String {
    format!(
        "You are building a knowledge graph from the passage below.\n\n\
Passage:\n{text}\n\n\
STEP 0 — Identify the passage's language. Everything you write (facts, \
predicates, attribute keys) MUST be in THAT language. Do not copy the language \
of the examples below: they are shown in several languages on purpose, and you \
must match the PASSAGE, never the example.\n\n\
Return THREE things.\n\n\
1. \"facts\": the atomic, standalone facts a person would remember, in the \
passage's language. Rewrite each as a self-contained sentence (resolve \
pronouns to names; keep absolute dates). For each, list 1-4 key TOPICS it \
concerns, as short canonical lowercase noun phrases, so the same topic recurs \
as the SAME tag across passages.\n\n\
2. \"relations\": every explicit relationship BETWEEN TWO NAMED ENTITIES, as \
subject/predicate/object triples. Use the entity's full name, lowercase \
(e.g. \"bruno durand\").\n\
The predicate is a LABEL, not a sentence: **at most 3 words**, lowercase, in \
the passage's language. Examples of the SHAPE, each in its own language — \
match the passage, not these: a French passage gives \"travaille chez\", \
\"pere de\"; an English passage gives \"works at\", \"father of\"; a Spanish \
passage gives \"trabaja en\". NEVER restate the sentence — write \"surveille \
les fuites\", not \"est utilise pour la surveillance de fuites de donnees\". \
If you cannot say it in 3 words, pick the closest short label.\n\
DIRECTION: the subject is whoever CARRIES the relation, not the subject of the \
sentence. \"A a une soeur, B\" means B is A's sister, so the triple is \
B/\"soeur de\"/A — never A/\"soeur de\"/B. \"A has a brother, B\" means B is \
A's brother: B/\"brother of\"/A. Same for every possessive.\n\
Never emit both directions of the SAME predicate over the same pair — \
\"X brother of Y\" plus \"Y brother of X\" is a contradiction, not a \
converse: emit exactly one. But two DIFFERENT predicates the passage states \
separately over the same pair (\"A possede B\" then \"B appartient a A\") \
are two stated facts — keep both.\n\
Every named entity the passage RELATES to another must appear in at least one \
triple — an entity that only receives attributes and no edge is a dead end in \
the graph.\n\n\
3. \"attributes\": every property a named entity HAS, as entity/key/value. Use \
short lowercase keys in the passage's language (\"age\", \"ville\", \
\"employeur\"). Emit numbers as JSON NUMBERS, never strings: 15, not \"15\". \
Omit anything the passage does not state.\n\n\
Return ONLY this JSON object, no prose, no markdown fence:\n\
{{\"facts\": [{{\"fact\": string, \"entities\": [string]}}], \
\"relations\": [{{\"subject\": string, \"predicate\": string, \"object\": string}}], \
\"attributes\": [{{\"entity\": string, \"key\": string, \"value\": string|number|boolean}}]}}"
    )
}

/// Build the extraction prompt: the passage plus a strict JSON contract.
#[cfg(feature = "extract")]
fn build_prompt(text: &str) -> String {
    format!(
        "You are building a memory graph from the passage below.\n\n\
Passage:\n{text}\n\n\
Extract the atomic, standalone facts a person would remember. Rewrite each as a \
self-contained sentence (resolve pronouns to names; keep absolute dates). For \
each fact also list 1-4 key TOPICS it concerns: the recurring subjects, \
activities, events, interests, plans, places, organisations, or named people a \
later question might reference. Use short, canonical, lowercase noun phrases \
(e.g. \"adoption\", \"charity race\", \"therapy\", \"new job\") so the same topic \
recurs as the SAME tag across passages.\n\n\
Return ONLY a JSON array, no prose, each item exactly:\n\
{{\"fact\": string, \"entities\": [string]}}"
    )
}

/// Pull the `response` string out of Ollama's `/api/generate` JSON envelope.
#[cfg(feature = "extract")]
fn parse_generate_response(body: &str) -> Result<String, ExtractError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| ExtractError::Backend(format!("invalid generate response: {err}")))?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ExtractError::Backend("ollama reply had no `response` field".to_string()))?;
    Ok(text.trim().to_string())
}

/// A short, single-line preview of model output for error messages.
#[cfg(feature = "extract")]
fn truncate(text: &str) -> String {
    const LIMIT: usize = 120;
    let mut out = String::new();
    for word in text.split_whitespace() {
        // Check the budget *before* pushing so we never need a post-hoc
        // `String::truncate`, which would panic if the limit fell mid-UTF-8 char.
        let sep_len = usize::from(!out.is_empty());
        if out.len() + sep_len + word.len() > LIMIT {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Parse `text` into `T`, first slicing out the outermost JSON array/object.
/// Local models usually honour "return only JSON" but occasionally wrap it in
/// fences or a sentence; slicing the first balanced span tolerates that.
#[cfg(feature = "extract")]
fn json_slice<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice(text)?;
    serde_json::from_str::<T>(slice).ok()
}

/// [`json_slice`] for a reply whose top level is a JSON **object**.
///
/// The array-preferring form cannot be reused: the graph reply is
/// `{"facts": [...], ...}`, whose first `[` belongs to a *nested* field, so
/// preferring arrays slices out the inner facts list and then fails to read it
/// as the whole extraction. That failure is invisible to a stub-backed test —
/// only a real model reply goes through this path.
#[cfg(feature = "extract")]
fn json_slice_object<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice_preferring(text, b'{')?;
    serde_json::from_str::<T>(slice).ok()
}

/// Return the substring spanning the first balanced `[..]` or `{..}`, honouring
/// string literals and escapes so brackets inside quotes don't miscount.
///
/// Prefers an array: the fact-only reply is a JSON list, and prose before it
/// ("Result {ok}: [...]") may carry a stray `{` that would mis-slice the
/// object span instead of the array.
#[cfg(feature = "extract")]
fn balanced_slice(text: &str) -> Option<&str> {
    balanced_slice_preferring(text, b'[')
}

/// [`balanced_slice`] with the caller choosing which delimiter wins when both
/// appear — the shape the caller actually expects at the top level.
#[cfg(feature = "extract")]
fn balanced_slice_preferring(text: &str, preferred: u8) -> Option<&str> {
    let bytes = text.as_bytes();
    let fallback = if preferred == b'[' { b'{' } else { b'[' };
    let start = bytes
        .iter()
        .position(|&b| b == preferred)
        .or_else(|| bytes.iter().position(|&b| b == fallback))?;
    let open = bytes[start];
    let close = if open == b'[' { b']' } else { b'}' };
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        if in_string {
            in_string = step_string(&mut escaped, byte);
        } else if scan_structural(byte, open, close, &mut in_string, &mut depth) {
            return Some(&text[start..=start + offset]);
        }
    }
    None
}

/// Advance the structural scan for one out-of-string byte; returns `true` once
/// the outermost bracket has just closed (`depth` back to zero).
#[cfg(feature = "extract")]
fn scan_structural(byte: u8, open: u8, close: u8, in_string: &mut bool, depth: &mut u32) -> bool {
    if byte == b'"' {
        *in_string = true;
    } else if byte == open {
        *depth += 1;
    } else if byte == close {
        *depth = depth.saturating_sub(1);
        return *depth == 0;
    }
    false
}

/// Advance the in-string escape state for one byte; returns whether the scanner
/// is still inside the string literal afterwards.
#[cfg(feature = "extract")]
fn step_string(escaped: &mut bool, byte: u8) -> bool {
    match (*escaped, byte) {
        (true, _) => {
            *escaped = false;
            true
        }
        (false, b'\\') => {
            *escaped = true;
            true
        }
        (false, b'"') => false,
        (false, _) => true,
    }
}

#[cfg(test)]
#[path = "extractor_selection_tests.rs"]
mod selection_tests;

#[cfg(all(test, feature = "extract"))]
mod tests {
    use super::*;

    /// Regression: the graph reply is an OBJECT whose first `[` belongs to the
    /// nested `facts` field. Slicing with the array preference grabbed that
    /// inner list and failed to read it as the whole extraction — a real model
    /// reply was rejected wholesale while every stub-backed test stayed green.
    #[test]
    fn parses_a_graph_reply_whose_first_bracket_is_nested() {
        let reply = r#"{ "facts": [ { "fact": "Zephyrin is the father of Kaltar.", "entities": ["zephyrin", "kaltar"] } ], "relations": [ { "subject": "zephyrin", "predicate": "pere de", "object": "kaltar" } ], "attributes": [ { "entity": "kaltar", "key": "age", "value": 15 } ] }"#;
        let raw: RawExtraction = json_slice_object(reply).expect("the object is sliced whole");
        let extraction = raw.into_extraction();
        assert_eq!(extraction.facts.len(), 1);
        assert_eq!(extraction.relations.len(), 1);
        // Endpoints asserted by name: a subject↔object swap in
        // `into_relation` kept this test green when only the predicate was
        // checked (#1792).
        assert_eq!(extraction.relations[0].subject, "zephyrin");
        assert_eq!(extraction.relations[0].predicate, "pere de");
        assert_eq!(extraction.relations[0].object, "kaltar");
        assert_eq!(extraction.attributes.len(), 1);
        assert_eq!(extraction.attributes[0].value, serde_json::json!(15));
    }

    /// Prose (and a fenced block) around the object must not defeat slicing.
    #[test]
    fn parses_a_graph_reply_wrapped_in_prose_and_fences() {
        let reply = "Here you go:\n```json\n{\"facts\": [], \"relations\": [{\"subject\": \"a\", \"predicate\": \"knows\", \"object\": \"b\"}], \"attributes\": []}\n```";
        let raw: RawExtraction = json_slice_object(reply).expect("sliced past the fence");
        assert_eq!(raw.into_extraction().relations.len(), 1);
    }

    /// The fact-only path must keep preferring an array: prose carrying a stray
    /// `{` before the list is exactly what that preference exists to survive.
    #[test]
    fn fact_only_slicing_still_prefers_the_array() {
        let reply = "Result {ok}: [{\"fact\": \"A ships B.\", \"entities\": [\"b\"]}]";
        let raw: Vec<RawFact> = json_slice(reply).expect("array sliced despite the stray brace");
        assert_eq!(raw.len(), 1);
    }

    #[test]
    fn graph_prompt_demands_numeric_values_and_the_three_sections() {
        let prompt = build_graph_prompt("Kaltar a 15 ans.");
        assert!(prompt.contains("Kaltar a 15 ans."));
        assert!(prompt.contains("\"relations\""));
        assert!(prompt.contains("\"attributes\""));
        assert!(prompt.contains("15, not \"15\""));
    }

    #[test]
    fn prompt_carries_the_passage_and_json_contract() {
        let prompt = build_prompt("Alice adopted a dog in 2021.");
        assert!(prompt.contains("Alice adopted a dog in 2021."));
        assert!(prompt.contains("\"fact\": string"));
    }

    /// The graph prompt has to bound the predicate explicitly. Asking for "a
    /// short label" was not enough: on real content the model answered
    /// "est utilise pour la surveillance de fuites de donnees" — a restated
    /// sentence, which makes the edge unreadable in `entity()`.
    #[test]
    fn graph_prompt_bounds_the_predicate_and_demands_edges() {
        let prompt = build_graph_prompt("Ahmia is an onion search engine.");
        assert!(prompt.contains("Ahmia is an onion search engine."));
        assert!(
            prompt.contains("at most 3 words"),
            "the predicate length must be a hard bound, not a suggestion"
        );
        assert!(
            prompt.contains("NEVER restate the sentence"),
            "the counter-example is what stops a restated sentence"
        );
        assert!(
            prompt.contains("at least one triple"),
            "an entity with attributes but no edge is a dead end — the prompt \
             must ask for the edge"
        );
    }

    /// The prompt must make the language rule SYMMETRIC (#1846). Its previous
    /// form gave predicate examples in mixed languages with no rule tying the
    /// answer to the passage: on an English passage, `default:fast` emitted
    /// `frere de` (French) — and a graph holding both `works at` and
    /// `travaille chez` for the same relation fragments it into two
    /// predicates. Measured fix: with this rule the same model passes both
    /// languages; a one-sided rule ("answer in French") merely inverted the
    /// defect.
    #[test]
    fn graph_prompt_ties_every_output_to_the_passage_language() {
        let prompt = build_graph_prompt("Sarah Miller has a brother, Tom Miller.");
        assert!(
            prompt.contains("Identify the passage's language"),
            "the language rule must be an explicit first step, not an aside"
        );
        assert!(
            prompt.contains("match the PASSAGE, never the example"),
            "mixed-language examples are load-bearing — the rule must say they \
             are examples of SHAPE, not of language"
        );
    }

    /// The prompt must forbid the fake converse (#1846). Its previous form
    /// said "add the converse ONLY if the passage states it too", and on an
    /// English passage `default:fast` still emitted BOTH `tom brother of
    /// sarah` AND `sarah brother of tom` — each the sibling of the other, a
    /// contradiction `orient_kinship` repairs for kinship only: `works at` /
    /// `manages` would ship inverted. The rule must name the failure, not
    /// just permit its absence.
    #[test]
    fn graph_prompt_forbids_both_directions_of_one_predicate() {
        let prompt = build_graph_prompt("Sarah Miller has a brother, Tom Miller.");
        assert!(
            prompt.contains("emit exactly one"),
            "the one-per-predicate rule must be stated as a hard bound"
        );
        assert!(
            prompt.contains("keep both"),
            "the rule must carry its POSITIVE half too: two different \
             predicates stated separately are two facts — without it a model \
             collapses a real converse pair into one edge (measured 3/3 on \
             the bench's converse case)"
        );
        assert!(
            prompt.contains("is a contradiction, not a converse"),
            "the counter-example is what makes the rule unambiguous — the same \
             device the carrier rule below relies on"
        );
    }

    /// The prompt must state the possessive rule explicitly: asked only to
    /// "state the triple in the direction the passage states it", the model
    /// read the grammatical subject as the subject of the triple and mirrored
    /// every possessive.
    #[test]
    fn graph_prompt_states_which_side_carries_the_relation() {
        let prompt = build_graph_prompt("Theo Durand a une soeur, Camille Durand.");
        assert!(
            prompt.contains("whoever CARRIES the relation"),
            "the rule must name the carrier, not just \"the direction\""
        );
        assert!(
            prompt.contains("never A/\"soeur de\"/B"),
            "the counter-example is what makes the rule unambiguous"
        );
    }

    #[test]
    fn parses_facts_from_a_fenced_reply() {
        let reply = "Sure!\n```json\n[{\"fact\":\"Alice adopted a dog.\",\"entities\":[\"Adoption\",\"adoption\",\"\"]}]\n```";
        let facts: Vec<RawFact> = json_slice(reply).expect("slice json");
        let facts: Vec<ExtractedFact> = facts.into_iter().filter_map(RawFact::into_fact).collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "Alice adopted a dog.");
        // Trimmed, lowercased, deduplicated, blanks dropped.
        assert_eq!(facts[0].entities, vec!["adoption".to_string()]);
    }

    #[test]
    fn drops_a_textless_fact() {
        let raw = RawFact {
            fact: "   ".to_string(),
            entities: vec!["x".to_string()],
        };
        assert!(raw.into_fact().is_none());
    }

    #[test]
    fn parses_response_envelope() {
        let text = parse_generate_response(r#"{"response":"  [] "}"#).expect("parse");
        assert_eq!(text, "[]");
    }

    #[test]
    fn rejects_response_without_field() {
        assert!(matches!(
            parse_generate_response(r#"{"oops":true}"#),
            Err(ExtractError::Backend(_))
        ));
    }
}
