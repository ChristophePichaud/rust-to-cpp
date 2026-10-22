pub fn sum_to(limit: i32) -> i32 {
    let mut sum = 0;
    for value in 0..limit {
        if value % 2 == 0 {
            sum += value;
        }
    }
    sum
}
