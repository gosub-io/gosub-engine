import re
from typing import Callable, List, Optional, Tuple, Type, TypeVar, Union

from ._vendor.funcparserlib.lexer import LexerError, Token
from ._vendor.funcparserlib.parser import (
    NoParseError,
    Parser,
    _Tuple,
    finished,
    many,
    pure,
    skip,
    some,
    tok,
)

StringLike = TypeVar("StringLike", str, bytes)


class Test:
    def __init__(
        self, data: List[Tuple[StringLike, StringLike]], lineno: Optional[int] = None
    ) -> None:
        self.data = data
        self.lineno = lineno


def _make_tokenizer(specs: List[Tuple[str, Tuple[StringLike]]]) -> Callable:
    # Forked from upstream funcparserlib.lexer to fix #46
    def compile_spec(spec):
        name, args = spec
        return name, re.compile(*args)

    compiled = [compile_spec(s) for s in specs]

    def match_specs(specs, s, i, position):
        if isinstance(s, str):
            lf = "\n"
        else:
            lf = b"\n"
        line, pos = position
        for type, regexp in specs:
            m = regexp.match(s, i)
            if m is not None:
                value = m.group()
                nls = value.count(lf)
                n_line = line + nls
                if nls == 0:
                    n_pos = pos + len(value)
                else:
                    n_pos = len(value) - value.rfind(lf) - 1
                return Token(type, value, (line, pos + 1), (n_line, n_pos))
        else:
            errline = s.splitlines()[line - 1]
            raise LexerError((line, pos + 1), errline)

    def f(s):
        length = len(s)
        line, pos = 1, 0
        i = 0
        while i < length:
            t = match_specs(compiled, s, i, (line, pos))
            yield t
            line, pos = t.end
            i += len(t.value)

    return f


_token_specs_u = [
    ("HEADER", (r"[ \t]*#[^\n]*",)),
    ("BODY", (r"[^#\n][^\n]*",)),
    ("EOL", (r"\n",)),
]

_token_specs_b = [
    (name, (regexp.encode("ascii"),)) for (name, (regexp,)) in _token_specs_u
]

_tokenizer_u = _make_tokenizer(_token_specs_u)
_tokenizer_b = _make_tokenizer(_token_specs_b)


def _many_merge(toks: _Tuple) -> List[Test]:
    x, xs = toks
    return [x] + xs


def _notFollowedBy(p: Parser) -> Parser:
    @Parser
    def __notFollowedBy(tokens, s):
        try:
            p.run(tokens, s)
        except NoParseError:
            return skip(pure(None)).run(tokens, s)
        else:
            raise NoParseError("is followed by", s)

    __notFollowedBy.name = "(notFollowedBy {})".format(p)
    return __notFollowedBy


def _trim_prefix(s: StringLike, prefix: StringLike) -> StringLike:
    if s.startswith(prefix):
        return s[len(prefix) :]
    else:
        return s


def _make_test(result: _Tuple) -> Test:
    first, rest = result
    (first_header, first_lineno), first_body = first
    return Test([(first_header, first_body)] + rest, lineno=first_lineno)


def _parser(
    tokens: List[Token],
    new_test_header: StringLike,
    tok_type: Union[Type[str], Type[bytes]],
) -> List[Test]:
    if tok_type is str:
        header_prefix = "#"
    elif tok_type is bytes:
        header_prefix = b"#"
    else:
        assert False, "unreachable"

    first_header = (
        some(
            lambda tok: tok.type == "HEADER"
            and tok.value == header_prefix + new_test_header
        )
        >> (
            lambda x: (
                _trim_prefix(x.value, header_prefix),
                x.start[0] if x.start is not None else None,
            )
        )
    ) + skip(tok("EOL"))

    header = (
        some(
            lambda tok: tok.type == "HEADER"
            and tok.value != header_prefix + new_test_header
        )
        >> (lambda x: _trim_prefix(x.value, header_prefix))
    ) + skip(tok("EOL"))

    body = tok("BODY") + tok("EOL") >> (lambda x: x[0] + x[1])
    empty = tok("EOL")

    actual_body = many(body | (empty + skip(_notFollowedBy(first_header)))) >> (
        lambda xs: tok_type().join(xs)[:-1]
    )

    first_segment = first_header + actual_body >> tuple
    rest_segment = header + actual_body >> tuple

    test = first_segment + many(rest_segment) >> _make_test

    tests = (test + many(skip(empty) + test)) >> _many_merge

    toplevel = tests + skip(finished)

    return toplevel.parse(tokens)


def parse(s: StringLike, new_test_header: StringLike) -> List[Test]:
    if type(s) != type(new_test_header):
        raise TypeError("s and new_test_header must have same type")

    if isinstance(s, str):
        return _parser(list(_tokenizer_u(s)), new_test_header, str)
    elif isinstance(s, bytes):
        return _parser(list(_tokenizer_b(s)), new_test_header, bytes)
    else:
        raise TypeError("s must be unicode or bytes object")
