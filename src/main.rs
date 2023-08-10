use std::fs::File;
mod parser;
use parser::input_stream::{InputStream, Confidence, Encoding};

fn main() {
    let file = File::open("hello.html").expect("could not open file");

    let mut stream = InputStream::new();
    stream.read_from_file(file).expect("can't read from file");
    stream.set_encoding(Encoding::UTF8, Confidence::Certain);

    stream.reset();
    println!("{}", stream.read_char());
    println!("{}", stream.read_char());
    println!("{}", stream.read_char());
    println!("{}", stream.read_char());

    stream.reset();
    println!("{}", stream.read_char());
    println!("{}", stream.read_char());

    println!("{}", stream.eof());
}
