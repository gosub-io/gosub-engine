# Parsing HTML5 sites

Parsing a HTML5 site is not difficult, although it currently require some manual work. Later on, this will be encapsulated in the engine API.

First, we need to fetch the actual HTML content. This can be done by a simple HTTP request, or reading a file from disk. These HTML bytes must be 
passed to the byte streamer so it can be converted to tokens without worrying about the encoding:

```rust
    let stream = &mut ByteStream::new(Encoding::UTF8, None);
```

Here, the `stream` points to a string containing the HTML content. The `ByteStream` will take care of converting the bytes to characters, and handle the encoding.
We assume UTF-8 here, but other encodings could be supported later on as well.

Next, we need to create a document, which will be the main object that will be filled by the parser. The document will contain all the node elements and other 
data that is generated during the parsing of the HTML. This also includes any stylesheets that are found, both internally and externally.
    
```rust
    let document = DocumentBuilder::new_document();
```

Note that a document itself isn't a document, but a HANDLE to a document (a `DocumentHandle`). Once we have our document handle, we can start the parser
by calling the `parse_document` method on the `Html5Parser` struct. This method will return a list of parse errors, if any. 

```rust
    let parse_errors = Html5Parser::parse_document(&mut stream, Document::clone(&document), None)?;

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }
```

If there are any errors during parsing, they will be added to the parse_errors list. These errors can be printed to the console, or handled in any other way.

Finally, we can do whatever we need to do with the document. Normally it will be used to render the HTML by passing it into a render pipeline, but for now
we can simply print the document. This will output a tree-like structure of all node and text elements found in the document.

```rust
    println!("Generated tree: \n\n {document}");
```

It is possible to traverse the document tree with a visitor pattern. You can create a visitor struct that implements the `NodeVisitor` trait, and then pass
it to the `visit` method.

```rust
    let mut visitor = Box::new(TextVisitor::default()) as Box<dyn Visitor<Node>>;
    visit(&Document::clone(&document), &mut visitor);
```

A simple visitor could hide all non-renderable nodes, or change the text color based on the CSS properties, or even generate colored links for `<a>` tags.