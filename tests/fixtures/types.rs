pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub struct Pair(i32, i32);
pub struct Marker;

pub struct Container<T> {
    pub value: T,
}

pub fn identity<T>(value: T) -> T {
    value
}

pub type Points = Vec<Point>;
pub const MAX_POINTS: usize = 12;
