use crate::css3::new_tokenizer::Token;
use nom::{Compare, InputIter, InputLength, InputTake};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(name: &str) -> Node {
        Node {
            name: name.to_string(),
            attributes: HashMap::new(),
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Span<'t>(&'t [Token]);

impl<'t> Span<'t> {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl<'t> Span<'t> {
    pub(crate) fn new(input: &'t Vec<Token>) -> Self {
        Span(input.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_token(&self) -> Token {
        self.0[0].clone()
    }
}

impl InputTake for Span<'_> {
    fn take(&self, count: usize) -> Self {
        let tokens = &self.0[0..count];
        Span(tokens)
    }

    fn take_split(&self, count: usize) -> (Self, Self) {
        let (left, right) = self.0.split_at(count);
        // Yes, other way around :/
        (Span(right), Span(left))
    }
}

impl Compare<Token> for Span<'_> {
    fn compare(&self, t: Token) -> nom::CompareResult {
        if self.0[0] == t {
            nom::CompareResult::Ok
        } else {
            nom::CompareResult::Error
        }
    }

    fn compare_no_case(&self, _t: Token) -> nom::CompareResult {
        todo!()
    }
}

impl InputLength for Span<'_> {
    fn input_len(&self) -> usize {
        self.0.len()
    }
}

pub struct SpanIter;

impl Iterator for SpanIter {
    type Item = (usize, Token);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

pub struct TokenIter;

impl Iterator for TokenIter {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl InputIter for Span<'_> {
    type Item = Token;

    type Iter = SpanIter;

    type IterElem = TokenIter;

    fn iter_indices(&self) -> Self::Iter {
        todo!()
    }

    fn iter_elements(&self) -> Self::IterElem {
        todo!()
    }

    fn position<P>(&self, _predicate: P) -> Option<usize>
    where
        P: Fn(Self::Item) -> bool,
    {
        todo!()
    }

    fn slice_index(&self, _count: usize) -> Result<usize, nom::Needed> {
        todo!()
    }
}
