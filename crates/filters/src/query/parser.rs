//! Recursive-descent parser of the quick-query grammar. Owner: W2-C.
//!
//! Builds a small AST first (so `top:N` placement can be validated) and then
//! lowers it to a `mj_core::Rule`. Term-level problems are collected so one
//! call can underline several mistakes; a syntax problem (missing `)`, dangling
//! `OR`) stops the parse at that point.

use mj_core::Rule;

use super::lexer::{lex, Span, Tok};
use super::values::{build_term, Env};
use super::QueryError;

/// Deepest nesting of `(`, `-` and `NOT` accepted.
pub const MAX_NESTING: usize = 32;

enum Ast {
    /// A leaf rule; `Some(span)` when it is a `top:N` limiter.
    Term(Rule, Option<Span>),
    And(Vec<Ast>),
    Or(Vec<Ast>),
    Not(Box<Ast>),
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    end: usize,
    env: &'a Env<'a>,
    errors: Vec<QueryError>,
}

type Fail = ();

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn keyword(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Word(w)) => w.keyword(),
            _ => None,
        }
    }

    fn here(&self) -> Span {
        self.peek().map(Tok::span).unwrap_or(Span { start: self.end, end: self.end })
    }

    fn fail<T>(&mut self, span: Span, msg: &str) -> Result<T, Fail> {
        self.errors.push(span.err(msg));
        Err(())
    }

    fn at_term_end(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::RParen(_))) || self.keyword() == Some("OR")
    }

    fn parse_or(&mut self, depth: usize) -> Result<Ast, Fail> {
        let mut alts = vec![self.parse_and(depth)?];
        while self.keyword() == Some("OR") {
            let or_span = self.here();
            self.pos += 1;
            if self.at_term_end() {
                return self.fail(or_span, "OR needs a term after it");
            }
            alts.push(self.parse_and(depth)?);
        }
        Ok(if alts.len() == 1 { alts.remove(0) } else { Ast::Or(alts) })
    }

    fn parse_and(&mut self, depth: usize) -> Result<Ast, Fail> {
        let mut items = Vec::new();
        loop {
            if self.at_term_end() {
                break;
            }
            if self.keyword() == Some("AND") {
                let span = self.here();
                if items.is_empty() {
                    return self.fail(span, "AND needs a term before it");
                }
                self.pos += 1;
                if self.at_term_end() {
                    return self.fail(span, "AND needs a term after it");
                }
                continue;
            }
            items.push(self.parse_unary(depth)?);
        }
        if items.is_empty() {
            let span = self.here();
            let msg =
                if self.keyword() == Some("OR") { "OR needs a term before it" } else { "expected a search term here" };
            return self.fail(span, msg);
        }
        Ok(if items.len() == 1 { items.remove(0) } else { Ast::And(items) })
    }

    fn parse_unary(&mut self, depth: usize) -> Result<Ast, Fail> {
        if depth > MAX_NESTING {
            let span = self.here();
            return self.fail(span, &format!("nested more than {MAX_NESTING} levels deep"));
        }
        let Some(tok) = self.peek().cloned() else {
            let span = self.here();
            return self.fail(span, "expected a search term here");
        };
        match tok {
            Tok::Neg(span) => {
                self.pos += 1;
                self.negated(span, depth)
            }
            Tok::Word(w) if w.keyword() == Some("NOT") => {
                self.pos += 1;
                self.negated(w.span, depth)
            }
            Tok::LParen(span) => {
                self.pos += 1;
                if matches!(self.peek(), Some(Tok::RParen(_))) {
                    let s = self.here();
                    return self.fail(s, "there is nothing between '(' and ')'");
                }
                let inner = self.parse_or(depth + 1)?;
                match self.peek() {
                    Some(Tok::RParen(_)) => {
                        self.pos += 1;
                        Ok(inner)
                    }
                    _ => self.fail(span, "this '(' has no matching ')'"),
                }
            }
            Tok::RParen(span) => self.fail(span, "unexpected ')' with no matching '('"),
            Tok::Word(w) => {
                self.pos += 1;
                match build_term(&w, self.env) {
                    Ok((rule, top)) => Ok(Ast::Term(rule, top)),
                    Err(e) => {
                        // Keep going so every bad term in the query is reported.
                        self.errors.push(e);
                        Ok(Ast::Term(Rule::everything(), None))
                    }
                }
            }
        }
    }

    fn negated(&mut self, op_span: Span, depth: usize) -> Result<Ast, Fail> {
        if self.at_term_end() || matches!(self.keyword(), Some("AND")) {
            return self.fail(op_span, "'-' / NOT must be followed by a term; quote a lone dash to search for it");
        }
        Ok(Ast::Not(Box::new(self.parse_unary(depth + 1)?)))
    }
}

/// Lower the AST. `top_ok` is true while every ancestor is an AND of the whole query.
fn lower(ast: Ast, top_ok: bool, errors: &mut Vec<QueryError>) -> Rule {
    match ast {
        Ast::Term(rule, top) => {
            if let (Some(span), false) = (top, top_ok) {
                errors.push(span.err("top:N limits the whole result, so it can only be combined with AND at the top level, not inside OR, NOT or a group under them"));
            }
            rule
        }
        Ast::Not(inner) => Rule::Not(Box::new(lower(*inner, false, errors))),
        Ast::Or(alts) => Rule::Any(alts.into_iter().map(|a| lower(a, false, errors)).collect()),
        Ast::And(items) => {
            let mut flat = Vec::new();
            flatten(items, &mut flat);
            Rule::All(flat.into_iter().map(|a| lower(a, top_ok, errors)).collect())
        }
    }
}

/// `a AND (b AND c)` is `a AND b AND c`.
fn flatten(items: Vec<Ast>, out: &mut Vec<Ast>) {
    for it in items {
        match it {
            Ast::And(inner) => flatten(inner, out),
            other => out.push(other),
        }
    }
}

/// Parse a query into a rule.
pub fn parse_query(query: &str, env: &Env<'_>) -> Result<Rule, Vec<QueryError>> {
    let toks = lex(query).map_err(|e| vec![e])?;
    if toks.is_empty() {
        return Err(vec![Span { start: 0, end: query.len() }
            .err("the query is empty; try ext:mp4 size:>1GB or a word from the file name")]);
    }
    let mut p = Parser { toks, pos: 0, end: query.len(), env, errors: Vec::new() };
    let ast = p.parse_or(0);
    if ast.is_ok() && p.pos < p.toks.len() {
        // Only a stray ')' can stop the top-level parse early.
        let span = p.here();
        p.errors.push(span.err("unexpected ')' with no matching '('"));
    }
    let Ok(ast) = ast else { return Err(p.errors) };
    let mut errors = p.errors;
    let rule = lower(ast, true, &mut errors);
    if errors.is_empty() {
        Ok(rule)
    } else {
        errors.sort_by_key(|e| e.offset);
        Err(errors)
    }
}
