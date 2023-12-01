use core::slice::Iter;
use nom::{InputIter, InputLength, InputTake, Needed, Slice, UnspecializedInput};
use std::iter::Enumerate;
use std::ops::{Range, RangeFrom, RangeFull, RangeTo};
use crate::css3::parser::{ComponentValue, Function, SimpleBlock};
use crate::css3::tokenizer::Token;

/// Span is a slice of component values that are parsed by the main css3 parser that can be used
/// as input for nom parsers. Originally taken from
/// https://github.com/Rydgel/monkey-rust/blob/master/lib/lexer/token.rs

#[derive(Clone, Debug)]
pub struct Span<'t> {
    pub list: &'t [ComponentValue],
    pub start: usize,
    pub end: usize,
}

impl<'t> Span<'t> {
    pub(crate) fn new(input: &'t Vec<ComponentValue>) -> Self {
        Span {
            list: input.as_slice(),
            start: 0,
            end: input.len(),
        }
    }

    pub fn to_token(&self) -> Option<&Token> {
        match self.list.get(0) {
            Some(ComponentValue::PreservedToken(token)) => Some(token),
            _ => None,
        }
    }

    pub fn to_simple_block(&self) -> Option<&SimpleBlock> {
        match self.list.get(0) {
            Some(ComponentValue::SimpleBlock(block)) => Some(block),
            _ => None,
        }
    }

    pub fn to_function(&self) -> Option<&Function> {
        match self.list.get(0) {
            Some(ComponentValue::Function(func)) => Some(func),
            _ => None,
        }
    }

    // pub(crate) fn get(&self, index: usize) -> Option<&ComponentValue> {
    //     self.list.get(index)
    // }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

// impl<'t> InputLength for Span<'t> {
//     #[inline]
//     fn input_len(&self) -> usize {
//         self.list.len()
//     }
// }

impl<'t> InputTake for Span<'_> {
    #[inline]
    fn take(&self, count: usize) -> Self {
        Span{
            list: &self.list[0..count],
            start: 0,
            end: count,
        }
    }

    #[inline]
    fn take_split(&self, count: usize) -> (Self, Self) {
        let (prefix, suffix) = self.list.split_at(count);
        (
            Span {
                list: suffix,
                start: count,
                end: self.end,
            },
            Span {
                list: prefix,
                start: 0,
                end: count,
            }
        )
    }
}

impl<'t> InputLength for Span<'_> {
    #[inline]
    fn input_len(&self) -> usize {
        self.list.len()
    }
}

impl<'t> Slice<Range<usize>> for Span<'t> {
    #[inline]
    fn slice(&self, range: Range<usize>) -> Self {
        Span {
            list: self.list.slice(range.clone()),
            start: self.start + range.start,
            end: self.start + range.end,
        }
    }
}

impl<'t> Slice<RangeTo<usize>> for Span<'t> {
    #[inline]
    fn slice(&self, range: RangeTo<usize>) -> Self {
        self.slice(0..range.end)
    }
}

impl<'t> Slice<RangeFrom<usize>> for Span<'t> {
    #[inline]
    fn slice(&self, range: RangeFrom<usize>) -> Self {
        self.slice(range.start..self.end - self.start)
    }
}

impl<'t> Slice<RangeFull> for Span<'t> {
    #[inline]
    fn slice(&self, _: RangeFull) -> Self {
        Span {
            list: self.list,
            start: self.start,
            end: self.end,
        }
    }
}

impl<'t> InputIter for Span<'t> {
    type Item = &'t ComponentValue;
    type Iter = Enumerate<Iter<'t, ComponentValue>>;
    type IterElem = Iter<'t, ComponentValue>;

    #[inline]
    fn iter_indices(&self) -> Self::Iter {
        self.list.iter().enumerate()
    }

    #[inline]
    fn iter_elements(&self) -> Self::IterElem {
        self.list.iter()
    }

    #[inline]
    fn position<P>(&self, predicate: P) -> Option<usize>
    where
        P: Fn(Self::Item) -> bool,
    {
        self.list.iter().position(predicate)
    }

    #[inline]
    fn slice_index(&self, count: usize) -> Result<usize, nom::Needed> {
        if self.list.len() >= count {
            return Ok(count);
        }

        Err(Needed::Unknown)
    }
}

impl UnspecializedInput for Span<'_> {
}