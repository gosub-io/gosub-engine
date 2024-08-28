# CSS Style calculator

The css style calculator is the middleman between CSS data and DOM documentation. It is used to generate 
all the CSS properties per node. 

### Step 1:
The HTML5 parser detects external CSS links and inline CSS styles. This data is passed to the 
CSS parser which in turn returns a CSS AST (Abstract Syntax Tree) and finally turned into a 
CSSStylesheet structure that allows easy usage in the code later on. 

### Step 2:
Once all sheets are collected, the CSS styles module will try and find all the declared values 
per document node (html elements). This is done by iterating the whole document tree and each 
CSS stylesheet. This will ultimately result in a list of "declared" CSS properties per node. These
are stored separately in a CSS Map and not directly in the document object itself.

### Step 3:
Once we have all the declared properties, the CSS style calculator must figure out which of these 
values will "win". This is done by calculating the specificity of each selector and by checking
the importance flag and the priority of the stylesheet. Ultimately, once single value is chosen as 
the "cascaded" value for each property. This again is stored in the CSS map and not per document.

### Step 4:
There must be more steps in order to calculate the final styles for each node. This is not done 
yet as we are still in the process of implementing the render pipeline in which this will take place.


## Calculating CSS values

There are 5 steps in calculating the final CSS values for each node:

1. Find declared values
2. Find cascaded value
3. Find specified value (not yet implemented)
4. Find computed value (not yet implemented)
5. Find used value (not yet implemented)
6. Find actual value (not yet implemented)


# Using CSS:

To use the CSS properties, you can simply check through the CSS map:

```
// Retrieve the properties for the given node
let props = style_calculator.get_properties(node.id);

// Check if property exists
if props.exists("color") {
    let color = props.get("color").unwrap();    
    // do something with the color
}
```


### Shorthand properties
There is a list of shorthand properties that are expanded into multiple properties. For example,
the property "background" is expanded into "background-color", "background-image", "background-repeat", 
"background-attachment", "background-position", and "background-size".

These shorthand properties are currently not expanded yet, but are defined in the styles module.


### Property list
There is a set of properties that we need to know if they are inheritable and if they have default values.
If there are no declared values (and thus not a selected value) for a property, the default value is used.

This is currently not yet implemented.


### CSS colors
The CSS color module specifies a list of color names and their corresponding RGB values. This list is 
declared and can be used to convert color names to RGB values.

(note: we miss "rebeccapurple" in the list, which is a valid color name in CSS3, so we know that we 
don't have all the color names yet)