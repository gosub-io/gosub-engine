import codecs
import contextlib
import io
import json
import os
import re
import sys
from collections import Counter
from os.path import dirname, join, pardir, relpath
from typing import Any, Dict, List, Optional, Set, TypeVar

from . import parser
from ._vendor.funcparserlib.parser import NoParseError

text_type = str
binary_type = bytes

StringLike = TypeVar("StringLike", str, bytes)

base = join(dirname(__file__), pardir)

_surrogateRe = re.compile(r"\\u([0-9A-Fa-f]{4})(?:\\u([0-9A-Fa-f]{4}))?")


def clean_path(path: str) -> str:
    return relpath(path, base)


def is_subsequence(l1: List[StringLike], l2: List[StringLike]) -> bool:
    """checks if l1 is a subsequence of l2"""
    i = 0
    for x in l2:
        if l1[i] == x:
            i += 1
            if i == len(l1):
                return True
    return False


def unescape_json(obj: Any) -> Any:
    def decode_str(inp):
        """Decode \\uXXXX escapes

        This decodes \\uXXXX escapes, possibly into non-BMP characters when
        two surrogate character escapes are adjacent to each other.
        """

        # This cannot be implemented using the unicode_escape codec
        # because that requires its input be ISO-8859-1, and we need
        # arbitrary unicode as input.
        def repl(m):
            if m.group(2) is not None:
                high = int(m.group(1), 16)
                low = int(m.group(2), 16)
                if (
                    0xD800 <= high <= 0xDBFF
                    and 0xDC00 <= low <= 0xDFFF
                    and sys.maxunicode == 0x10FFFF
                ):
                    cp = ((high - 0xD800) << 10) + (low - 0xDC00) + 0x10000
                    return chr(cp)
                else:
                    return chr(high) + chr(low)
            else:
                return chr(int(m.group(1), 16))

        return _surrogateRe.sub(repl, inp)

    if isinstance(obj, dict):
        return {decode_str(k): unescape_json(v) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [unescape_json(x) for x in obj]
    elif isinstance(obj, text_type):
        return decode_str(obj)
    else:
        return obj


def lint_dat_format(
    path: str,
    encoding: Optional[str],
    first_header: StringLike,
    expected_headers: Optional[List[StringLike]] = None,
    input_headers: Optional[Set[StringLike]] = None,
) -> List[Dict[StringLike, StringLike]]:
    if expected_headers is not None and first_header not in expected_headers:
        raise ValueError("First header must be an expected header. (lint config error)")

    if (
        input_headers is not None
        and expected_headers is not None
        and not (set(input_headers) < set(expected_headers))
    ):
        raise ValueError(
            "Input header must be a subset of expected headers. (lint config error)"
        )

    if expected_headers is not None and len(set(expected_headers)) < len(
        expected_headers
    ):
        raise ValueError(
            "Can't expect a single header multiple times. (lint config error)"
        )

    if input_headers is None:
        input_headers = set(expected_headers)

    try:
        if encoding is not None:
            with codecs.open(path, "r", encoding=encoding) as fp:
                dat = fp.read()
                parsed = parser.parse(dat, first_header)
        else:
            with open(path, "rb") as fp:
                dat = fp.read()
                parsed = parser.parse(dat, first_header)
    except NoParseError as e:
        print("Parse error in {}, {}".format(path, e))
        return

    seen_items = {}

    for item in parsed:
        # Check we don't have duplicate headers within one item.
        headers = Counter(x[0] for x in item.data)
        headers.subtract(set(headers.elements()))  # remove one instance of each
        for header in set(headers.elements()):
            c = headers[header]
            print(
                f"Duplicate header {header!r} occurs {c+1} times in one item in {path} at line {item.lineno}"
            )

        item_dict = dict(item.data)

        # Check we only have expected headers.
        if expected_headers is not None:
            if not is_subsequence(
                list(item_dict.keys()),
                expected_headers,
            ):
                unexpected = item_dict.keys()
                print(
                    f"Unexpected item headings in {list(unexpected)!r} in {path} at line {item.lineno}"
                )

        # Check for duplicated items.
        if input_headers is not None:
            found_input = set()
            for input_header in input_headers:
                found_input.add((input_header, item_dict.get(input_header)))
        else:
            found_input = set(item_dict.items())

        first_line = seen_items.setdefault(frozenset(found_input), item.lineno)
        if first_line is not None and first_line != item.lineno:
            print(
                f"Duplicate item in {path} at line {item.lineno} previously seen on line {first_line}"
            )

    return [dict(x.data) for x in parsed]


def lint_encoding_test(path: str) -> None:
    parsed = lint_dat_format(
        path,
        None,
        b"data",
        expected_headers=[b"data", b"encoding"],
        input_headers={b"data"},
    )
    if not parsed:
        # We'll already have output if there's a parse error.
        return

    # We'd put extra linting here, if we ever have anything specific to the
    # encoding tests here.


def lint_encoding_tests(path: str) -> None:
    for root, dirs, files in os.walk(path):
        for file in sorted(files):
            if not file.endswith(".dat"):
                continue
            lint_encoding_test(clean_path(join(root, file)))


def lint_tokenizer_test(path: str) -> None:
    all_keys = {
        "description",
        "input",
        "output",
        "initialStates",
        "lastStartTag",
        "ignoreErrorOrder",
        "doubleEscaped",
        "errors",
    }
    required = {"input", "output"}
    with codecs.open(path, "r", "utf-8") as fp:
        parsed = json.load(fp)
    if not parsed:
        return
    if not isinstance(parsed, dict):
        print("Top-level must be an object in %s" % path)
        return
    for test_group in parsed.values():
        if not isinstance(test_group, list):
            print("Test groups must be a lists in %s" % path)
            continue
        for test in test_group:
            if "doubleEscaped" in test and test["doubleEscaped"] is True:
                test = unescape_json(test)
            keys = set(test.keys())
            if not (required <= keys):
                print(
                    "missing test properties {!r} in {}".format(required - keys, path)
                )
            if not (keys <= all_keys):
                print(
                    "unknown test properties {!r} in {}".format(keys - all_keys, path)
                )


def lint_tokenizer_tests(path: str) -> None:
    for root, dirs, files in os.walk(path):
        for file in sorted(files):
            if not file.endswith(".test"):
                continue
            lint_tokenizer_test(clean_path(join(root, file)))


def lint_tree_construction_test(path: str) -> None:
    parsed = lint_dat_format(
        path,
        "utf-8",
        "data",
        expected_headers=[
            "data",
            "errors",
            "new-errors",
            "document-fragment",
            "script-off",
            "script-on",
            "document",
        ],
        input_headers={
            "data",
            "document-fragment",
            "script-on",
            "script-off",
        },
    )
    if not parsed:
        # We'll already have output if there's a parse error.
        return

    # We'd put extra linting here, if we ever have anything specific to the
    # tree construction tests here.


def lint_tree_construction_tests(path: str) -> None:
    for root, dirs, files in os.walk(path):
        for file in sorted(files):
            if not file.endswith(".dat"):
                continue
            lint_tree_construction_test(clean_path(join(root, file)))


def main() -> int:
    with contextlib.redirect_stdout(io.StringIO()) as f:
        lint_encoding_tests(join(base, "encoding"))
        lint_tokenizer_tests(join(base, "tokenizer"))
        lint_tree_construction_tests(join(base, "tree-construction"))

    print(f.getvalue(), end="")
    return 0 if f.getvalue() == "" else 1


if __name__ == "__main__":
    sys.exit(main())
