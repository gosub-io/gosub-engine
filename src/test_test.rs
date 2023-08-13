pub struct InputStream {
}

impl InputStream {
    pub fn new() -> Self {
        InputStream {}
    }
}

// =======================================================================================

pub struct Token;

impl Token {
    fn to_string(&self) -> String {
        return String::from("token");
    }
}

// =======================================================================================

pub struct Tokenizer<'a> {
    pub stream: &'a mut InputStream,
    pub emitter: &'a mut dyn Emitter,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a mut InputStream, emitter: &'a mut dyn Emitter) -> Self {
        return Tokenizer {
            stream: input,
            emitter,
        }
    }

    pub fn next_token(&mut self)
    {
        let t = Token;
        self.emitter.emit(t)
    }
}

// =======================================================================================

pub struct HtmlParser<'a> {
    pub tokenizer: &'a mut Tokenizer<'a>,
}

impl<'a> HtmlParser<'a> {
    pub fn new(tokenizer: &'a mut Tokenizer<'a>) -> Self {
        HtmlParser{
            tokenizer
        }
    }

    pub fn get_tokenizer(&mut self) -> &mut Tokenizer<'a> {
        return self.tokenizer;
    }
}

// =======================================================================================

pub trait Emitter {
    fn emit(&mut self, t: Token);
}

pub struct StrEmitter {
    pub output: String
}

impl StrEmitter {
    pub fn new() -> Self {
        StrEmitter {
            output: String::new(),
        }
    }

    fn get_output(&self) -> &String {
        return &self.output;
    }
}

impl Emitter for StrEmitter {
    fn emit(&mut self, t: Token) {
        self.output.push_str(&*t.to_string());
    }
}

pub struct AppEmitter;

impl AppEmitter {
    pub fn new() -> Self {
        AppEmitter
    }
}

impl Emitter for AppEmitter {
    fn emit(&mut self, t: Token) {
        println!("O [{}]", t.to_string());
    }
}

// =======================================================================================

pub fn main() {
    let mut is = InputStream::new();
    let mut e = AppEmitter::new();
    let mut t = Tokenizer::new(&mut is, &mut e);

    let mut p = HtmlParser::new(&mut t);

    p.get_tokenizer().next_token();
    // println!("Output: {}", e.get_output())
}
