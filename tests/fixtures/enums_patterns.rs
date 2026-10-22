pub enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(String),
}

pub fn message_size(message: Message) -> i32 {
    match message {
        Message::Quit => 0,
        Message::Move { x, y } => x + y,
        Message::Write(text) => text.len() as i32,
    }
}
