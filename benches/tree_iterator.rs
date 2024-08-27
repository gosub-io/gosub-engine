use std::fs::File;

use criterion::{criterion_group, criterion_main, Criterion};
use gosub_html5::node::NodeId;
use gosub_html5::parser::document::{Document, DocumentBuilder, TreeIterator};
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};

fn wikipedia_main_page(c: &mut Criterion) {
    // Criterion can report inconsistent results from run to run in some cases.  We attempt to
    // minimize that in this setup.
    // https://stackoverflow.com/a/74136347/61048
    let mut group = c.benchmark_group("Tree Iterator");
    group.significance_level(0.1).sample_size(500);

    let html_file = File::open("tests/data/tree_iterator/wikipedia_main.html").unwrap();
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    let _ = stream.read_from_file(html_file);

    let main_document = DocumentBuilder::new_document(None);
    let document = Document::clone(&main_document);
    let _ = Html5Parser::parse_document(&mut stream, document, None);

    group.bench_function("wikipedia main page", |b| {
        b.iter(|| {
            let tree_iterator = TreeIterator::new(&main_document);
            let _ = tree_iterator.collect::<Vec<NodeId>>();
        })
    });

    group.finish();
}

fn stackoverflow_home(c: &mut Criterion) {
    // Criterion can report inconsistent results from run to run in some cases.  We attempt to
    // minimize that in this setup.
    // https://stackoverflow.com/a/74136347/61048
    let mut group = c.benchmark_group("Tree Iterator");
    group.significance_level(0.1).sample_size(500);

    // using the main page of (english) wikipedia as a rough estimate of traversing a decently sized website
    let html_file = File::open("tests/data/tree_iterator/stackoverflow.html").unwrap();
    let mut bytestream = ByteStream::new(Encoding::UTF8, None);
    let _ = bytestream.read_from_file(html_file);

    let main_document = DocumentBuilder::new_document(None);
    let document = Document::clone(&main_document);
    let _ = Html5Parser::parse_document(&mut bytestream, document, None);

    group.bench_function("stackoverflow home", |b| {
        b.iter(|| {
            let tree_iterator = TreeIterator::new(&main_document);
            let _ = tree_iterator.collect::<Vec<NodeId>>();
        })
    });

    group.finish();
}

criterion_group!(benches, wikipedia_main_page, stackoverflow_home);
criterion_main!(benches);
