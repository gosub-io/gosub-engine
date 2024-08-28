# CSS properties and parsing

CSS is not easy.. It looks deceivingly simple from the outside, but there are a lot of things to consider when parsing 
and calculating CSS properties. This document will explain how the CSS properties are parsed and calculated in the 
engine. 

First of all, we need to convert stylesheets (inline stylesheets, external stylesheets
and even style attributes on elements) from a textual representation into a structured representation. This is done by
parsing the CSS data into an Abstract Syntax Tree (AST). The AST is then converted into a `CSSStylesheet` structure that
allows easy usage in the code later on.

A `CssStylesheet` consists of a set of CSS rules, and some information about where this stylesheet is loaded from (is 
it an external stylesheet, and if so, what is its url etc.).

Css rules are contained in a list of `CssRule` structures. A rule has a list of selectors, and a list of declarations.
The selectors are used to match elements in the HTML document, and the declarations are the actual CSS properties that
are applied to the matched elements.

The `CssSelector` itself consists of a list of `CssSelectorParts` structures, and a `CssDeclaration` consists of the 
property name, the value as a `CssValue` struct and a boolean to represent if the importanct flag has been set on this 
property (ie: `color: black !important`)


## CSS Values
When the engine deals with CSS properties, it will only handle `CSSValue` structures. It doesn't handle strings so 
if there are strings they must be converted to `CSSValue` structures first.

The `CSSValue` struct has some functions to parse strings or even AST nodes from stylesheets into a `CSSValue` struct.
For this you can use the `parse_ast_node()` and `parse_str()` functions.

There are many different CssValue types:

```
    None,
    Color(RgbColor),
    Number(f32),
    Percentage(f32),
    String(String),
    Unit(f32, String),
    Function(String, Vec<CssValue>),
    List(Vec<CssValue>),
    Initial,
    Inherit,
```

The parser functions will parse any value to their correct type. Note that when there are multiple values in a property
(ie: `border: 1px solid black`) the parser will return a `List` of `CssValue` structures.


# Syntax checking
At this point, we can convert a stylesheet to a rust structure. But this doesn't mean that the stylesheet is correct.
For instance, the following will happily be parsed by the system:

```
    div {
        color: thisisnotacolor;
        border: 5%;
        background-color: 10deg 20px solid;
    }
```

So, in order to understand if a stylesheet is correct, we need to do some syntax checking. For this we need some 
external help. Each CSS property has a list of possible values that it can accept. For instance, the `color` property
can accept a color value, but also the `initial` and `inherit` values. A color value by itself can be a hex value, a
rgb value, a hsl value, a keyword etc.

With the help of the CSS specifications, we extracted all possible values for each CSS property and put them in a
definition file. This file is used to check if a value is correct for a certain property. These possible values are 
a language on its own, and we have a parser for this language that can parse a string into a structure that represents
the syntax.

This is done in the `syntax.rs` file and we use a nom parser to compile this language into a `CssSyntaxTree` structure.

Specification of the "language" can be found at: https://developer.mozilla.org/en-US/docs/Web/CSS/Value_definition_syntax

So for instance:

```
    <color> | transparent | currentcolor | <image>
```

means it a value can only match if its either be a color (a typedef), the word `transparent`, the word `currentcolor` 
or an image (another typedef). These typedefs are also defined in the CSS specifications.

### Typedefs
The CSS specifications also define some typedefs. For instance, the `color` typedef is defined as:

```
    "<color>": "<color-base> | currentColor | <system-color> ",
```

where `color-base`, and `system-color` are again typedefs. The system resolves all typedefs when compiling a property 
syntax.

Browsing the CSS specifications for typedefs is no fun. Fortunately, there is a page
https://www.w3.org/Style/CSS/all-properties.en.json that contains all CSS properties and where their syntax and 
typedefs are defined. We have an external repository (https://github.com/gosub-browser/css-definition-generator) that
parses this file and generates the `css_definitions.json` and `css_typedefs.json` files for us.


 






