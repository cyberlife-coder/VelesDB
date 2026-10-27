//! Orienting a kinship triple, whichever form the passage states it in.

use super::ExtractedRelation;

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
