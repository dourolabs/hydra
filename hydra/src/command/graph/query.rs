//! Pipe-form query language for `hydra graph search`/`diff`/`log`.
//!
//! This module is the shared parsing + lowering library that the three
//! `hydra graph` subcommands link against. The grammar, semantics, and
//! lowering rules are specified in `/designs/hydra-graph-query-language.md`.
//!
//! # Example
//!
//! ```ignore
//! use hydra::command::graph::query::{parse, LoweredStage};
//!
//! let q = parse("i-abcdef | children rel=child-of transitive | kind=issue").unwrap();
//! let lowered = q.lower();
//! assert_eq!(lowered.source.len(), 1);
//! match &lowered.stages[0] {
//!     LoweredStage::Relations(_) => {}
//!     _ => panic!("first stage should be Relations"),
//! }
//! ```

use std::fmt;
use std::str::FromStr;

use hydra_common::graph::ObjectKind;
use hydra_common::HydraId;

// -- AST ----------------------------------------------------------------

/// A parsed query: an initial vertex set (`source`) followed by zero or more
/// pipe-separated transformer stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Source vertex set, deduplicated at parse time, with original order preserved.
    pub source: Vec<HydraId>,
    pub stages: Vec<Stage>,
}

/// One pipe stage in the query.
///
/// `Ancestors` / `Descendants` are parse-time sugar; [`Query::lower`] collapses
/// them into `Parents` / `Children` with `transitive = true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Parents {
        rel: Option<RelType>,
        transitive: bool,
        exclusive: bool,
    },
    Children {
        rel: Option<RelType>,
        transitive: bool,
        exclusive: bool,
    },
    Neighbors {
        rel: Option<RelType>,
        exclusive: bool,
    },
    Ancestors {
        rel: RelType,
        exclusive: bool,
    },
    Descendants {
        rel: RelType,
        exclusive: bool,
    },
    Scope,
    Kind(Vec<ObjectKind>),
}

/// The five edge labels recognized by the DSL. Mirrors `RelationshipType` in
/// `hydra-server::store::RelationshipType` but lives in `hydra-common` so the
/// parser does not pull in the server crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelType {
    ChildOf,
    BlockedOn,
    HasPatch,
    HasDocument,
    RefersTo,
}

impl RelType {
    /// Canonical kebab-case spelling — the form accepted by the server's
    /// `rel_type` query parameter and emitted by [`Display`](fmt::Display).
    pub const fn as_str(self) -> &'static str {
        match self {
            RelType::ChildOf => "child-of",
            RelType::BlockedOn => "blocked-on",
            RelType::HasPatch => "has-patch",
            RelType::HasDocument => "has-document",
            RelType::RefersTo => "refers-to",
        }
    }
}

impl fmt::Display for RelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a string does not name a known [`RelType`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseRelTypeError(pub String);

impl fmt::Display for ParseRelTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown rel type '{}'", self.0)
    }
}

impl std::error::Error for ParseRelTypeError {}

impl FromStr for RelType {
    type Err = ParseRelTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "child-of" => Ok(RelType::ChildOf),
            "blocked-on" => Ok(RelType::BlockedOn),
            "has-patch" => Ok(RelType::HasPatch),
            "has-document" => Ok(RelType::HasDocument),
            "refers-to" => Ok(RelType::RefersTo),
            other => Err(ParseRelTypeError(other.to_string())),
        }
    }
}

// -- Lowered form -------------------------------------------------------

/// Direction-of-traversal selector for a single `/v1/relations` call.
///
/// Maps to the query-parameter shape:
/// - `Source` → `source_id` / `source_ids` (children of the input set)
/// - `Target` → `target_id` / `target_ids` (parents of the input set)
/// - `Object` → `object_id` / `object_ids` (neighbors of the input set)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Source,
    Target,
    Object,
}

/// One `/v1/relations` call's worth of work, decoupled from the actual HTTP
/// dispatch. The resolver (CLI-side, PRs 3-5) issues the request and walks the
/// vertex set per the inclusive-by-default contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationsQuery {
    pub direction: Direction,
    pub rel: Option<RelType>,
    pub transitive: bool,
    pub exclusive: bool,
}

/// One stage's worth of resolver work after sugar collapse and kind
/// intersection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweredStage {
    /// One `/v1/relations` HTTP call. The resolver applies the inclusive-by-
    /// default contract: `V' = V ∪ T(V)` (default) or `T(V) \ V` (if
    /// `exclusive`).
    ///
    /// Resolver note: for `direction == Direction::Object` over a multi-
    /// element vertex set, dispatch via the `object_ids` plural parameter; for
    /// a single-element set, the singular `object_id` is equivalent and a
    /// fine fallback.
    Relations(RelationsQuery),
    /// `scope` stage: 3 calls' worth (descendants-via-child-of, then
    /// has-patch children, then has-document children). The resolver expands
    /// this at runtime.
    Scope,
    /// Client-side post-filter on the hydrated nodes. Consecutive `Kind`
    /// stages in the parsed query are intersected here.
    Kind(Vec<ObjectKind>),
}

/// Flat instruction list produced by [`Query::lower`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredQuery {
    pub source: Vec<HydraId>,
    pub stages: Vec<LoweredStage>,
}

// -- ParseError ---------------------------------------------------------

/// A parse error with enough information to render a caret-quoted block.
///
/// Position is a byte offset into [`input`](ParseError::input); `span_len`
/// is the length of the offending token in bytes (capped at the remaining
/// input). For non-ASCII inputs the caret may not line up character-for-
/// character, but the DSL alphabet is restricted to ASCII so this does not
/// affect well-formed queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
    pub span_len: usize,
    pub hint: Option<String>,
    pub input: String,
}

impl ParseError {
    fn new(input: &str, position: usize, span_len: usize, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position,
            span_len: span_len.max(1),
            hint: None,
            input: input.to_string(),
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {} at position {}", self.message, self.position)?;
        writeln!(f, "  {}", self.input)?;
        let pad = " ".repeat(self.position);
        let carets = "^".repeat(self.span_len.max(1));
        if let Some(hint) = &self.hint {
            writeln!(f, "  {pad}{carets}")?;
            write!(f, "hint: {hint}")
        } else {
            write!(f, "  {pad}{carets}")
        }
    }
}

impl std::error::Error for ParseError {}

// -- Lexer --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    /// `[A-Za-z0-9_-]+`
    Word(String),
    Pipe,
    Comma,
    Eq,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    /// Byte offset of the first character of the token in the input.
    pos: usize,
    /// Length of the token in bytes.
    len: usize,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn tokenize(input: &str) -> Vec<Token> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == '|' {
            out.push(Token {
                kind: TokenKind::Pipe,
                pos: i,
                len: 1,
            });
            i += 1;
        } else if c == ',' {
            out.push(Token {
                kind: TokenKind::Comma,
                pos: i,
                len: 1,
            });
            i += 1;
        } else if c == '=' {
            out.push(Token {
                kind: TokenKind::Eq,
                pos: i,
                len: 1,
            });
            i += 1;
        } else if is_word_char(c) {
            let start = i;
            while i < bytes.len() && is_word_char(bytes[i] as char) {
                i += 1;
            }
            let word = input[start..i].to_string();
            out.push(Token {
                kind: TokenKind::Word(word),
                pos: start,
                len: i - start,
            });
        } else {
            // Unknown character — emit a single-char "word" so the parser
            // can produce a positional error pointing at it.
            out.push(Token {
                kind: TokenKind::Word(c.to_string()),
                pos: i,
                len: c.len_utf8(),
            });
            i += c.len_utf8();
        }
    }
    out
}

// -- Parser -------------------------------------------------------------

/// Parse a pipe-form query string into a [`Query`] AST.
///
/// Returns a [`ParseError`] with caret-formatted [`Display`](fmt::Display)
/// output on failure.
pub fn parse(input: &str) -> Result<Query, ParseError> {
    let tokens = tokenize(input);
    let mut parser = Parser {
        input,
        tokens: &tokens,
        pos: 0,
    };
    parser.parse_query()
}

struct Parser<'a> {
    input: &'a str,
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eof_position(&self) -> usize {
        self.input.len()
    }

    fn err(&self, position: usize, span_len: usize, message: impl Into<String>) -> ParseError {
        ParseError::new(self.input, position, span_len, message)
    }

    fn parse_query(&mut self) -> Result<Query, ParseError> {
        let source = self.parse_source()?;
        let mut stages = Vec::new();
        while let Some(tok) = self.peek() {
            match &tok.kind {
                TokenKind::Pipe => {
                    self.bump();
                    let stage = self.parse_stage()?;
                    stages.push(stage);
                }
                _ => {
                    return Err(self.err(tok.pos, tok.len, "expected '|' between stages"));
                }
            }
        }
        Ok(Query { source, stages })
    }

    fn parse_source(&mut self) -> Result<Vec<HydraId>, ParseError> {
        let first = match self.peek() {
            Some(tok) => tok.clone(),
            None => {
                return Err(self.err(
                    self.eof_position(),
                    1,
                    "expected source id at start of query",
                ));
            }
        };
        // The first token must be a Word (id). If it's a known stage name,
        // give a helpful "stage before source" hint.
        match &first.kind {
            TokenKind::Word(word) => {
                if STAGE_NAMES.iter().any(|n| n == word) {
                    return Err(self
                        .err(
                            first.pos,
                            first.len,
                            format!("stage '{word}' has no preceding source"),
                        )
                        .with_hint(
                            "queries must start with a source id (e.g., 'i-abcdef | scope')",
                        ));
                }
                if word == "kind" {
                    return Err(self
                        .err(
                            first.pos,
                            first.len,
                            "filter stage 'kind=' has no preceding source",
                        )
                        .with_hint(
                            "queries must start with a source id (e.g., 'i-abcdef | kind=patch')",
                        ));
                }
                let id = parse_hydra_id(self.input, &first)?;
                self.bump();
                let mut ids = vec![id];
                while let Some(tok) = self.peek() {
                    if matches!(tok.kind, TokenKind::Comma) {
                        self.bump();
                        let next_tok = match self.peek() {
                            Some(t) => t.clone(),
                            None => {
                                return Err(self.err(
                                    self.eof_position(),
                                    1,
                                    "expected source id after ','",
                                ));
                            }
                        };
                        let next_id = parse_hydra_id(self.input, &next_tok)?;
                        if !ids.iter().any(|i| i == &next_id) {
                            ids.push(next_id);
                        }
                        self.bump();
                    } else {
                        break;
                    }
                }
                Ok(ids)
            }
            _ => Err(self.err(first.pos, first.len, "expected source id at start of query")),
        }
    }

    fn parse_stage(&mut self) -> Result<Stage, ParseError> {
        let name_tok = match self.peek() {
            Some(tok) => tok.clone(),
            None => {
                return Err(self.err(self.eof_position(), 1, "expected stage after '|'"));
            }
        };
        let name = match &name_tok.kind {
            TokenKind::Word(w) => w.clone(),
            _ => {
                return Err(self.err(name_tok.pos, name_tok.len, "expected stage name after '|'"));
            }
        };

        // Filter stage: `kind=...`.
        if name == "kind" {
            self.bump();
            return self.parse_kind_stage();
        }

        // Relation stage.
        let stage_kind = match name.as_str() {
            "parents" | "children" | "neighbors" | "ancestors" | "descendants" | "scope" => {
                name.clone()
            }
            _ => {
                // Unknown name: maybe Levenshtein hint.
                let mut err = self.err(
                    name_tok.pos,
                    name_tok.len,
                    format!("unknown stage '{name}'"),
                );
                if let Some(hint) = suggest_stage_name(&name) {
                    err = err.with_hint(format!("did you mean '{hint}'?"));
                }
                return Err(err);
            }
        };
        self.bump();
        self.parse_relation_stage(&stage_kind, &name_tok)
    }

    fn parse_kind_stage(&mut self) -> Result<Stage, ParseError> {
        // We've consumed the `kind` word. Now require `=`, then KINDLIST.
        let eq_tok = match self.peek() {
            Some(tok) => tok.clone(),
            None => {
                return Err(self
                    .err(self.eof_position(), 1, "expected '=' after 'kind'")
                    .with_hint(
                        "filter stages are written as 'kind=patch' or 'kind=patch,document'",
                    ));
            }
        };
        if !matches!(eq_tok.kind, TokenKind::Eq) {
            return Err(self
                .err(eq_tok.pos, eq_tok.len, "expected '=' after 'kind'")
                .with_hint("filter stages are written as 'kind=patch' or 'kind=patch,document'"));
        }
        self.bump();

        let mut kinds = Vec::new();
        loop {
            let tok = match self.peek() {
                Some(t) => t.clone(),
                None => {
                    return Err(self.err(
                        self.eof_position(),
                        1,
                        "expected kind name after 'kind='",
                    ));
                }
            };
            let word = match &tok.kind {
                TokenKind::Word(w) => w.clone(),
                _ => {
                    return Err(self.err(tok.pos, tok.len, "expected kind name"));
                }
            };
            let kind = ObjectKind::from_str(&word).map_err(|_| {
                self.err(tok.pos, tok.len, format!("unknown kind '{word}'"))
                    .with_hint("known kinds: issue, patch, document, conversation")
            })?;
            if kinds.contains(&kind) {
                return Err(self.err(
                    tok.pos,
                    tok.len,
                    format!("duplicate kind '{word}' in kind= list"),
                ));
            }
            kinds.push(kind);
            self.bump();
            match self.peek() {
                Some(t) if matches!(t.kind, TokenKind::Comma) => {
                    self.bump();
                    continue;
                }
                _ => break,
            }
        }
        Ok(Stage::Kind(kinds))
    }

    fn parse_relation_stage(&mut self, name: &str, name_tok: &Token) -> Result<Stage, ParseError> {
        let mut rel: Option<RelType> = None;
        let mut rel_span: Option<(usize, usize)> = None;
        let mut transitive = false;
        let mut transitive_span: Option<(usize, usize)> = None;
        let mut exclusive = false;

        while let Some(tok) = self.peek().cloned() {
            match &tok.kind {
                TokenKind::Pipe => break,
                TokenKind::Word(w) => {
                    if w == "transitive" {
                        if transitive {
                            return Err(self.err(
                                tok.pos,
                                tok.len,
                                "duplicate argument 'transitive'",
                            ));
                        }
                        transitive = true;
                        transitive_span = Some((tok.pos, tok.len));
                        self.bump();
                    } else if w == "exclusive" {
                        if exclusive {
                            return Err(self.err(
                                tok.pos,
                                tok.len,
                                "duplicate argument 'exclusive'",
                            ));
                        }
                        exclusive = true;
                        self.bump();
                    } else if w == "rel" {
                        // Expect `=` then RELTYPE.
                        let kw_tok = tok.clone();
                        self.bump();
                        let eq_tok = match self.peek() {
                            Some(t) => t.clone(),
                            None => {
                                return Err(self.err(
                                    self.eof_position(),
                                    1,
                                    "expected '=' after 'rel'",
                                ));
                            }
                        };
                        if !matches!(eq_tok.kind, TokenKind::Eq) {
                            return Err(self.err(
                                eq_tok.pos,
                                eq_tok.len,
                                "expected '=' after 'rel'",
                            ));
                        }
                        self.bump();
                        let rel_tok = match self.peek() {
                            Some(t) => t.clone(),
                            None => {
                                return Err(self.err(
                                    self.eof_position(),
                                    1,
                                    "expected rel type after 'rel='",
                                ));
                            }
                        };
                        let rel_word = match &rel_tok.kind {
                            TokenKind::Word(w) => w.clone(),
                            _ => {
                                return Err(self.err(
                                    rel_tok.pos,
                                    rel_tok.len,
                                    "expected rel type after 'rel='",
                                ));
                            }
                        };
                        let parsed = RelType::from_str(&rel_word).map_err(|_| {
                            let mut e = self.err(
                                rel_tok.pos,
                                rel_tok.len,
                                format!("unknown rel type '{rel_word}'"),
                            );
                            if let Some(hint) = suggest_rel_type(&rel_word) {
                                e = e.with_hint(format!("did you mean '{hint}'?"));
                            } else {
                                e = e.with_hint(
                                    "known rel types: child-of, blocked-on, has-patch, has-document, refers-to",
                                );
                            }
                            e
                        })?;
                        if rel.is_some() {
                            return Err(self.err(
                                kw_tok.pos,
                                rel_tok.pos + rel_tok.len - kw_tok.pos,
                                "duplicate argument 'rel='",
                            ));
                        }
                        rel = Some(parsed);
                        rel_span = Some((kw_tok.pos, rel_tok.pos + rel_tok.len - kw_tok.pos));
                        self.bump();
                    } else if w == "kind" {
                        return Err(self
                            .err(
                                tok.pos,
                                tok.len,
                                format!("unexpected filter stage 'kind=' inside '{name}' stage"),
                            )
                            .with_hint("use '|' to separate stages: '... | kind=patch'"));
                    } else {
                        let mut e = self.err(
                            tok.pos,
                            tok.len,
                            format!("unknown argument '{w}' in '{name}' stage"),
                        );
                        if STAGE_NAMES.iter().any(|n| n == w) {
                            e = e.with_hint(format!(
                                "did you forget a '|' before '{w}'? stages are pipe-separated"
                            ));
                        } else if let Some(hint) = suggest_stage_name(w) {
                            e = e.with_hint(format!(
                                "did you forget a '|'? '{w}' looks like '{hint}'"
                            ));
                        }
                        return Err(e);
                    }
                }
                TokenKind::Comma | TokenKind::Eq => {
                    return Err(self.err(tok.pos, tok.len, "unexpected punctuation in stage"));
                }
            }
        }

        // Apply per-stage validation rules.
        match name {
            "parents" => {
                if transitive && rel.is_none() {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self
                        .err(pos, len, "'transitive' on 'parents' requires 'rel='")
                        .with_hint("add a rel filter: 'parents rel=child-of transitive'"));
                }
                Ok(Stage::Parents {
                    rel,
                    transitive,
                    exclusive,
                })
            }
            "children" => {
                if transitive && rel.is_none() {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self
                        .err(pos, len, "'transitive' on 'children' requires 'rel='")
                        .with_hint("add a rel filter: 'children rel=child-of transitive'"));
                }
                Ok(Stage::Children {
                    rel,
                    transitive,
                    exclusive,
                })
            }
            "neighbors" => {
                if transitive {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self
                        .err(
                            pos,
                            len,
                            "'transitive' is not supported on 'neighbors'",
                        )
                        .with_hint(
                            "neighbors cannot do transitive closure; use 'parents' or 'children' with a 'rel=' filter",
                        ));
                }
                Ok(Stage::Neighbors { rel, exclusive })
            }
            "ancestors" => {
                if transitive {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self
                        .err(
                            pos,
                            len,
                            "'transitive' is implicit on 'ancestors'",
                        )
                        .with_hint(
                            "drop 'transitive' (use 'parents rel=… transitive' for the explicit form)",
                        ));
                }
                let rel = rel.ok_or_else(|| {
                    self.err(name_tok.pos, name_tok.len, "'ancestors' requires 'rel='")
                        .with_hint("e.g., 'ancestors rel=child-of'")
                })?;
                Ok(Stage::Ancestors { rel, exclusive })
            }
            "descendants" => {
                if transitive {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self
                        .err(
                            pos,
                            len,
                            "'transitive' is implicit on 'descendants'",
                        )
                        .with_hint(
                            "drop 'transitive' (use 'children rel=… transitive' for the explicit form)",
                        ));
                }
                let rel = rel.ok_or_else(|| {
                    self.err(name_tok.pos, name_tok.len, "'descendants' requires 'rel='")
                        .with_hint("e.g., 'descendants rel=child-of'")
                })?;
                Ok(Stage::Descendants { rel, exclusive })
            }
            "scope" => {
                if exclusive {
                    return Err(self
                        .err(
                            name_tok.pos,
                            name_tok.len,
                            "'exclusive' is not accepted on 'scope'",
                        )
                        .with_hint("scope is inherently inclusive"));
                }
                if transitive {
                    let (pos, len) = transitive_span.expect("set when transitive is true");
                    return Err(self.err(pos, len, "'transitive' is not accepted on 'scope'"));
                }
                if let Some((pos, len)) = rel_span {
                    return Err(self.err(pos, len, "'rel=' is not accepted on 'scope'"));
                }
                Ok(Stage::Scope)
            }
            _ => unreachable!("validated above"),
        }
    }
}

const STAGE_NAMES: &[&str] = &[
    "parents",
    "children",
    "neighbors",
    "ancestors",
    "descendants",
    "scope",
];

const REL_TYPES: &[&str] = &[
    "child-of",
    "blocked-on",
    "has-patch",
    "has-document",
    "refers-to",
];

fn parse_hydra_id(input: &str, tok: &Token) -> Result<HydraId, ParseError> {
    let s = match &tok.kind {
        TokenKind::Word(w) => w.clone(),
        _ => {
            return Err(ParseError::new(
                input,
                tok.pos,
                tok.len,
                "expected source id",
            ));
        }
    };
    HydraId::try_from(s.clone())
        .map_err(|e| ParseError::new(input, tok.pos, tok.len, format!("invalid source id: {e}")))
}

// -- Suggestions (Levenshtein) ------------------------------------------

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

fn closest_match<'a>(target: &str, candidates: &[&'a str], max_dist: usize) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (*c, levenshtein(target, c)))
        .filter(|(_, d)| *d <= max_dist)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

/// Common typos / alternate spellings that don't fall within Levenshtein ≤ 2
/// of the canonical name. The table is consulted first; the Levenshtein
/// search runs as a fallback.
const STAGE_ALIASES: &[(&str, &str)] = &[
    ("kids", "children"),
    ("child", "children"),
    ("kid", "children"),
    ("neighbour", "neighbors"),
    ("neighbours", "neighbors"),
    ("neighbor", "neighbors"),
    ("ancestor", "ancestors"),
    ("descendant", "descendants"),
];

fn suggest_stage_name(word: &str) -> Option<&'static str> {
    if let Some((_, canonical)) = STAGE_ALIASES.iter().find(|(k, _)| *k == word) {
        return Some(*canonical);
    }
    closest_match(word, STAGE_NAMES, 2)
}

fn suggest_rel_type(word: &str) -> Option<&'static str> {
    // Always suggest if the underscore-equivalent matches.
    let normalized = word.replace('_', "-");
    if REL_TYPES.iter().any(|r| **r == normalized) {
        return REL_TYPES.iter().find(|r| ***r == normalized).copied();
    }
    closest_match(word, REL_TYPES, 2)
}

// -- Lowering ------------------------------------------------------------

impl Query {
    /// Collapse parse-time sugar (`ancestors` / `descendants`) into the
    /// canonical `parents` / `children` form and produce the resolver-side
    /// instruction list.
    ///
    /// The lowering does **not** issue HTTP calls. It produces a
    /// [`LoweredQuery`] that the resolver (in the CLI crate) walks against an
    /// evolving vertex set, applying the inclusive-by-default contract per
    /// stage.
    pub fn lower(self) -> LoweredQuery {
        let Query { source, stages } = self;
        let mut out: Vec<LoweredStage> = Vec::with_capacity(stages.len());
        for stage in stages {
            let lowered = match stage {
                Stage::Parents {
                    rel,
                    transitive,
                    exclusive,
                } => LoweredStage::Relations(RelationsQuery {
                    direction: Direction::Target,
                    rel,
                    transitive,
                    exclusive,
                }),
                Stage::Children {
                    rel,
                    transitive,
                    exclusive,
                } => LoweredStage::Relations(RelationsQuery {
                    direction: Direction::Source,
                    rel,
                    transitive,
                    exclusive,
                }),
                Stage::Neighbors { rel, exclusive } => LoweredStage::Relations(RelationsQuery {
                    direction: Direction::Object,
                    rel,
                    transitive: false,
                    exclusive,
                }),
                Stage::Ancestors { rel, exclusive } => LoweredStage::Relations(RelationsQuery {
                    direction: Direction::Target,
                    rel: Some(rel),
                    transitive: true,
                    exclusive,
                }),
                Stage::Descendants { rel, exclusive } => LoweredStage::Relations(RelationsQuery {
                    direction: Direction::Source,
                    rel: Some(rel),
                    transitive: true,
                    exclusive,
                }),
                Stage::Scope => LoweredStage::Scope,
                Stage::Kind(ks) => LoweredStage::Kind(ks),
            };

            // Merge consecutive Kind stages by intersection. Two adjacent
            // `kind=` filters in the parsed query collapse to one filter that
            // keeps only kinds present in both lists.
            if let (Some(LoweredStage::Kind(prev)), LoweredStage::Kind(next)) =
                (out.last_mut(), &lowered)
            {
                let merged: Vec<ObjectKind> =
                    prev.iter().copied().filter(|k| next.contains(k)).collect();
                *prev = merged;
            } else {
                out.push(lowered);
            }
        }
        LoweredQuery {
            source,
            stages: out,
        }
    }
}

// -- Display (round-trip) ------------------------------------------------

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stage::Parents {
                rel,
                transitive,
                exclusive,
            } => write_relation_stage(f, "parents", *rel, *transitive, *exclusive),
            Stage::Children {
                rel,
                transitive,
                exclusive,
            } => write_relation_stage(f, "children", *rel, *transitive, *exclusive),
            Stage::Neighbors { rel, exclusive } => {
                write_relation_stage(f, "neighbors", *rel, false, *exclusive)
            }
            Stage::Ancestors { rel, exclusive } => {
                write_relation_stage(f, "ancestors", Some(*rel), false, *exclusive)
            }
            Stage::Descendants { rel, exclusive } => {
                write_relation_stage(f, "descendants", Some(*rel), false, *exclusive)
            }
            Stage::Scope => f.write_str("scope"),
            Stage::Kind(kinds) => {
                f.write_str("kind=")?;
                let mut first = true;
                for k in kinds {
                    if !first {
                        f.write_str(",")?;
                    }
                    first = false;
                    f.write_str(k.as_str())?;
                }
                Ok(())
            }
        }
    }
}

fn write_relation_stage(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    rel: Option<RelType>,
    transitive: bool,
    exclusive: bool,
) -> fmt::Result {
    f.write_str(name)?;
    if let Some(r) = rel {
        write!(f, " rel={r}")?;
    }
    if transitive {
        f.write_str(" transitive")?;
    }
    if exclusive {
        f.write_str(" exclusive")?;
    }
    Ok(())
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for id in &self.source {
            if !first {
                f.write_str(",")?;
            }
            first = false;
            f.write_str(id.as_ref())?;
        }
        for stage in &self.stages {
            write!(f, " | {stage}")?;
        }
        Ok(())
    }
}

// -- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> HydraId {
        HydraId::try_from(s.to_string()).expect("valid id in test")
    }

    // -- Parses (one per worked-example row, plus CSV-source forms) ----

    #[test]
    fn parses_bare_id() {
        let q = parse("i-abcdef").unwrap();
        assert_eq!(q.source, vec![id("i-abcdef")]);
        assert!(q.stages.is_empty());
    }

    #[test]
    fn parses_bare_id_csv_source() {
        let q = parse("i-abcd, i-defg").unwrap();
        assert_eq!(q.source, vec![id("i-abcd"), id("i-defg")]);
    }

    #[test]
    fn parses_csv_source_deduplicates() {
        let q = parse("i-abcd,i-defg,i-abcd").unwrap();
        assert_eq!(q.source, vec![id("i-abcd"), id("i-defg")]);
    }

    #[test]
    fn parses_neighbors_default() {
        let q = parse("i-abcd | neighbors").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Neighbors {
                rel: None,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_neighbors_exclusive() {
        let q = parse("i-abcd | neighbors exclusive").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Neighbors {
                rel: None,
                exclusive: true,
            }]
        );
    }

    #[test]
    fn parses_neighbors_with_rel() {
        let q = parse("i-abcd | neighbors rel=refers-to").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Neighbors {
                rel: Some(RelType::RefersTo),
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_children_with_rel() {
        let q = parse("i-abcd | children rel=child-of").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Children {
                rel: Some(RelType::ChildOf),
                transitive: false,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_children_with_rel_transitive() {
        let q = parse("i-abcd | children rel=child-of transitive").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Children {
                rel: Some(RelType::ChildOf),
                transitive: true,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_children_with_rel_transitive_exclusive() {
        let q = parse("i-abcd | children rel=child-of transitive exclusive").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Children {
                rel: Some(RelType::ChildOf),
                transitive: true,
                exclusive: true,
            }]
        );
    }

    #[test]
    fn parses_descendants_sugar() {
        let q = parse("i-abcd | descendants rel=child-of").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Descendants {
                rel: RelType::ChildOf,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_descendants_exclusive() {
        let q = parse("i-abcd | descendants rel=child-of exclusive").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Descendants {
                rel: RelType::ChildOf,
                exclusive: true,
            }]
        );
    }

    #[test]
    fn parses_ancestors_sugar() {
        let q = parse("i-abcd | ancestors rel=child-of").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Ancestors {
                rel: RelType::ChildOf,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_scope() {
        let q = parse("i-abcd | scope").unwrap();
        assert_eq!(q.stages, vec![Stage::Scope]);
    }

    #[test]
    fn parses_scope_kind_filter() {
        let q = parse("i-abcd | scope | kind=patch").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Scope, Stage::Kind(vec![ObjectKind::Patch])]
        );
    }

    #[test]
    fn parses_long_pipeline() {
        let q = parse("i-abcd | scope | kind=issue | children rel=has-patch").unwrap();
        assert_eq!(
            q.stages,
            vec![
                Stage::Scope,
                Stage::Kind(vec![ObjectKind::Issue]),
                Stage::Children {
                    rel: Some(RelType::HasPatch),
                    transitive: false,
                    exclusive: false,
                },
            ]
        );
    }

    #[test]
    fn parses_chained_relation_stages() {
        let q = parse("i-abcd | neighbors rel=refers-to | parents rel=child-of").unwrap();
        assert_eq!(
            q.stages,
            vec![
                Stage::Neighbors {
                    rel: Some(RelType::RefersTo),
                    exclusive: false,
                },
                Stage::Parents {
                    rel: Some(RelType::ChildOf),
                    transitive: false,
                    exclusive: false,
                },
            ]
        );
    }

    #[test]
    fn parses_csv_source_into_scope() {
        let q = parse("i-abcd, i-defg | scope").unwrap();
        assert_eq!(q.source, vec![id("i-abcd"), id("i-defg")]);
        assert_eq!(q.stages, vec![Stage::Scope]);
    }

    #[test]
    fn parses_patch_parents() {
        let q = parse("p-deadbe | parents").unwrap();
        assert_eq!(q.source, vec![id("p-deadbe")]);
        assert_eq!(
            q.stages,
            vec![Stage::Parents {
                rel: None,
                transitive: false,
                exclusive: false,
            }]
        );
    }

    #[test]
    fn parses_patch_parents_exclusive() {
        let q = parse("p-deadbe | parents exclusive").unwrap();
        assert_eq!(
            q.stages,
            vec![Stage::Parents {
                rel: None,
                transitive: false,
                exclusive: true,
            }]
        );
    }

    #[test]
    fn parses_kind_csv_list() {
        let q = parse("i-abcd | scope | kind=patch,document").unwrap();
        assert_eq!(
            q.stages,
            vec![
                Stage::Scope,
                Stage::Kind(vec![ObjectKind::Patch, ObjectKind::Document]),
            ]
        );
    }

    #[test]
    fn parses_free_arg_order() {
        let a = parse("i-abcd | children exclusive transitive rel=child-of").unwrap();
        let b = parse("i-abcd | children rel=child-of transitive exclusive").unwrap();
        assert_eq!(a, b);
    }

    // -- Fails to parse, with caret + hint ------------------------------

    #[test]
    fn fails_transitive_on_neighbors() {
        let err = parse("i-abcd | neighbors transitive").unwrap_err();
        assert_eq!(err.message, "'transitive' is not supported on 'neighbors'");
        let expected = "\
error: 'transitive' is not supported on 'neighbors' at position 19
  i-abcd | neighbors transitive
                     ^^^^^^^^^^
hint: neighbors cannot do transitive closure; use 'parents' or 'children' with a 'rel=' filter";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_transitive_without_rel_parents() {
        let err = parse("i-abcd | parents transitive").unwrap_err();
        assert_eq!(err.message, "'transitive' on 'parents' requires 'rel='");
        let expected = "\
error: 'transitive' on 'parents' requires 'rel=' at position 17
  i-abcd | parents transitive
                   ^^^^^^^^^^
hint: add a rel filter: 'parents rel=child-of transitive'";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_transitive_without_rel_children() {
        let err = parse("i-abcd | children transitive").unwrap_err();
        assert_eq!(err.message, "'transitive' on 'children' requires 'rel='");
    }

    #[test]
    fn fails_exclusive_on_scope() {
        let err = parse("i-abcd | scope exclusive").unwrap_err();
        assert_eq!(err.message, "'exclusive' is not accepted on 'scope'");
        let expected = "\
error: 'exclusive' is not accepted on 'scope' at position 9
  i-abcd | scope exclusive
           ^^^^^
hint: scope is inherently inclusive";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_stage_without_source() {
        let err = parse("neighbors").unwrap_err();
        assert_eq!(err.message, "stage 'neighbors' has no preceding source");
        let expected = "\
error: stage 'neighbors' has no preceding source at position 0
  neighbors
  ^^^^^^^^^
hint: queries must start with a source id (e.g., 'i-abcdef | scope')";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_scope_with_rel() {
        let err = parse("i-abcd | scope rel=child-of").unwrap_err();
        assert_eq!(err.message, "'rel=' is not accepted on 'scope'");
    }

    #[test]
    fn fails_explicit_transitive_on_ancestors() {
        let err = parse("i-abcd | ancestors rel=child-of transitive").unwrap_err();
        assert_eq!(err.message, "'transitive' is implicit on 'ancestors'");
        assert!(err.hint.as_deref().unwrap().contains("parents"));
    }

    #[test]
    fn fails_explicit_transitive_on_descendants() {
        let err = parse("i-abcd | descendants rel=child-of transitive").unwrap_err();
        assert_eq!(err.message, "'transitive' is implicit on 'descendants'");
        assert!(err.hint.as_deref().unwrap().contains("children"));
    }

    #[test]
    fn fails_ancestors_without_rel() {
        let err = parse("i-abcd | ancestors").unwrap_err();
        assert_eq!(err.message, "'ancestors' requires 'rel='");
        assert!(err
            .hint
            .as_deref()
            .unwrap()
            .contains("ancestors rel=child-of"));
    }

    #[test]
    fn fails_descendants_without_rel() {
        let err = parse("i-abcd | descendants").unwrap_err();
        assert_eq!(err.message, "'descendants' requires 'rel='");
    }

    #[test]
    fn fails_duplicate_exclusive() {
        let err = parse("i-abcd | children exclusive exclusive").unwrap_err();
        assert_eq!(err.message, "duplicate argument 'exclusive'");
        let expected = "\
error: duplicate argument 'exclusive' at position 28
  i-abcd | children exclusive exclusive
                              ^^^^^^^^^";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_duplicate_transitive() {
        let err = parse("i-abcd | children rel=child-of transitive transitive").unwrap_err();
        assert_eq!(err.message, "duplicate argument 'transitive'");
    }

    #[test]
    fn fails_duplicate_rel() {
        let err = parse("i-abcd | children rel=child-of rel=has-patch").unwrap_err();
        assert_eq!(err.message, "duplicate argument 'rel='");
    }

    #[test]
    fn fails_unknown_stage_with_levenshtein_kids() {
        let err = parse("i-abcd | kids").unwrap_err();
        assert_eq!(err.message, "unknown stage 'kids'");
        let expected = "\
error: unknown stage 'kids' at position 9
  i-abcd | kids
           ^^^^
hint: did you mean 'children'?";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn fails_unknown_stage_with_levenshtein_neighbour() {
        let err = parse("i-abcd | neighbour").unwrap_err();
        assert_eq!(err.message, "unknown stage 'neighbour'");
        assert_eq!(err.hint.as_deref(), Some("did you mean 'neighbors'?"));
    }

    #[test]
    fn fails_unknown_stage_with_levenshtein_parent() {
        let err = parse("i-abcd | parent").unwrap_err();
        assert_eq!(err.message, "unknown stage 'parent'");
        assert_eq!(err.hint.as_deref(), Some("did you mean 'parents'?"));
    }

    #[test]
    fn fails_unknown_rel_type_underscore_form() {
        let err = parse("i-abcd | neighbors rel=refers_to").unwrap_err();
        assert_eq!(err.message, "unknown rel type 'refers_to'");
        assert_eq!(err.hint.as_deref(), Some("did you mean 'refers-to'?"));
    }

    #[test]
    fn fails_unknown_kind() {
        let err = parse("i-abcd | kind=widget").unwrap_err();
        assert_eq!(err.message, "unknown kind 'widget'");
        assert!(err.hint.as_deref().unwrap().contains("issue"));
    }

    #[test]
    fn fails_kind_without_eq() {
        let err = parse("i-abcd | kind").unwrap_err();
        assert!(err.message.contains("expected '='"));
    }

    #[test]
    fn fails_empty_input() {
        let err = parse("").unwrap_err();
        assert!(err.message.contains("expected source id"));
    }

    #[test]
    fn fails_invalid_source_id() {
        let err = parse("not-an-id").unwrap_err();
        assert!(err.message.contains("invalid source id"));
    }

    // -- Lowering identities --------------------------------------------

    #[test]
    fn lower_ancestors_collapses_to_parents_transitive() {
        let q = parse("i-abcd | ancestors rel=child-of").unwrap();
        let lq = q.lower();
        assert_eq!(lq.source, vec![id("i-abcd")]);
        assert_eq!(
            lq.stages,
            vec![LoweredStage::Relations(RelationsQuery {
                direction: Direction::Target,
                rel: Some(RelType::ChildOf),
                transitive: true,
                exclusive: false,
            })]
        );
    }

    #[test]
    fn lower_descendants_collapses_to_children_transitive() {
        let q = parse("i-abcd | descendants rel=child-of").unwrap();
        let lq = q.lower();
        assert_eq!(
            lq.stages,
            vec![LoweredStage::Relations(RelationsQuery {
                direction: Direction::Source,
                rel: Some(RelType::ChildOf),
                transitive: true,
                exclusive: false,
            })]
        );
    }

    // Acceptance criterion: `RelationsQuery.exclusive` exercised in at
    // least three lowering tests (default-false, explicit-true,
    // ancestors-with-exclusive). The three tests below satisfy that.

    #[test]
    fn lower_exclusive_default_false() {
        let q = parse("i-abcd | children rel=child-of").unwrap();
        let lq = q.lower();
        let LoweredStage::Relations(r) = &lq.stages[0] else {
            panic!("expected Relations");
        };
        assert!(!r.exclusive);
    }

    #[test]
    fn lower_exclusive_explicit_true() {
        let q = parse("i-abcd | children rel=child-of exclusive").unwrap();
        let lq = q.lower();
        let LoweredStage::Relations(r) = &lq.stages[0] else {
            panic!("expected Relations");
        };
        assert!(r.exclusive);
    }

    #[test]
    fn lower_ancestors_with_exclusive_propagates() {
        let q = parse("i-abcd | ancestors rel=child-of exclusive").unwrap();
        let lq = q.lower();
        let LoweredStage::Relations(r) = &lq.stages[0] else {
            panic!("expected Relations");
        };
        assert!(r.exclusive);
        assert!(r.transitive);
        assert_eq!(r.direction, Direction::Target);
    }

    #[test]
    fn lower_neighbors_single_element_source_doc_comment() {
        // Resolver-side concern: with a single-element source, the
        // resolver should map Object direction to `object_id` singular
        // (not the new `object_ids` plural). The parser cannot express
        // this in the lowered struct; the `LoweredStage::Relations` doc
        // comment flags this for resolver implementers.
        let q = parse("i-abcd | neighbors").unwrap();
        let lq = q.lower();
        assert_eq!(lq.source.len(), 1);
        let LoweredStage::Relations(r) = &lq.stages[0] else {
            panic!("expected Relations");
        };
        assert_eq!(r.direction, Direction::Object);
    }

    #[test]
    fn lower_kind_filter_intersection() {
        let q = parse("i-abcd | kind=patch | kind=patch,document").unwrap();
        let lq = q.lower();
        // Two consecutive Kind stages collapse into one with the
        // intersection of the kind lists.
        assert_eq!(lq.stages, vec![LoweredStage::Kind(vec![ObjectKind::Patch])]);
    }

    #[test]
    fn lower_kind_filter_intersection_empty() {
        let q = parse("i-abcd | kind=patch | kind=document").unwrap();
        let lq = q.lower();
        // No overlap → empty intersection.
        assert_eq!(lq.stages, vec![LoweredStage::Kind(vec![])]);
    }

    #[test]
    fn lower_neighbors_lowers_to_object_direction() {
        let q = parse("i-abcd | neighbors rel=refers-to").unwrap();
        let lq = q.lower();
        assert_eq!(
            lq.stages,
            vec![LoweredStage::Relations(RelationsQuery {
                direction: Direction::Object,
                rel: Some(RelType::RefersTo),
                transitive: false,
                exclusive: false,
            })]
        );
    }

    #[test]
    fn lower_scope_passes_through() {
        let q = parse("i-abcd | scope | kind=patch").unwrap();
        let lq = q.lower();
        assert_eq!(
            lq.stages,
            vec![
                LoweredStage::Scope,
                LoweredStage::Kind(vec![ObjectKind::Patch])
            ]
        );
    }

    // -- Display round-trip --------------------------------------------

    #[test]
    fn display_roundtrip_bare_id() {
        let q = parse("i-abcd").unwrap();
        let s = format!("{q}");
        assert_eq!(s, "i-abcd");
        assert_eq!(parse(&s).unwrap(), q);
    }

    #[test]
    fn display_roundtrip_neighbors() {
        let q = parse("i-abcd | neighbors rel=refers-to").unwrap();
        let s = format!("{q}");
        assert_eq!(s, "i-abcd | neighbors rel=refers-to");
        assert_eq!(parse(&s).unwrap(), q);
    }

    #[test]
    fn display_roundtrip_normalizes_arg_order() {
        // Display canonicalizes arg order to `rel=` → `transitive` →
        // `exclusive`. The round-trip parses to the same AST regardless of
        // the user's chosen order in the input.
        let q1 = parse("i-abcd | children exclusive transitive rel=child-of").unwrap();
        let s = format!("{q1}");
        assert_eq!(s, "i-abcd | children rel=child-of transitive exclusive");
        let q2 = parse(&s).unwrap();
        assert_eq!(q1, q2);
    }

    #[test]
    fn display_roundtrip_long_pipeline() {
        let q = parse("i-abcd,i-defg | scope | kind=patch,document").unwrap();
        let s = format!("{q}");
        assert_eq!(s, "i-abcd,i-defg | scope | kind=patch,document");
        assert_eq!(parse(&s).unwrap(), q);
    }

    #[test]
    fn display_roundtrip_descendants() {
        let q = parse("i-abcd | descendants rel=child-of exclusive").unwrap();
        let s = format!("{q}");
        assert_eq!(s, "i-abcd | descendants rel=child-of exclusive");
        assert_eq!(parse(&s).unwrap(), q);
    }

    // -- FromStr / Display for type strings ----------------------------

    #[test]
    fn rel_type_roundtrip() {
        for s in REL_TYPES {
            let r: RelType = s.parse().unwrap();
            assert_eq!(r.as_str(), *s);
            assert_eq!(format!("{r}"), *s);
        }
    }

    #[test]
    fn rel_type_unknown_errors() {
        let err = "child_of".parse::<RelType>().unwrap_err();
        assert_eq!(err.to_string(), "unknown rel type 'child_of'");
    }

    #[test]
    fn object_kind_roundtrip() {
        for s in ["issue", "patch", "document", "conversation"] {
            let k: ObjectKind = s.parse().unwrap();
            assert_eq!(k.as_str(), s);
            assert_eq!(format!("{k}"), s);
        }
    }
}
