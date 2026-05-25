//! Query DSL parser for `hydra graph search`/`diff`/`log`.
//!
//! Authoritative spec: `/designs/hydra-graph-query-language.md` in the doc
//! store. This module implements the AST, recursive-descent parser, and
//! lowering machinery only; the CLI cutovers ride in follow-up PRs.
//!
//! Grammar (summary):
//!
//! ```text
//! QUERY    := atom (PIPE filter)*
//! atom     := id | call
//! call     := name '(' arglist? ')'
//! name     := 'parents' | 'children' | 'neighbors'
//!           | 'ancestors' | 'descendants' | 'scope'
//! arglist  := arg (',' arg)*
//! arg      := id | kw '=' value | 'transitive'
//! kw       := 'rel'
//! filter   := 'kind' '=' kind (',' kind)*
//! ```
//!
//! See the design doc for the atom-by-atom semantics, the lowering table,
//! and the alias-collapse rules used in [`Query::lower`].

use std::{fmt, str::FromStr};

use crate::HydraId;
use crate::graph::ObjectKind;

/// AST root: an atom plus zero-or-more pipe filters.
///
/// See `/designs/hydra-graph-query-language.md` for the full grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub atom: Atom,
    pub filters: Vec<Filter>,
}

/// Node-selection atom.
///
/// `Ancestors` and `Descendants` are parse-time-only variants preserved so
/// the AST mirrors the source string (useful for error messages and future
/// tooling); they collapse to `Parents { transitive: true }` /
/// `Children { transitive: true }` inside [`Query::lower`], so the resolver
/// only needs to handle the canonical atoms.
///
/// See `/designs/hydra-graph-query-language.md` for the per-atom semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    /// A bare id (`i-abc123`). Lowers to a "skip relations, hydrate
    /// directly" marker — there is no reserved `object` atom.
    BareId(HydraId),
    /// `parents(<id> [, rel=<R>] [, transitive])`
    Parents {
        id: HydraId,
        rel: Option<RelType>,
        transitive: bool,
    },
    /// `children(<id> [, rel=<R>] [, transitive])`
    Children {
        id: HydraId,
        rel: Option<RelType>,
        transitive: bool,
    },
    /// `neighbors(<id> [, rel=<R>])` — no `transitive` (server constraint).
    Neighbors { id: HydraId, rel: Option<RelType> },
    /// `ancestors(<id>, rel=<R>)` — sugar for `parents(<id>, rel=<R>, transitive)`.
    Ancestors { id: HydraId, rel: RelType },
    /// `descendants(<id>, rel=<R>)` — sugar for `children(<id>, rel=<R>, transitive)`.
    Descendants { id: HydraId, rel: RelType },
    /// `scope(<id>)` — the existing 3-call scope algorithm's input.
    Scope(HydraId),
}

/// Pipe filter. Currently only `Kind`; the enum is a placeholder for
/// future post-pipe filters (`| status=...`, etc.).
///
/// See `/designs/hydra-graph-query-language.md` for the filter catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// `| kind=<list>` — keeps only nodes whose kind appears in the list.
    Kind(Vec<ObjectKind>),
}

/// The five relation types accepted by the DSL. Matches the server's
/// existing rel-type vocabulary verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelType {
    ChildOf,
    BlockedOn,
    HasPatch,
    HasDocument,
    RefersTo,
}

impl RelType {
    /// The on-wire string form (`child-of`, etc.) used in
    /// `ListRelationsRequest::rel_type`.
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

/// Error returned when a string does not match any [`RelType`] variant.
///
/// The `Display` impl spells out the accepted values so the DSL parser can
/// surface it as a hint without duplicating the value list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseRelTypeError;

impl fmt::Display for ParseRelTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected one of: child-of, blocked-on, has-patch, has-document, refers-to")
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
            _ => Err(ParseRelTypeError),
        }
    }
}

/// The lowered representation consumed by the CLI resolver.
///
/// Produced by [`Query::lower`]; PRs 2–4 will translate the
/// [`LoweredAtom::Relations`] variant into the corresponding
/// `ListRelationsRequest` and the [`LoweredAtom::Scope`] variant into the
/// existing 3-call scope algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredQuery {
    pub atom: LoweredAtom,
    /// Post-hydration kind filter. Empty ⇒ no filter applied. Multiple
    /// pipe filters intersect (per the design doc).
    pub kind_filter: Option<Vec<ObjectKind>>,
}

/// Lowered atom: every alias has been collapsed to its canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweredAtom {
    /// Bare-id fast path: skip `/v1/relations`, hydrate the id directly
    /// via `GET /v1/<kind>/<id>` (kind inferred from id prefix).
    BareId(HydraId),
    /// Lowers to a single `GET /v1/relations?...` call. Fields map 1-1
    /// to `ListRelationsRequest`.
    Relations(RelationsQuery),
    /// Lowers to the existing 3-call scope algorithm (`resolve_scope_node_ids`).
    Scope(HydraId),
}

/// `ListRelationsRequest`-equivalent fields. The CLI resolver fills the
/// corresponding `ListRelationsRequest`; this struct deliberately mirrors
/// the field set so the lowering is mechanical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelationsQuery {
    pub source_id: Option<HydraId>,
    pub target_id: Option<HydraId>,
    pub object_id: Option<HydraId>,
    pub rel_type: Option<RelType>,
    pub transitive: bool,
}

/// A parse failure with the offending span. The [`fmt::Display`] impl
/// renders a caret diagram and (optionally) a "did you mean" hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The full input string the parser was given.
    pub input: String,
    /// Byte-offset span `[start, end)` of the offending token. `end == start`
    /// is permitted; rendering pads it to a single caret.
    pub span: (usize, usize),
    /// The primary message (no trailing punctuation, no caret diagram).
    pub message: String,
    /// Optional "did you mean ..." / "use ... instead" hint.
    pub hint: Option<String>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {} at position {}", self.message, self.span.0)?;
        writeln!(f, "  {}", self.input)?;
        let pad: String = " ".repeat(self.span.0);
        let len = self.span.1.saturating_sub(self.span.0).max(1);
        let carets: String = "^".repeat(len);
        write!(f, "  {pad}{carets}")?;
        if let Some(hint) = &self.hint {
            write!(f, "\nhint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

const RESERVED_NAMES: &[&str] = &[
    "parents",
    "children",
    "neighbors",
    "ancestors",
    "descendants",
    "scope",
];

/// Canned "did you mean" mappings for common typos that fall outside
/// Levenshtein distance 2 (per the design doc).
const TYPO_ALIASES: &[(&str, &str)] = &[
    ("kids", "children"),
    ("child", "children"),
    ("parent", "parents"),
    ("ancestor", "ancestors"),
    ("descendant", "descendants"),
    ("neighbour", "neighbors"),
    ("neighbor", "neighbors"),
];

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn suggest_atom_name(input: &str) -> Option<&'static str> {
    for (alias, target) in TYPO_ALIASES {
        if *alias == input {
            return Some(target);
        }
    }
    let mut best: Option<(&'static str, usize)> = None;
    for &name in RESERVED_NAMES {
        let d = levenshtein(input, name);
        if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((name, d));
        }
    }
    best.map(|(name, _)| name)
}

/// Build a "did you mean" hint for the unknown atom `name`. `id` is the
/// first id-looking arg of the call, if the parser could peek one.
fn unknown_atom_hint(name: &str, id: Option<&str>) -> Option<String> {
    if name == "object" {
        return Some(match id {
            Some(id) => format!("'object' is not a reserved atom; use the bare id form: {id}"),
            None => {
                "'object' is not a reserved atom; use the bare id form (e.g. 'i-abc123') directly"
                    .to_string()
            }
        });
    }
    let suggestion = suggest_atom_name(name)?;
    Some(match id {
        Some(id) => format!("did you mean '{suggestion}({id})'?"),
        None => format!("did you mean '{suggestion}'?"),
    })
}

impl Query {
    /// Parse a query string into the AST. See the module docs for the
    /// grammar; see [`ParseError`] for the error shape.
    pub fn parse(input: &str) -> Result<Query, ParseError> {
        let mut parser = Parser::new(input);
        parser.skip_ws();
        if parser.at_eof() {
            return Err(parser.err((0, 0), "empty query", None));
        }
        let atom = parser.parse_atom()?;
        let mut filters = Vec::new();
        loop {
            parser.skip_ws();
            if parser.peek() != Some('|') {
                break;
            }
            let pipe_pos = parser.pos;
            parser.bump();
            parser.skip_ws();
            if parser.peek() == Some('|') {
                let end = parser.pos + 1;
                return Err(parser.err((pipe_pos, end), "double pipe '||' is not allowed", None));
            }
            filters.push(parser.parse_filter()?);
        }
        parser.skip_ws();
        if !parser.at_eof() {
            let end = input.len();
            return Err(parser.err((parser.pos, end), "trailing input after query", None));
        }
        Ok(Query { atom, filters })
    }

    /// Lower the AST into a representation the CLI resolver can consume.
    ///
    /// Collapses `Ancestors`/`Descendants` aliases to their canonical
    /// `Parents`/`Children` forms (per the design doc's lowering table).
    /// Multiple `| kind=` filters intersect into a single `kind_filter`.
    pub fn lower(self) -> LoweredQuery {
        let atom = match self.atom {
            Atom::BareId(id) => LoweredAtom::BareId(id),
            Atom::Parents {
                id,
                rel,
                transitive,
            } => LoweredAtom::Relations(RelationsQuery {
                target_id: Some(id),
                rel_type: rel,
                transitive,
                ..Default::default()
            }),
            Atom::Children {
                id,
                rel,
                transitive,
            } => LoweredAtom::Relations(RelationsQuery {
                source_id: Some(id),
                rel_type: rel,
                transitive,
                ..Default::default()
            }),
            Atom::Neighbors { id, rel } => LoweredAtom::Relations(RelationsQuery {
                object_id: Some(id),
                rel_type: rel,
                transitive: false,
                ..Default::default()
            }),
            Atom::Ancestors { id, rel } => LoweredAtom::Relations(RelationsQuery {
                target_id: Some(id),
                rel_type: Some(rel),
                transitive: true,
                ..Default::default()
            }),
            Atom::Descendants { id, rel } => LoweredAtom::Relations(RelationsQuery {
                source_id: Some(id),
                rel_type: Some(rel),
                transitive: true,
                ..Default::default()
            }),
            Atom::Scope(id) => LoweredAtom::Scope(id),
        };
        let mut kind_filter: Option<Vec<ObjectKind>> = None;
        for filter in self.filters {
            match filter {
                Filter::Kind(kinds) => match kind_filter.as_mut() {
                    None => kind_filter = Some(kinds),
                    Some(existing) => existing.retain(|k| kinds.contains(k)),
                },
            }
        }
        LoweredQuery { atom, kind_filter }
    }
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    /// Read an ident starting at the current position. Returns
    /// `(start, end, text)` and advances `pos` past the ident.
    fn read_ident(&mut self) -> Option<(usize, usize, &'a str)> {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        if start >= bytes.len() {
            return None;
        }
        let first = bytes[start];
        if !first.is_ascii_lowercase() {
            return None;
        }
        let mut end = start + 1;
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        self.pos = end;
        Some((start, end, &self.input[start..end]))
    }

    fn err(
        &self,
        span: (usize, usize),
        message: impl Into<String>,
        hint: Option<String>,
    ) -> ParseError {
        ParseError {
            input: self.input.to_string(),
            span,
            message: message.into(),
            hint,
        }
    }

    fn parse_atom(&mut self) -> Result<Atom, ParseError> {
        self.skip_ws();
        let pre = self.pos;
        let (start, end, raw) = match self.read_ident() {
            Some(t) => t,
            None => {
                return Err(self.err(
                    (pre, pre + 1),
                    "expected atom (a bare id or one of: parents, children, neighbors, ancestors, descendants, scope)",
                    None,
                ));
            }
        };
        let ident = raw.to_string();
        // Bare-id case: the ident is itself a valid HydraId.
        if HydraId::validate_str(&ident).is_ok() {
            // If it's followed by `(`, that's ambiguous; we treat it as a
            // bare id (atom names never contain '-') and let the caller
            // hit a "trailing input" error below.
            let hid = HydraId::try_from(ident).expect("validated above");
            return Ok(Atom::BareId(hid));
        }

        // Otherwise the ident must be an atom name followed by `(`.
        self.skip_ws();
        if self.peek() != Some('(') {
            if ident.contains('-') {
                // Looks id-shaped but failed HydraId validation.
                return Err(self.err((start, end), format!("invalid id '{ident}'"), None));
            }
            let hint = unknown_atom_hint(&ident, None);
            return Err(self.err((start, end), format!("unknown atom '{ident}'"), hint));
        }

        // Validate atom name *before* parsing args so the error span is
        // tight on the name and we can build a hint from the first id arg.
        if !RESERVED_NAMES.contains(&ident.as_str()) {
            let peeked = self.peek_first_id_in_call();
            let hint = unknown_atom_hint(&ident, peeked.as_deref());
            return Err(self.err((start, end), format!("unknown atom '{ident}'"), hint));
        }

        self.parse_call_body(&ident, (start, end))
    }

    /// Peek into `( ... )` (without committing) and return the first ident
    /// inside that parses as a HydraId. Used only to enrich error hints.
    fn peek_first_id_in_call(&self) -> Option<String> {
        // We're currently at '(', do NOT advance the real cursor.
        let bytes = self.input.as_bytes();
        let mut i = self.pos;
        if i >= bytes.len() || bytes[i] != b'(' {
            return None;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        let start = i;
        if start >= bytes.len() || !bytes[start].is_ascii_lowercase() {
            return None;
        }
        let mut end = start + 1;
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        let candidate = &self.input[start..end];
        if HydraId::validate_str(candidate).is_ok() {
            Some(candidate.to_string())
        } else {
            None
        }
    }

    fn parse_call_body(
        &mut self,
        name: &str,
        name_span: (usize, usize),
    ) -> Result<Atom, ParseError> {
        // Consume '('
        let paren_open = self.pos;
        self.bump();
        let args = self.parse_arglist(paren_open)?;
        match name {
            "parents" => self
                .build_directional(name, name_span, args, /* allow_transitive */ true)
                .map(|(id, rel, transitive)| Atom::Parents {
                    id,
                    rel,
                    transitive,
                }),
            "children" => {
                self.build_directional(name, name_span, args, true)
                    .map(|(id, rel, transitive)| Atom::Children {
                        id,
                        rel,
                        transitive,
                    })
            }
            "neighbors" => self
                .build_directional(name, name_span, args, false)
                .map(|(id, rel, _)| Atom::Neighbors { id, rel }),
            "ancestors" => self
                .build_alias(name, name_span, args)
                .map(|(id, rel)| Atom::Ancestors { id, rel }),
            "descendants" => self
                .build_alias(name, name_span, args)
                .map(|(id, rel)| Atom::Descendants { id, rel }),
            "scope" => self.build_scope(name_span, args).map(Atom::Scope),
            _ => unreachable!("validated by parse_atom"),
        }
    }

    fn build_directional(
        &self,
        name: &str,
        name_span: (usize, usize),
        args: ParsedArgs,
        allow_transitive: bool,
    ) -> Result<(HydraId, Option<RelType>, bool), ParseError> {
        let pivot = args
            .pivot
            .ok_or_else(|| self.err(name_span, format!("{name} requires an id argument"), None))?;
        if let Some(t_span) = args.transitive {
            if !allow_transitive {
                return Err(self.err(
                    t_span,
                    "'transitive' is not allowed on neighbors",
                    Some(
                        "the server does not support transitive closure on object_id=; \
use parents(<id>, rel=<R>, transitive) or children(<id>, rel=<R>, transitive) instead"
                            .to_string(),
                    ),
                ));
            }
            if args.rel.is_none() {
                return Err(self.err(
                    t_span,
                    "'transitive' requires 'rel='",
                    Some(
                        "transitive closure requires a rel filter; \
add e.g. 'rel=child-of'"
                            .to_string(),
                    ),
                ));
            }
        }
        Ok((pivot.0, args.rel.map(|(r, _)| r), args.transitive.is_some()))
    }

    fn build_alias(
        &self,
        name: &str,
        name_span: (usize, usize),
        args: ParsedArgs,
    ) -> Result<(HydraId, RelType), ParseError> {
        let pivot = args
            .pivot
            .ok_or_else(|| self.err(name_span, format!("{name} requires an id argument"), None))?;
        if let Some(t_span) = args.transitive {
            let canonical = if name == "ancestors" {
                "parents"
            } else {
                "children"
            };
            let rel_display = args
                .rel
                .map(|(r, _)| r.as_str().to_string())
                .unwrap_or_else(|| "<R>".to_string());
            return Err(self.err(
                t_span,
                format!("'transitive' is redundant on {name}"),
                Some(format!(
                    "{name} is sugar for the transitive form; for the explicit form use '{canonical}({}, rel={rel_display}, transitive)'",
                    pivot.0
                )),
            ));
        }
        let rel = args
            .rel
            .map(|(r, _)| r)
            .ok_or_else(|| {
                self.err(
                    name_span,
                    format!("'rel=' is required on {name}"),
                    Some(format!(
                        "{name} is sugar for the transitive form; the server requires a rel filter for transitive closure"
                    )),
                )
            })?;
        Ok((pivot.0, rel))
    }

    fn build_scope(
        &self,
        name_span: (usize, usize),
        args: ParsedArgs,
    ) -> Result<HydraId, ParseError> {
        let pivot = args
            .pivot
            .ok_or_else(|| self.err(name_span, "scope requires an id argument", None))?;
        if let Some((_, span)) = args.rel {
            return Err(self.err(span, "scope does not accept 'rel='", None));
        }
        if let Some(span) = args.transitive {
            return Err(self.err(span, "scope does not accept 'transitive'", None));
        }
        Ok(pivot.0)
    }

    fn parse_arglist(&mut self, paren_open: usize) -> Result<ParsedArgs, ParseError> {
        let mut args = ParsedArgs::default();
        let mut first = true;
        loop {
            self.skip_ws();
            if self.at_eof() {
                return Err(self.err(
                    (paren_open, self.input.len()),
                    "unterminated '(' in call",
                    None,
                ));
            }
            if self.peek() == Some(')') {
                self.bump();
                return Ok(args);
            }
            if !first {
                if self.peek() != Some(',') {
                    return Err(self.err((self.pos, self.pos + 1), "expected ',' or ')'", None));
                }
                self.bump();
                self.skip_ws();
            }
            first = false;
            self.parse_arg(&mut args)?;
        }
    }

    fn parse_arg(&mut self, args: &mut ParsedArgs) -> Result<(), ParseError> {
        let (start, end, raw) = match self.read_ident() {
            Some(t) => t,
            None => {
                let span = (self.pos, self.pos + 1);
                return Err(self.err(span, "expected argument (id, rel=<R>, or transitive)", None));
            }
        };
        let ident = raw.to_string();
        self.skip_ws();
        if self.peek() == Some('=') {
            // kw=value form
            let eq_pos = self.pos;
            self.bump();
            self.skip_ws();
            let value = self.read_ident();
            let (vs, ve, value_str) = value.ok_or_else(|| {
                self.err(
                    (eq_pos, eq_pos + 1),
                    format!("expected value after '{ident}='"),
                    None,
                )
            })?;
            match ident.as_str() {
                "rel" => {
                    let rt = value_str.parse::<RelType>().map_err(|e| {
                        self.err(
                            (vs, ve),
                            format!("invalid rel type '{value_str}'"),
                            Some(e.to_string()),
                        )
                    })?;
                    if args.rel.is_some() {
                        return Err(self.err((start, ve), "duplicate 'rel=' argument", None));
                    }
                    args.rel = Some((rt, (start, ve)));
                }
                _ => {
                    return Err(self.err(
                        (start, end),
                        format!("unknown keyword argument '{ident}='"),
                        None,
                    ));
                }
            }
        } else if HydraId::validate_str(&ident).is_ok() {
            if args.pivot.is_some() {
                return Err(self.err((start, end), "only one pivot id is accepted", None));
            }
            let hid = HydraId::try_from(ident).expect("validated above");
            args.pivot = Some((hid, (start, end)));
        } else if ident == "transitive" {
            if args.transitive.is_some() {
                return Err(self.err((start, end), "duplicate 'transitive' flag", None));
            }
            args.transitive = Some((start, end));
        } else if ident.contains('-') {
            return Err(self.err((start, end), format!("invalid id '{ident}'"), None));
        } else {
            return Err(self.err(
                (start, end),
                format!("unknown argument '{ident}'"),
                Some("expected a pivot id, 'rel=<R>', or 'transitive'".to_string()),
            ));
        }
        Ok(())
    }

    fn parse_filter(&mut self) -> Result<Filter, ParseError> {
        let (start, end, raw) = match self.read_ident() {
            Some(t) => t,
            None => {
                let span = (self.pos, self.pos + 1);
                return Err(self.err(span, "expected filter name after '|'", None));
            }
        };
        let name = raw.to_string();
        if name != "kind" {
            return Err(self.err(
                (start, end),
                format!("unknown filter '{name}'"),
                Some("only 'kind' is supported".to_string()),
            ));
        }
        self.skip_ws();
        if self.peek() != Some('=') {
            return Err(self.err((self.pos, self.pos + 1), "expected '=' after 'kind'", None));
        }
        self.bump();
        self.skip_ws();
        let mut kinds = Vec::new();
        loop {
            let (vs, ve, value) = match self.read_ident() {
                Some(t) => t,
                None => {
                    return Err(self.err((self.pos, self.pos + 1), "expected kind value", None));
                }
            };
            let kind = value.parse::<ObjectKind>().map_err(|e| {
                self.err(
                    (vs, ve),
                    format!("invalid kind '{value}'"),
                    Some(e.to_string()),
                )
            })?;
            kinds.push(kind);
            self.skip_ws();
            if self.peek() == Some(',') {
                self.bump();
                self.skip_ws();
                continue;
            }
            break;
        }
        Ok(Filter::Kind(kinds))
    }
}

#[derive(Default)]
struct ParsedArgs {
    pivot: Option<(HydraId, (usize, usize))>,
    rel: Option<(RelType, (usize, usize))>,
    transitive: Option<(usize, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hid(s: &str) -> HydraId {
        s.parse().unwrap_or_else(|e| panic!("bad id '{s}': {e}"))
    }

    // ---------- Parsing: success cases ----------

    #[test]
    fn parses_bare_id() {
        let q = Query::parse("i-abcdef").unwrap();
        assert_eq!(q.atom, Atom::BareId(hid("i-abcdef")));
        assert!(q.filters.is_empty());
    }

    #[test]
    fn parses_parents_no_args() {
        let q = Query::parse("parents(i-abcdef)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Parents {
                id: hid("i-abcdef"),
                rel: None,
                transitive: false,
            }
        );
    }

    #[test]
    fn parses_parents_with_rel() {
        let q = Query::parse("parents(i-abcdef, rel=child-of)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Parents {
                id: hid("i-abcdef"),
                rel: Some(RelType::ChildOf),
                transitive: false,
            }
        );
    }

    #[test]
    fn parses_parents_with_rel_and_transitive() {
        let q = Query::parse("parents(i-abcdef, rel=child-of, transitive)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Parents {
                id: hid("i-abcdef"),
                rel: Some(RelType::ChildOf),
                transitive: true,
            }
        );
    }

    #[test]
    fn parses_children_no_args() {
        let q = Query::parse("children(i-abcdef)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Children {
                id: hid("i-abcdef"),
                rel: None,
                transitive: false,
            }
        );
    }

    #[test]
    fn parses_children_with_rel() {
        let q = Query::parse("children(i-abcdef, rel=child-of)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Children {
                id: hid("i-abcdef"),
                rel: Some(RelType::ChildOf),
                transitive: false,
            }
        );
    }

    #[test]
    fn parses_children_with_rel_and_transitive() {
        let q = Query::parse("children(i-abcdef, rel=child-of, transitive)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Children {
                id: hid("i-abcdef"),
                rel: Some(RelType::ChildOf),
                transitive: true,
            }
        );
    }

    #[test]
    fn parses_neighbors_no_args() {
        let q = Query::parse("neighbors(i-abcdef)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Neighbors {
                id: hid("i-abcdef"),
                rel: None,
            }
        );
    }

    #[test]
    fn parses_neighbors_with_rel() {
        let q = Query::parse("neighbors(i-abcdef, rel=refers-to)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Neighbors {
                id: hid("i-abcdef"),
                rel: Some(RelType::RefersTo),
            }
        );
    }

    #[test]
    fn parses_ancestors() {
        let q = Query::parse("ancestors(i-abcdef, rel=child-of)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Ancestors {
                id: hid("i-abcdef"),
                rel: RelType::ChildOf,
            }
        );
    }

    #[test]
    fn parses_descendants() {
        let q = Query::parse("descendants(i-abcdef, rel=child-of)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Descendants {
                id: hid("i-abcdef"),
                rel: RelType::ChildOf,
            }
        );
    }

    #[test]
    fn parses_scope() {
        let q = Query::parse("scope(i-abcdef)").unwrap();
        assert_eq!(q.atom, Atom::Scope(hid("i-abcdef")));
    }

    #[test]
    fn parses_with_kind_filter() {
        let q = Query::parse("scope(i-abcdef) | kind=patch").unwrap();
        assert_eq!(q.atom, Atom::Scope(hid("i-abcdef")));
        assert_eq!(q.filters, vec![Filter::Kind(vec![ObjectKind::Patch])]);
    }

    #[test]
    fn parses_with_multi_kind_filter() {
        let q = Query::parse("scope(i-abcdef) | kind=patch,document").unwrap();
        assert_eq!(
            q.filters,
            vec![Filter::Kind(vec![ObjectKind::Patch, ObjectKind::Document])]
        );
    }

    #[test]
    fn parses_bare_id_with_kind_filter() {
        let q = Query::parse("i-abcdef | kind=issue").unwrap();
        assert_eq!(q.atom, Atom::BareId(hid("i-abcdef")));
        assert_eq!(q.filters, vec![Filter::Kind(vec![ObjectKind::Issue])]);
    }

    #[test]
    fn parses_parents_with_pipe() {
        let q = Query::parse("parents(i-abcdef, rel=child-of, transitive) | kind=patch,document")
            .unwrap();
        assert!(matches!(q.atom, Atom::Parents { .. }));
        assert_eq!(
            q.filters,
            vec![Filter::Kind(vec![ObjectKind::Patch, ObjectKind::Document])]
        );
    }

    #[test]
    fn whitespace_tolerance_inside_call_and_around_pipe() {
        let q = Query::parse(
            "  parents(  i-abcdef ,  rel = child-of , transitive )   |   kind = patch  ",
        )
        .unwrap();
        assert_eq!(
            q.atom,
            Atom::Parents {
                id: hid("i-abcdef"),
                rel: Some(RelType::ChildOf),
                transitive: true,
            }
        );
        assert_eq!(q.filters, vec![Filter::Kind(vec![ObjectKind::Patch])]);
    }

    #[test]
    fn parses_patch_pivot() {
        let q = Query::parse("parents(p-deadbe)").unwrap();
        assert_eq!(
            q.atom,
            Atom::Parents {
                id: hid("p-deadbe"),
                rel: None,
                transitive: false,
            }
        );
    }

    // ---------- Parsing: failure cases ----------

    fn err(input: &str) -> ParseError {
        Query::parse(input).unwrap_err()
    }

    #[test]
    fn rejects_transitive_on_neighbors() {
        let e = err("neighbors(i-abcdef, rel=child-of, transitive)");
        assert!(
            e.message
                .contains("'transitive' is not allowed on neighbors")
        );
        // Caret should land on the `transitive` keyword.
        let token = &e.input[e.span.0..e.span.1];
        assert_eq!(token, "transitive");
    }

    #[test]
    fn rejects_parents_transitive_without_rel() {
        let e = err("parents(i-abcdef, transitive)");
        assert!(
            e.message.contains("'transitive' requires 'rel='"),
            "msg: {}",
            e.message
        );
        let token = &e.input[e.span.0..e.span.1];
        assert_eq!(token, "transitive");
    }

    #[test]
    fn rejects_children_transitive_without_rel() {
        let e = err("children(i-abcdef, transitive)");
        assert!(e.message.contains("'transitive' requires 'rel='"));
    }

    #[test]
    fn rejects_explicit_transitive_on_ancestors() {
        let e = err("ancestors(i-abcdef, rel=child-of, transitive)");
        assert!(
            e.message.contains("redundant on ancestors"),
            "msg: {}",
            e.message
        );
        let hint = e.hint.unwrap();
        assert!(
            hint.contains("parents(i-abcdef, rel=child-of, transitive)"),
            "hint: {hint}"
        );
    }

    #[test]
    fn rejects_explicit_transitive_on_descendants() {
        let e = err("descendants(i-abcdef, rel=child-of, transitive)");
        assert!(e.message.contains("redundant on descendants"));
        let hint = e.hint.unwrap();
        assert!(
            hint.contains("children(i-abcdef, rel=child-of, transitive)"),
            "hint: {hint}"
        );
    }

    #[test]
    fn rejects_ancestors_without_rel() {
        let e = err("ancestors(i-abcdef)");
        assert!(e.message.contains("'rel=' is required on ancestors"));
        assert!(e.hint.is_some());
    }

    #[test]
    fn rejects_descendants_without_rel() {
        let e = err("descendants(i-abcdef)");
        assert!(e.message.contains("'rel=' is required on descendants"));
    }

    #[test]
    fn rejects_unknown_atom_with_hint() {
        let e = err("kids(i-abcdef)");
        assert!(e.message.contains("unknown atom 'kids'"));
        let hint = e.hint.unwrap();
        assert!(hint.contains("children"), "hint: {hint}");
        // Caret on the atom name.
        let token = &e.input[e.span.0..e.span.1];
        assert_eq!(token, "kids");
    }

    #[test]
    fn rejects_unknown_atom_levenshtein_suggestion() {
        // 'parets' is 1 edit from 'parents'.
        let e = err("parets(i-abcdef)");
        let hint = e.hint.unwrap();
        assert!(hint.contains("parents"), "hint: {hint}");
    }

    #[test]
    fn rejects_object_atom_with_bare_id_hint() {
        let e = err("object(i-abcdef)");
        assert!(e.message.contains("unknown atom 'object'"));
        let hint = e.hint.unwrap();
        assert!(hint.contains("bare id"), "hint: {hint}");
        assert!(hint.contains("i-abcdef"), "hint: {hint}");
    }

    #[test]
    fn rejects_malformed_id_alone() {
        // 'i-ab' has too-short suffix (MIN_RANDOM_LEN = 4).
        let e = err("i-ab");
        assert!(e.message.contains("invalid id"), "msg: {}", e.message);
        let token = &e.input[e.span.0..e.span.1];
        assert_eq!(token, "i-ab");
    }

    #[test]
    fn rejects_unterminated_paren() {
        let e = err("parents(i-abcdef");
        assert!(e.message.contains("unterminated"), "msg: {}", e.message);
    }

    #[test]
    fn rejects_trailing_junk() {
        let e = err("scope(i-abcdef) garbage");
        assert!(e.message.contains("trailing input"), "msg: {}", e.message);
    }

    #[test]
    fn rejects_double_pipe() {
        let e = err("scope(i-abcdef) || kind=patch");
        assert!(e.message.contains("double pipe"), "msg: {}", e.message);
    }

    #[test]
    fn rejects_unknown_filter_name() {
        let e = err("scope(i-abcdef) | status=open");
        assert!(
            e.message.contains("unknown filter 'status'"),
            "msg: {}",
            e.message
        );
    }

    #[test]
    fn rejects_empty_input() {
        let e = err("");
        assert!(e.message.contains("empty query"), "msg: {}", e.message);
    }

    #[test]
    fn rejects_invalid_rel_type() {
        let e = err("parents(i-abcdef, rel=bogus)");
        assert!(
            e.message.contains("invalid rel type 'bogus'"),
            "msg: {}",
            e.message
        );
    }

    #[test]
    fn rejects_invalid_kind() {
        let e = err("scope(i-abcdef) | kind=bogus");
        assert!(e.message.contains("invalid kind 'bogus'"));
    }

    // ---------- Lowering ----------

    #[test]
    fn lowers_bare_id() {
        let q = Query::parse("i-abcdef").unwrap().lower();
        assert_eq!(q.atom, LoweredAtom::BareId(hid("i-abcdef")));
        assert_eq!(q.kind_filter, None);
    }

    #[test]
    fn lowers_parents() {
        let q = Query::parse("parents(i-abcdef)").unwrap().lower();
        assert_eq!(
            q.atom,
            LoweredAtom::Relations(RelationsQuery {
                target_id: Some(hid("i-abcdef")),
                ..Default::default()
            })
        );
    }

    #[test]
    fn lowers_parents_with_rel_and_transitive() {
        let q = Query::parse("parents(i-abcdef, rel=child-of, transitive)")
            .unwrap()
            .lower();
        assert_eq!(
            q.atom,
            LoweredAtom::Relations(RelationsQuery {
                target_id: Some(hid("i-abcdef")),
                rel_type: Some(RelType::ChildOf),
                transitive: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn lowers_children() {
        let q = Query::parse("children(i-abcdef, rel=child-of)")
            .unwrap()
            .lower();
        assert_eq!(
            q.atom,
            LoweredAtom::Relations(RelationsQuery {
                source_id: Some(hid("i-abcdef")),
                rel_type: Some(RelType::ChildOf),
                transitive: false,
                ..Default::default()
            })
        );
    }

    #[test]
    fn lowers_neighbors() {
        let q = Query::parse("neighbors(i-abcdef, rel=refers-to)")
            .unwrap()
            .lower();
        assert_eq!(
            q.atom,
            LoweredAtom::Relations(RelationsQuery {
                object_id: Some(hid("i-abcdef")),
                rel_type: Some(RelType::RefersTo),
                transitive: false,
                ..Default::default()
            })
        );
    }

    #[test]
    fn ancestors_lowers_identically_to_parents_transitive() {
        let a = Query::parse("ancestors(i-abcdef, rel=child-of)")
            .unwrap()
            .lower();
        let p = Query::parse("parents(i-abcdef, rel=child-of, transitive)")
            .unwrap()
            .lower();
        assert_eq!(a, p);
    }

    #[test]
    fn descendants_lowers_identically_to_children_transitive() {
        let d = Query::parse("descendants(i-abcdef, rel=child-of)")
            .unwrap()
            .lower();
        let c = Query::parse("children(i-abcdef, rel=child-of, transitive)")
            .unwrap()
            .lower();
        assert_eq!(d, c);
    }

    #[test]
    fn lowers_scope() {
        let q = Query::parse("scope(i-abcdef)").unwrap().lower();
        assert_eq!(q.atom, LoweredAtom::Scope(hid("i-abcdef")));
    }

    #[test]
    fn lowers_kind_filter_preserved() {
        let q = Query::parse("scope(i-abcdef) | kind=patch")
            .unwrap()
            .lower();
        assert_eq!(q.kind_filter, Some(vec![ObjectKind::Patch]));
    }

    #[test]
    fn lowers_multi_kind_filter_preserved() {
        let q = Query::parse("scope(i-abcdef) | kind=patch,document")
            .unwrap()
            .lower();
        assert_eq!(
            q.kind_filter,
            Some(vec![ObjectKind::Patch, ObjectKind::Document])
        );
    }

    #[test]
    fn lowers_chained_kind_filters_intersect() {
        let q = Query::parse("scope(i-abcdef) | kind=patch,document | kind=patch")
            .unwrap()
            .lower();
        assert_eq!(q.kind_filter, Some(vec![ObjectKind::Patch]));
    }

    // ---------- ParseError::Display: explicit, reviewable format ----------

    #[test]
    fn parse_error_display_renders_caret_diagram() {
        let e = err("kids(i-abcdef)");
        let rendered = format!("{e}");
        let expected = "\
error: unknown atom 'kids' at position 0
  kids(i-abcdef)
  ^^^^
hint: did you mean 'children(i-abcdef)'?";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn parse_error_display_carets_at_offset() {
        let e = err("neighbors(i-abcdef, rel=child-of, transitive)");
        let rendered = format!("{e}");
        // Locate the caret line and check the carets sit under the 'transitive' token.
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 4); // error, input, carets, hint
        let input_line = lines[1];
        let caret_line = lines[2];
        let token_idx = input_line.find("transitive").unwrap();
        // input/caret lines are both indented by 2 spaces.
        assert_eq!(caret_line.find('^').unwrap(), token_idx);
        assert_eq!(caret_line.matches('^').count(), "transitive".len());
    }
}
