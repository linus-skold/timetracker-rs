/// A single-line editable string with a cursor held as a *char* index.
/// Shared by the six entry-form fields and the search bar.
#[derive(Debug, Default, Clone)]
pub(crate) struct TextInput {
    value: String,
    /// Char index, not a byte index; clamped to the value's length on use.
    cursor: usize,
}

impl TextInput {
    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Replace the text, leaving the cursor at the end.
    pub(crate) fn set_from(&mut self, s: &str) {
        self.value.clear();
        self.value.push_str(s);
        self.cursor_to_end();
    }

    pub(crate) fn cursor_to_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    /// The text left of the cursor — what the renderer measures to place it.
    pub(crate) fn before_cursor(&self) -> &str {
        &self.value[..self.byte_idx(self.cursor)]
    }

    pub(crate) fn insert(&mut self, c: char) {
        let at = self.byte_idx(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    pub(crate) fn backspace(&mut self) {
        let pos = self.clamped();
        if pos == 0 {
            return;
        }
        let start = self.byte_idx(pos - 1);
        let end = self.byte_idx(pos);
        self.value.drain(start..end);
        self.cursor = pos - 1;
    }

    pub(crate) fn left(&mut self) {
        self.cursor = self.clamped().saturating_sub(1);
    }

    pub(crate) fn right(&mut self) {
        let len = self.value.chars().count();
        self.cursor = (self.clamped() + 1).min(len);
    }

    /// Jump left past whitespace/punctuation, then past the preceding word.
    pub(crate) fn word_left(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut pos = self.cursor.min(chars.len());
        while pos > 0 && !chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        self.cursor = pos;
    }

    /// Jump right past the current word, then past any trailing whitespace/punctuation.
    pub(crate) fn word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor.min(len);
        while pos < len && chars[pos].is_alphanumeric() {
            pos += 1;
        }
        while pos < len && !chars[pos].is_alphanumeric() {
            pos += 1;
        }
        self.cursor = pos;
    }

    fn clamped(&self) -> usize {
        self.cursor.min(self.value.chars().count())
    }

    /// Byte offset of char index `pos`, or the end of the string past the last char.
    fn byte_idx(&self, pos: usize) -> usize {
        self.value
            .char_indices()
            .nth(pos)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }
}

#[cfg(test)]
mod tests {
    use super::TextInput;

    fn input(s: &str, cursor: usize) -> TextInput {
        let mut t = TextInput::default();
        t.set_from(s);
        t.cursor = cursor;
        t
    }

    #[test]
    fn set_from_puts_cursor_at_end() {
        let t = input("hello", 5);
        assert_eq!(t.value(), "hello");
        assert_eq!(t.cursor, 5);
    }

    #[test]
    fn insert_at_cursor() {
        let mut t = input("ac", 1);
        t.insert('b');
        assert_eq!(t.value(), "abc");
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn insert_between_multibyte_chars() {
        let mut t = input("åä", 1);
        t.insert('x');
        assert_eq!(t.value(), "åxä");
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn backspace_removes_multibyte_char() {
        let mut t = input("åäö", 2);
        t.backspace();
        assert_eq!(t.value(), "åö");
        assert_eq!(t.cursor, 1);
    }

    #[test]
    fn backspace_at_start_is_a_no_op() {
        let mut t = input("abc", 0);
        t.backspace();
        assert_eq!(t.value(), "abc");
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn backspace_clamps_an_overrun_cursor() {
        let mut t = input("ab", 9);
        t.backspace();
        assert_eq!(t.value(), "a");
        assert_eq!(t.cursor, 1);
    }

    #[test]
    fn left_and_right_clamp_at_the_ends() {
        let mut t = input("日本語", 0);
        t.left();
        assert_eq!(t.cursor, 0);
        t.right();
        t.right();
        t.right();
        t.right();
        assert_eq!(t.cursor, 3);
        t.left();
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn word_left_skips_separators_then_the_word() {
        let mut t = input("hello world", 11);
        t.word_left();
        assert_eq!(t.cursor, 6);
        t.word_left();
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn word_right_skips_the_word_then_separators() {
        let mut t = input("hello world", 0);
        t.word_right();
        assert_eq!(t.cursor, 6);
        t.word_right();
        assert_eq!(t.cursor, 11);
    }

    #[test]
    fn word_jumps_count_chars_not_bytes() {
        let mut t = input("héllo wörld", 11);
        t.word_left();
        assert_eq!(t.cursor, 6);
        t.word_right();
        assert_eq!(t.cursor, 11);
    }

    #[test]
    fn before_cursor_splits_on_a_char_boundary() {
        let t = input("åäö", 2);
        assert_eq!(t.before_cursor(), "åä");
    }

    #[test]
    fn clear_resets_the_cursor() {
        let mut t = input("abc", 3);
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.cursor, 0);
    }
}
