pub trait Measure {
    fn measure(&self) -> i32;
}

pub struct BoxedValue {
    pub value: i32,
}

impl Measure for BoxedValue {
    fn measure(&self) -> i32 {
        self.value
    }
}
