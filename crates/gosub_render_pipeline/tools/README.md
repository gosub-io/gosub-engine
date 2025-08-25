# SouperToo
A script that will fetch a webpage (via playwright) and outputs a json file with tags, attrs, stylesheets.
This can be read by the document parser so we can have a simple(ish) way to render different webpages.

# Usage

```bash
  python3 -m venv .
  source bin/activate
  pip install -r requirements.txt
  playwright install
  python soupertoo.py www.google.com
```