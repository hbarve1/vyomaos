// Tests for word-wrap logic.
// We extract the wrapper into a pure function so it's testable without rendering.

/// Pure word-wrap helper — duplicated here because supervisor is a binary crate.
fn wrap_words(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    lines.push(current);  // always push, even if empty
    lines
}

#[test]
fn empty_string_gives_one_empty_line() {
    assert_eq!(wrap_words("", 10), vec![""]);
}

#[test]
fn single_word_fits_on_one_line() {
    assert_eq!(wrap_words("hello", 10), vec!["hello"]);
}

#[test]
fn two_words_fit_on_one_line() {
    assert_eq!(wrap_words("hello world", 11), vec!["hello world"]);
}

#[test]
fn two_words_split_across_lines() {
    assert_eq!(wrap_words("hello world", 5), vec!["hello", "world"]);
}

#[test]
fn max_chars_zero_returns_full_text() {
    assert_eq!(wrap_words("hello world", 0), vec!["hello world"]);
}

#[test]
fn long_single_word_not_split() {
    // A word longer than max_chars is placed on its own line without truncation
    assert_eq!(wrap_words("superlongword", 5), vec!["superlongword"]);
}

#[test]
fn three_words_two_fit_then_one() {
    // "aa bb" = 5 chars fits in 5, "cc" goes to next line
    assert_eq!(wrap_words("aa bb cc", 5), vec!["aa bb", "cc"]);
}

#[test]
fn sentence_wraps_correctly() {
    let result = wrap_words("the quick brown fox", 9);
    assert_eq!(result, vec!["the quick", "brown fox"]);
}

#[test]
fn exactly_max_chars_stays_on_one_line() {
    // "ab cd" = 5 chars, max_chars = 5 → fits exactly
    assert_eq!(wrap_words("ab cd", 5), vec!["ab cd"]);
}
