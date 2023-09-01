Almost all token tests (found in html5lib-test/tokenizer) will pass:

🏁 Tests completed: Ran 6805 tests, 2770 assertions, 2748 succeeded, 22 failed (18 position failures)

The failing test are due to the fact that rust-lang does not handle surrogate characters (0xD800-0xDFFF) in char values.
These values cannot exists on their own in a valid utf-8 string.

For instance: 

`<!DOCTYPE a PUBLIC'\uDBC0\uDC00`

This test has a non-bmp character that is internally seen as a single character but from the perspective 
of the test seen as 2 characters (hi/lo surrogate). This means that the end-of-file is off by 1 position.