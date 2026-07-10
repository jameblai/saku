//! Split long text into Discord-safe message chunks.

const DISCORD_LIMIT: usize = 2000;

pub fn chunk_message(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.len() + line.len() > DISCORD_LIMIT {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            if line.len() > DISCORD_LIMIT {
                for piece in line.as_bytes().chunks(DISCORD_LIMIT) {
                    chunks.push(String::from_utf8_lossy(piece).into_owned());
                }
            } else {
                current.push_str(line);
            }
        } else {
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_message_one_chunk() {
        assert_eq!(chunk_message("hi"), vec!["hi".to_string()]);
    }

    #[test]
    fn long_message_splits() {
        let text = "a".repeat(2500);
        let chunks = chunk_message(&text);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.len() <= DISCORD_LIMIT));
    }
}
