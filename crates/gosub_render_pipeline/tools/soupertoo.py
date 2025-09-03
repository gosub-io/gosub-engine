import json
import sys
import asyncio
from bs4 import BeautifulSoup, Comment
from playwright.async_api import async_playwright


async def fetch_and_parse_html(url):
    """Fetch the HTML page, resolve styles, and return its JSON DOM structure."""
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)  # Run headless browser
        page = await browser.new_page()

        await page.set_viewport_size({"width": 1280, "height": 1144})
        await page.goto(url, wait_until="domcontentloaded")

        # Get the fully rendered HTML
        html_content = await page.content()

        # Extract computed styles for each node
        computed_styles_script = """
        (function() {
            function getStyles(element) {
                let computedStyle = window.getComputedStyle(element);
                let styles = {};
                for (let i = 0; i < computedStyle.length; i++) {
                    let prop = computedStyle[i];
                    styles[prop] = computedStyle.getPropertyValue(prop);
                }
                return styles;
            }

            function extractDOMTree(element) {
                if (element.nodeType === Node.COMMENT_NODE) {
                    return { comment: element.nodeValue.trim() };
                }
                if (element.nodeType === Node.TEXT_NODE) {
                    let text = element.nodeValue.trim();
                    return text ? { text: text } : null;
                }

                let children = [];
                for (let child of element.childNodes) {
                    let parsedChild = extractDOMTree(child);
                    if (parsedChild) {
                        children.push(parsedChild);
                    }
                }

                return {
                    tag: element.tagName.toLowerCase(),
                    self_closing: element.childNodes.length === 0,
                    attributes: Object.fromEntries([...element.attributes].map(attr => [attr.name, attr.value])),
                    styles: getStyles(element),
                    children: children
                };
            }

            return extractDOMTree(document.documentElement);
        })();
        """

        dom_tree = await page.evaluate(computed_styles_script)

        await browser.close()
        return {"tag": "DocumentRoot", "attributes": {}, "styles": {}, "children": [dom_tree]}


async def main():
    if len(sys.argv) != 2:
        print("Usage: souper.py <url>")
        sys.exit(1)

    dom_tree = await fetch_and_parse_html(sys.argv[1])

    # Save to JSON file
    with open("output.json", "w", encoding="utf-8") as f:
        json.dump(dom_tree, f, indent=2, ensure_ascii=False)

    print(f"DOM tree with computed styles saved to output.json")


# Run the async function
asyncio.run(main())
