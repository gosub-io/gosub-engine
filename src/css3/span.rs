use core::slice::Iter;
use crate::css3::tokenizer::Token;
use nom::{InputIter, InputLength, InputTake, Needed, Slice, UnspecializedInput};
use std::iter::Enumerate;
use std::ops::{Range, RangeFrom, RangeFull, RangeTo};

/// Span is a slice of tokens that can be used as input for nom parsers. Originally taken from
/// https://github.com/Rydgel/monkey-rust/blob/master/lib/lexer/token.rs

#[derive(Clone, Debug)]
pub struct Span<'t> {
    pub list: &'t [Token],
    pub start: usize,
    pub end: usize,
}

impl<'t> Span<'t> {
    pub(crate) fn new(input: &'t Vec<Token>) -> Self {
        Span {
            list: input.as_slice(),
            start: 0,
            end: input.len(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&Token> {
        self.list.get(index)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn to_token(&self) -> Token {
        self.list[0].clone()
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
    type Item = &'t Token;
    type Iter = Enumerate<Iter<'t, Token>>;
    type IterElem = Iter<'t, Token>;

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


//
//
//
//
//
//
//
//
//
//
//
//
//
//
//
//
//
//
//
// impl Compare<Token> for Span<'_> {
//     fn compare(&self, t: Token) -> nom::CompareResult {
//         if self.0[0] == t {
//             nom::CompareResult::Ok
//         } else {
//             nom::CompareResult::Error
//         }
//     }
//
//     fn compare_no_case(&self, _t: Token) -> nom::CompareResult {
//         todo!()
//     }
// }
//
//
// pub struct SpanIter;
//
// impl Iterator for SpanIter {
//     type Item = (usize, Token);
//
//     fn next(&mut self) -> Option<Self::Item> {
//         todo!()
//     }
// }
//
// pub struct TokenIter;
//
// impl Iterator for TokenIter {
//     type Item = Token;
//
//     fn next(&mut self) -> Option<Self::Item> {
//         todo!()
//     }
// }
//
// impl InputIter for Span<'_> {
//     type Item = Token;
//
//     type Iter = SpanIter;
//
//     type IterElem = TokenIter;
//
//     fn iter_indices(&self) -> Self::Iter {
//         todo!()
//     }
//
//     fn iter_elements(&self) -> Self::IterElem {
//         todo!()
//     }
//
//     fn position<P>(&self, predicate: P) -> Option<usize>
//     where
//         P: Fn(Self::Item) -> bool,
//     {
//         for (i, t) in self.0.iter().enumerate() {
//             if predicate(t.clone()) {
//                 return Some(i);
//             }
//         }
//         None
//     }
//
//     fn slice_index(&self, _count: usize) -> Result<usize, nom::Needed> {
//         todo!()
//     }
// }

impl UnspecializedInput for Span<'_> {
}