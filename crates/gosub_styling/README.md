# Gosub Styling crate

This crate holds the CSS3 styling functionality. It allows the engine to query CSS3 properties and returns the correct values based 
on the HTML and CSS3 documents parsed. 

## It consists of the following parts:

 - The CSS property definitions, as found in `resources/definitions`. This holds the syntax for each CSS property.
 - Css Value Syntax parser, which is used to parse the CSS property definitions syntax.
 - CSs Matcher that matches CSS values (ie: `1px solid red`) to the CSS property definitions.


## Still missing, but needs to be added:

 - API for querying CSS properties, and returning the correct values, minding things like inherited and initial values.
 - API for querying CSS properties at different levels. So normally we want to use the `"actual"` value, but we also need to 
   be able to query the `"computed"` value, and the `"used"` value. This is the case in for instance developer toolbars and such.


> Note that this crate might be merged later on with the gosub_ccs3 crate. 

# Css Property Definitions

The CSS property definitions are generated from the CSS3 specification and MDN, and are stored in the `resources/definitions` folder.
The tool to generate this data can be found in the `tools/generate_definitions` folder. The definitions are generated in a JSON format, 
which is then parsed on each run for now. Each property holds a `CSS Syntax Tree` that is used to match against CSS values. 

An example definition looks like this:

```json
    {
      "name": "border-clip-top",
      "syntax": "normal | [ <length-percentage [0,∞]> | <flex> ]+",
      "computed": [],
      "initial": "normal",
      "inherited": false
    }
```

This defines the `border-clip-top` property. It's syntax tells us that it can have the value `normal`, or a list of `<length-percentage>` or `<flex>` 
values. The bracketed values are actually other properties or values that are definined in the json as well. Some of these values are built-in properties, 
like `<length>`, `<string>`, `<number>` etc.

There are no computed values, meaning normally that there are no special rules for computing the value, and that there are no shorthand elements.
For instance, the `"border"` property has three computed elements: `border-color`, `border-style` and `border-width`, and thus results in three different
properties to be filled as well.

The initial value of `border-clip-top` is `normal`, and it is not inherited, so it does not inherit the value from the parent element.


# Css Value Syntax Parser 

In order to check if the CSS property value is correct, we first need to parse the value syntax for that property. Even though this syntax is fairly 
simple in setup, we use a nom-parser system (`syntax.rs`) to parse the syntax. This results in a `CSS Syntax Tree` that can be used to match against 
the CSS values. This syntax matching is done within `syntax_matcher.rs`.
