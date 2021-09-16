mod display;
mod finish_with;
mod ll_selection;
pub mod x;

pub use finish_with::FinishWith;
pub use ll_selection::LLSelection;

use crate::type_bucket::{self, AnyAttribute};
use crate::type_id_to_many::TypeIdToMany;
pub use display::LLLineDisplay;
use std::fmt::{self, Write};
use std::iter::FromIterator;
use std::{collections::HashMap, rc::Rc};
pub use x::{Attr, AttrEq};
use x::{XForwards, XMatch};

/// [TextTag] is an attribute added at the beginning of every new line.
///
/// Each piece of a line is sort of "tokenized" and each token is assigned a [TextTag] attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum TextTag {
    /// Natural number like `0`, `1200`, `0004`
    NATN,
    /// English sentence punctuation symbols.
    /// `,`, `.`, `!`, `;`, `:`, `?`, `'`, `"`
    PUNC,
    /// Any other symbol or emoji
    SYMB,
    /// A combination of unicode whitespaces
    SPACE,
    /// A word as identified by unicode word recognition rules.
    ///
    /// For example: `yello`, `Paris`, `don't`, `should've`
    WORD,
}

#[derive(Debug)]
pub enum LToken {
    Text(String, TextTag),
    /// TODO: something more interesting
    Value,
}

#[derive(Debug)]
pub struct LLToken {
    pub(crate) token_idx: usize,
    // token span position (not token index)
    pub(crate) pos_starts_at: usize,
    // token span position (not token index)
    pub(crate) pos_ends_at: usize,
    pub(crate) token: LToken,
}

impl LLToken {
    /// Get a reference to the l l token's token.
    pub fn get_token(&self) -> &LToken {
        &self.token
    }
}

/// (starts at, ends at) token indexes
type LRange = (usize, usize);
/// (starts at, ends at) token positions
type PositionRange = (usize, usize);

/// Top-level
struct LLLineAttrs {
    // "bi-map" / "tri-map"
    ranges: TypeIdToMany<LRange>,
    /// match_forwards uses [LLSelection::end_idx]
    starts_at: Vec<TypeIdToMany<LRange>>,
    /// match_backwards uses [LLSelection::start_idx]
    ends_at: Vec<TypeIdToMany<LRange>>,
    values: HashMap<LRange, type_bucket::TypeBucket>,
}

pub struct LLLineFind<'l, Found> {
    start_pos_at: usize,
    end_pos_at: usize,
    found: Found,
    _phantom: std::marker::PhantomData<&'l ()>,
}

impl<'l, Found: fmt::Debug> fmt::Debug for LLLineFind<'l, Found> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LLLineFind")
            .field("start", &self.start_pos_at)
            .field("end", &self.end_pos_at)
            .field("found", &self.found)
            .finish()
    }
}

impl<'l, Found> LLLineFind<'l, Found> {
    pub fn range(&self) -> PositionRange {
        (self.start_pos_at, self.end_pos_at)
    }
    pub fn attr(&self) -> &Found {
        &self.found
    }
}

/// Create using [layered_nlp::create_tokens] function.
pub struct LLLine {
    // how much do we actually need of the original Vec if much of the data is put into the bi-map?
    ll_tokens: Vec<LLToken>,
    attrs: LLLineAttrs,
}

impl LLLine {
    pub(crate) fn new(ll_tokens: Vec<LLToken>) -> Self {
        let starts_at: Vec<TypeIdToMany<LRange>> =
            (0..ll_tokens.len()).map(|_| Default::default()).collect();
        let ends_at: Vec<TypeIdToMany<LRange>> =
            (0..ll_tokens.len()).map(|_| Default::default()).collect();

        let mut attrs = LLLineAttrs {
            ranges: Default::default(),
            starts_at,
            ends_at,
            values: Default::default(),
        };

        for (token_idx, ll_token) in ll_tokens.iter().enumerate() {
            match &ll_token.token {
                LToken::Text(text, tag) => {
                    if text.chars().count() == 1 {
                        // insert char automatically if just one char
                        attrs.insert((token_idx, token_idx), text.chars().next().unwrap());
                    }
                    // insert TextTag automatically
                    attrs.insert((token_idx, token_idx), tag.clone());
                }
                LToken::Value => {
                    // nothing to do...
                }
            }
        }

        LLLine { ll_tokens, attrs }
    }

    pub fn run<R>(mut self, recognizer: &R) -> Self
    where
        R: Resolver,
    {
        // Empty line can't recognize anything since they can't create `LLSelection`
        if self.ll_tokens.is_empty() {
            return self;
        }

        let ll_line = Rc::new(self);

        let assignments = recognizer.go(LLSelection {
            ll_line: ll_line.clone(),
            start_idx: 0,
            end_idx: ll_line.ll_tokens().len() - 1,
        });

        self = Rc::try_unwrap(ll_line)
            .map_err(drop)
            .expect("there is no other Rc currently");

        // store new attributes generated by the resolver
        for LLCursorAssignment {
            start_idx,
            end_idx,
            value,
        } in assignments
        {
            self.attrs.insert((start_idx, end_idx), value);
        }

        self
    }
    pub(crate) fn add_any_attrs(
        &mut self,
        start_idx: usize,
        end_idx: usize,
        attrs: Vec<AnyAttribute>,
    ) {
        let range = (start_idx, end_idx);

        for attr in attrs {
            self.attrs
                .starts_at
                .get_mut(start_idx)
                .expect("has initial starts_at value in bounds")
                .insert_any_distinct(attr.type_id(), range);
            self.attrs
                .ends_at
                .get_mut(end_idx)
                .expect("has initial ends_at value in bounds")
                .insert_any_distinct(attr.type_id(), range);
            self.attrs.ranges.insert_any_distinct(attr.type_id(), range);
            self.attrs
                .values
                .entry(range)
                .or_default()
                .insert_any_attribute(attr);
        }
    }

    /// Get a reference to the ll line's ll tokens.
    pub fn ll_tokens(&self) -> &[LLToken] {
        &self.ll_tokens
    }

    /// Returns Attributes' information outside `LLLine`
    /// "find"
    pub fn find<'l, M: XMatch<'l>>(&'l self, matcher: &M) -> Vec<LLLineFind<'l, M::Out>> {
        (0..self.ll_tokens.len())
            .flat_map(|i| {
                let forwards = XForwards { from_idx: i };

                matcher
                    .go(&forwards, &self)
                    .into_iter()
                    .map(move |(out, next_idx)| LLLineFind {
                        start_pos_at: self.pos_start_at(i),
                        end_pos_at: self.pos_end_at(next_idx.0),
                        found: out,
                        _phantom: std::marker::PhantomData,
                    })
            })
            .collect()
    }

    /// Go from a token _index_ to its ending _position_ in the input tokens.
    fn pos_end_at(&self, idx: usize) -> usize {
        self.ll_tokens
            .get(idx)
            .expect("pos_end_at in bounds")
            .pos_ends_at
    }
    /// Go from a token _index_ to its starting _position_ in the input tokens.
    fn pos_start_at(&self, idx: usize) -> usize {
        self.ll_tokens
            .get(idx)
            .expect("pos_start_at in bounds")
            .pos_starts_at
    }

    /// Returns Attributes' information outside `LLLine`
    pub fn query<'a, T: 'static>(&'a self) -> Vec<(LRange, String, Vec<&T>)> {
        self.attrs
            .ranges
            .get::<T>()
            .iter()
            .map(|range| {
                let text =
                    String::from_iter(self.ll_tokens[range.0..=range.1].iter().map(|token| {
                        match &token.token {
                            LToken::Text(text, _) => text,
                            LToken::Value => "",
                        }
                    }));

                (
                    *range,
                    text,
                    self.attrs.values[range].get::<T>().iter().collect(),
                )
            })
            .collect()
    }
}

impl LLLineAttrs {
    fn insert<T: 'static + std::fmt::Debug + Send + Sync>(&mut self, range: LRange, value: T) {
        self.starts_at
            .get_mut(range.0)
            .expect("has initial starts_at value in bounds")
            .insert_distinct::<T>(range);
        self.ends_at
            .get_mut(range.1)
            .expect("has initial ends_at value in bounds")
            .insert_distinct::<T>(range);
        self.ranges.insert_distinct::<T>(range);
        self.values.entry(range).or_default().insert(value);
    }
}

#[track_caller]
fn assert_ll_lines_equals(first: &Rc<LLLine>, second: &Rc<LLLine>) {
    if !Rc::ptr_eq(first, second) {
        panic!("Two different lines used")
    }
}

// TODO rename
#[derive(Debug)]
pub struct LLCursorAssignment<Attr> {
    // private
    start_idx: usize,
    end_idx: usize,
    // provided from resolver
    value: Attr,
}

pub trait Resolver {
    /// The kind of value that this resolver will assign into the LLLine.
    ///
    /// It is constrained to [std::fmt::Debug] in order to ensure that it's easy
    /// to debug with [layered_nlp::LLLineDisplay].
    type Attr: std::fmt::Debug + 'static + Send + Sync;
    /// How to perform the assignments.
    fn go(&self, selection: LLSelection) -> Vec<LLCursorAssignment<Self::Attr>>;
}
