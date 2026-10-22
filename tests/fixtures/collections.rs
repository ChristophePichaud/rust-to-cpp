pub fn total(values: Vec<i32>) -> i32 {
    values.into_iter().sum()
}

pub fn first_or(values: Vec<i32>, fallback: i32) -> i32 {
    values.first().copied().unwrap_or(fallback)
}
