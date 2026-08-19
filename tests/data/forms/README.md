# Interactive forms test page

A page exercising the native form controls end to end - typing, focus, checkboxes/radios,
select dropdowns (long lists scroll), range sliders, textarea resize, submit/reset.
Submissions land on `/echo`, which lists the submitted fields.

    python3 tests/data/forms/serve.py 8080
    cargo run --example gtk4-cairo -- http://127.0.0.1:8080/index.html

Headless replay of the same page (see `gosub-screenshot --help` for the `-i` steps):

    gosub-screenshot http://127.0.0.1:8080/index.html out.png 1280 -i click:120,140 -i scroll:0,1
