import requests
import json

def generate_css_syntax():
    r = requests.get("https://raw.githubusercontent.com/mdn/data/main/css/syntaxes.json")
    if r.status_code != 200:
        print('Failed to fetch data from MDN')
        return

    result = {}
    for function_name, data in r.json().items():
        result[function_name] = data.get("syntax")

    with open('mdn_css_syntax.json', 'w') as f:
        f.write(json.dumps(result, indent=2))

def generate_css_functions():
    r = requests.get("https://raw.githubusercontent.com/mdn/data/main/css/functions.json")
    if r.status_code != 200:
        print('Failed to fetch data from MDN')
        return

    result = []
    for name, data in r.json().items():
        result.append({
            "name": name,
            "syntax": data.get("syntax"),
            "mdn_url": f"https://developer.mozilla.org/en-US/docs/Web/CSS/{name}",
        })

    with open('mdn_css_functions.json', 'w') as f:
        f.write(json.dumps(result, indent=2))

def generate_css_properties():
    r = requests.get("https://raw.githubusercontent.com/mdn/data/main/css/properties.json")
    if r.status_code != 200:
        print('Failed to fetch data from MDN')
        return

    result = []
    for name, data in r.json().items():
        result.append({
            "name": name,
            "syntax": data.get("syntax"),
            "computed": data.get("computed"),
            "initial": data.get("initial"),
            "inherited": data.get("inherited"),
            "mdn_url": f"https://developer.mozilla.org/en-US/docs/Web/CSS/{name}",
        })

    with open('mdn_css_properties.json', 'w') as f:
        f.write(json.dumps(result, indent=2))

if __name__ == '__main__':
    generate_css_properties()
    generate_css_syntax()
    generate_css_functions()
