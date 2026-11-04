//! Single-line text editing for the search box.

#[derive(Default)]
pub struct TextInput {
    chars: Vec<char>,
    pub caret: usize,
    /// The other end of the selection.
    anchor: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextInput {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.chars.len();
    }

    /// Returns whether anything was selected.
    pub fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        self.chars.drain(a..b);
        self.caret = a;
        self.anchor = a;
        a != b
    }

    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        let new: Vec<char> = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let n = new.len();
        self.chars.splice(self.caret..self.caret, new);
        self.caret += n;
        self.anchor = self.caret;
    }

    fn word_left(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && !is_word(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self, from: usize) -> usize {
        let n = self.chars.len();
        let mut i = from;
        while i < n && is_word(self.chars[i]) {
            i += 1;
        }
        while i < n && !is_word(self.chars[i]) {
            i += 1;
        }
        i
    }

    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.caret == 0 {
            return;
        }
        let to = if word { self.word_left(self.caret) } else { self.caret - 1 };
        self.chars.drain(to..self.caret);
        self.caret = to;
        self.anchor = to;
    }

    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() || self.caret == self.chars.len() {
            return;
        }
        let to = if word { self.word_right(self.caret) } else { self.caret + 1 };
        self.chars.drain(self.caret..to);
    }

    pub fn move_to(&mut self, pos: usize, extend: bool) {
        self.caret = pos.min(self.chars.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn left(&mut self, word: bool, extend: bool) {
        let (a, b) = self.selection();
        let pos = if word {
            self.word_left(self.caret)
        } else if !extend && a != b {
            a
        } else {
            self.caret.saturating_sub(1)
        };
        self.move_to(pos, extend);
    }

    pub fn right(&mut self, word: bool, extend: bool) {
        let (a, b) = self.selection();
        let pos = if word {
            self.word_right(self.caret)
        } else if !extend && a != b {
            b
        } else {
            self.caret + 1
        };
        self.move_to(pos, extend);
    }

    pub fn end(&self) -> usize {
        self.chars.len()
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.chars[a..b].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(s: &str) -> TextInput {
        let mut t = TextInput::default();
        t.insert(s);
        t
    }

    #[test]
    fn typing_and_deleting() {
        let mut t = input("hello world");
        t.backspace(false);
        assert_eq!(t.text(), "hello worl");
        t.backspace(true);
        assert_eq!(t.text(), "hello ");
        t.backspace(true);
        assert_eq!(t.text(), "");
        t.backspace(false);
        t.insert("a\tb");
        assert_eq!(t.text(), "a b");
        t.move_to(0, false);
        t.delete(true);
        assert_eq!(t.text(), "b");
    }

    #[test]
    fn selection() {
        let mut t = input("微信 beta");
        t.select_all();
        assert_eq!(t.selected_text(), "微信 beta");
        t.insert("x");
        assert_eq!((t.text().as_str(), t.caret), ("x", 1));
        let mut t = input("abc def");
        t.left(true, true);
        assert_eq!(t.selected_text(), "def");
        t.left(false, false);
        assert_eq!(t.caret, 4);
        t.right(false, true);
        t.right(false, true);
        assert_eq!(t.selected_text(), "de");
        t.right(false, false);
        assert_eq!(t.caret, 6);
        t.move_to(0, false);
        t.right(true, false);
        assert_eq!(t.caret, 4);
        t.move_to(99, false);
        assert_eq!(t.caret, t.end());
    }
}
