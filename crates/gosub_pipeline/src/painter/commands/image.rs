#[derive(Clone, Debug)]
pub struct Image {
    data: Vec<u8>,
    width: u32,
    height: u32,
    // @todo: Do we need ImageFormat??
}

impl Image {
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        let expected = (width * height * 4) as usize;
        assert_eq!(
            data.len(), expected,
            "Image buffer size {} does not match {}x{}x4={}", data.len(), width, height, expected
        );
        Image { data, width, height }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}
