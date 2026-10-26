use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Mutex;
use unicode_segmentation::UnicodeSegmentation;

static CLIPBOARD: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Debug, Default)]
pub struct UndoHistory {
    undo_stack: Vec<(String, TextSelection)>,
    redo_stack: Vec<(String, TextSelection)>,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, value: &str, selection: TextSelection) {
        if self.undo_stack.last().map_or(true, |(v, _)| v != value) {
            self.undo_stack.push((value.to_string(), selection));
            if self.undo_stack.len() > 100 {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, current_value: &str, current_sel: TextSelection) -> Option<(String, TextSelection)> {
        let (prev_val, prev_sel) = self.undo_stack.pop()?;
        self.redo_stack.push((current_value.to_string(), current_sel));
        Some((prev_val, prev_sel))
    }

    pub fn redo(&mut self, current_value: &str, current_sel: TextSelection) -> Option<(String, TextSelection)> {
        let (next_val, next_sel) = self.redo_stack.pop()?;
        self.undo_stack.push((current_value.to_string(), current_sel));
        Some((next_val, next_sel))
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

thread_local! {
    static THREAD_UNDO_HISTORY: RefCell<UndoHistory> = RefCell::new(UndoHistory::default());
}

pub fn with_thread_undo_history<R>(f: impl FnOnce(&mut UndoHistory) -> R) -> R {
    THREAD_UNDO_HISTORY.with(|cell| f(&mut cell.borrow_mut()))
}

pub fn get_clipboard() -> String {
    CLIPBOARD.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn set_clipboard(text: &str) {
    if let Ok(mut lock) = CLIPBOARD.lock() {
        *lock = text.to_string();
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeyboardInput {
    pub key: String,
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSelection {
    pub anchor: usize,
    pub cursor: usize,
}

impl TextSelection {
    pub fn collapsed(cursor: usize) -> Self {
        Self {
            anchor: cursor,
            cursor,
        }
    }
    pub fn select_all(&mut self, length: usize) {
        self.anchor = 0;
        self.cursor = length;
    }
    pub fn clamp(&mut self, length: usize) {
        self.anchor = self.anchor.min(length);
        self.cursor = self.cursor.min(length);
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    pub fn move_to(&mut self, cursor: usize, extend: bool) {
        self.cursor = cursor;
        if !extend {
            self.anchor = cursor;
        }
    }
}

#[derive(Default)]
pub struct EditResult {
    pub handled: bool,
    pub changed: bool,
}

pub fn edit_text(
    value: &mut String,
    selection: &mut TextSelection,
    input: &KeyboardInput,
    multiline: bool,
    readonly: bool,
) -> EditResult {
    with_thread_undo_history(|history| {
        edit_text_with_undo(value, selection, input, multiline, readonly, history)
    })
}

pub fn edit_text_with_undo(
    value: &mut String,
    selection: &mut TextSelection,
    input: &KeyboardInput,
    multiline: bool,
    readonly: bool,
    history: &mut UndoHistory,
) -> EditResult {
    let mut chars: Vec<char> = value.chars().collect();
    let mut boundaries = vec![0];
    for grapheme in value.graphemes(true) {
        boundaries.push(boundaries.last().copied().unwrap_or(0) + grapheme.chars().count());
    }
    selection.clamp(chars.len());
    let key = input.key.as_str();
    let code = input.code.as_str();
    let word = input.ctrl || input.meta;

    if word && !input.alt && (key.eq_ignore_ascii_case("a") || code == "KeyA") {
        selection.select_all(chars.len());
        return EditResult {
            handled: true,
            changed: false,
        };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("c") || code == "KeyC") {
        let range = selection.range();
        if !range.is_empty() {
            let selected_text: String = chars[range].iter().collect();
            set_clipboard(&selected_text);
        }
        return EditResult {
            handled: true,
            changed: false,
        };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("x") || code == "KeyX") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        let range = selection.range();
        if !range.is_empty() {
            history.push(value, *selection);
            let selected_text: String = chars[range.clone()].iter().collect();
            set_clipboard(&selected_text);
            chars.drain(range.clone());
            *selection = TextSelection::collapsed(range.start);
            *value = chars.into_iter().collect();
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("v") || code == "KeyV") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        let paste = get_clipboard();
        if !paste.is_empty() {
            history.push(value, *selection);
            let range = selection.range();
            let paste_chars: Vec<char> = paste.chars().collect();
            chars.splice(range.clone(), paste_chars.iter().copied());
            *selection = TextSelection::collapsed(range.start + paste_chars.len());
            *value = chars.into_iter().collect();
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && !input.shift && (key.eq_ignore_ascii_case("z") || code == "KeyZ") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        if let Some((prev_val, prev_sel)) = history.undo(value, *selection) {
            *value = prev_val;
            *selection = prev_sel;
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && (((key.eq_ignore_ascii_case("z") || code == "KeyZ") && input.shift)
        || key.eq_ignore_ascii_case("y") || code == "KeyY")
    {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        if let Some((next_val, next_sel)) = history.redo(value, *selection) {
            *value = next_val;
            *selection = next_sel;
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("w") || code == "KeyW") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        let range = selection.range();
        let target_range = if !range.is_empty() {
            range
        } else {
            word_left(&chars, selection.cursor)..selection.cursor
        };
        if !target_range.is_empty() {
            history.push(value, *selection);
            let cursor = target_range.start;
            chars.drain(target_range);
            *selection = TextSelection::collapsed(cursor);
            *value = chars.into_iter().collect();
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("u") || code == "KeyU") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        let start = line_start(&chars, selection.cursor);
        if start < selection.cursor {
            history.push(value, *selection);
            chars.drain(start..selection.cursor);
            *selection = TextSelection::collapsed(start);
            *value = chars.into_iter().collect();
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if word && !input.alt && (key.eq_ignore_ascii_case("k") || code == "KeyK") {
        if readonly {
            return EditResult { handled: true, changed: false };
        }
        let end = line_end(&chars, selection.cursor);
        if selection.cursor < end {
            history.push(value, *selection);
            chars.drain(selection.cursor..end);
            *selection = TextSelection::collapsed(selection.cursor);
            *value = chars.into_iter().collect();
            return EditResult { handled: true, changed: true };
        }
        return EditResult { handled: true, changed: false };
    }

    if input.alt {
        return EditResult::default();
    }
    let range = selection.range();

    let move_target = match key {
        "ArrowLeft" => Some(if !input.shift && !range.is_empty() {
            range.start
        } else if word {
            word_left(&chars, selection.cursor)
        } else {
            previous_boundary(&boundaries, selection.cursor)
        }),
        "ArrowRight" => Some(if !input.shift && !range.is_empty() {
            range.end
        } else if word {
            word_right(&chars, selection.cursor)
        } else {
            next_boundary(&boundaries, selection.cursor)
        }),
        "Home" => Some(if multiline && !word {
            line_start(&chars, selection.cursor)
        } else {
            0
        }),
        "End" => Some(if multiline && !word {
            line_end(&chars, selection.cursor)
        } else {
            chars.len()
        }),
        "ArrowUp" if multiline && !word => Some(vertical(&chars, selection.cursor, false)),
        "ArrowDown" if multiline && !word => Some(vertical(&chars, selection.cursor, true)),
        _ => None,
    };
    if let Some(target) = move_target {
        selection.move_to(target, input.shift);
        return EditResult {
            handled: true,
            changed: false,
        };
    }

    let deletion = match key {
        "Backspace" => Some(if !range.is_empty() {
            range.clone()
        } else {
            (if word {
                word_left(&chars, selection.cursor)
            } else {
                previous_boundary(&boundaries, selection.cursor)
            })..selection.cursor
        }),
        "Delete" => Some(if !range.is_empty() {
            range.clone()
        } else {
            selection.cursor..(if word {
                word_right(&chars, selection.cursor)
            } else {
                next_boundary(&boundaries, selection.cursor)
            })
        }),
        _ => None,
    };
    if let Some(deletion) = deletion {
        if readonly {
            return EditResult {
                handled: true,
                changed: false,
            };
        }
        let changed = !deletion.is_empty();
        if changed {
            history.push(value, *selection);
        }
        let cursor = deletion.start;
        chars.drain(deletion);
        *selection = TextSelection::collapsed(cursor);
        if changed {
            *value = chars.into_iter().collect();
        }
        return EditResult {
            handled: true,
            changed,
        };
    }
    let inserted = if key == "Enter" && multiline && !word {
        Some("\n")
    } else if !word && is_text_input_key(key) {
        Some(key)
    } else {
        None
    };
    if let Some(inserted) = inserted {
        if readonly {
            return EditResult {
                handled: true,
                changed: false,
            };
        }
        history.push(value, *selection);
        chars.splice(range.clone(), inserted.chars());
        *selection = TextSelection::collapsed(range.start + inserted.chars().count());
        *value = chars.into_iter().collect();
        return EditResult {
            handled: true,
            changed: true,
        };
    }
    EditResult::default()
}

fn previous_boundary(boundaries: &[usize], cursor: usize) -> usize {
    boundaries
        .iter()
        .copied()
        .rev()
        .find(|offset| *offset < cursor)
        .unwrap_or(0)
}
fn next_boundary(boundaries: &[usize], cursor: usize) -> usize {
    boundaries
        .iter()
        .copied()
        .find(|offset| *offset > cursor)
        .unwrap_or(cursor)
}

fn word_left(chars: &[char], cursor: usize) -> usize {
    let prefix: String = chars[..cursor].iter().collect();
    prefix
        .split_word_bound_indices()
        .rev()
        .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
        .map_or(0, |(byte, _)| prefix[..byte].chars().count())
}
fn word_right(chars: &[char], cursor: usize) -> usize {
    let suffix: String = chars[cursor..].iter().collect();
    let mut segments = suffix.split_word_bounds();
    let Some(first) = segments.next() else {
        return cursor;
    };
    let mut advanced = first.chars().count();
    for segment in segments {
        if !segment.chars().all(char::is_whitespace) {
            break;
        }
        advanced += segment.chars().count();
    }
    cursor + advanced
}
fn line_start(chars: &[char], cursor: usize) -> usize {
    chars[..cursor]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |n| n + 1)
}
fn line_end(chars: &[char], cursor: usize) -> usize {
    chars[cursor..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |n| cursor + n)
}
fn vertical(chars: &[char], cursor: usize, down: bool) -> usize {
    let start = line_start(chars, cursor);
    let prefix: String = chars[start..cursor].iter().collect();
    let column = prefix.graphemes(true).count();
    let (target_start, target_end) = if down {
        let end = line_end(chars, cursor);
        if end == chars.len() {
            return chars.len();
        }
        let next = end + 1;
        (next, line_end(chars, next))
    } else if start == 0 {
        return 0;
    } else {
        let end = start - 1;
        (line_start(chars, end), end)
    };
    let line: String = chars[target_start..target_end].iter().collect();
    target_start
        + line
            .graphemes(true)
            .take(column)
            .map(|cluster| cluster.chars().count())
            .sum::<usize>()
}

fn is_text_input_key(key: &str) -> bool {
    !key.is_empty()
        && !key.chars().any(char::is_control)
        && !matches!(
            key,
            "ArrowLeft"
                | "ArrowRight"
                | "ArrowUp"
                | "ArrowDown"
                | "PageUp"
                | "PageDown"
                | "Home"
                | "End"
                | "Delete"
                | "Insert"
                | "Tab"
                | "Enter"
                | "Escape"
                | "Backspace"
                | "Shift"
                | "Control"
                | "Alt"
                | "Meta"
                | "CapsLock"
                | "Unidentified"
                | "Dead"
                | "NumLock"
                | "ScrollLock"
                | "Pause"
                | "PrintScreen"
                | "ContextMenu"
        )
        && !(key
            .strip_prefix('F')
            .is_some_and(|suffix| suffix.parse::<u8>().is_ok_and(|n| (1..=35).contains(&n))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_input(key: &'static str, code: &'static str, ctrl: bool, shift: bool) -> KeyboardInput {
        KeyboardInput {
            key: key.to_string(),
            code: code.to_string(),
            ctrl,
            alt: false,
            shift,
            meta: false,
        }
    }

    #[test]
    fn test_typing_and_backspace() {
        let mut value = String::new();
        let mut sel = TextSelection::collapsed(0);

        let res = edit_text(&mut value, &mut sel, &key_input("a", "KeyA", false, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "a");
        assert_eq!(sel, TextSelection::collapsed(1));

        let res = edit_text(&mut value, &mut sel, &key_input("b", "KeyB", false, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "ab");
        assert_eq!(sel, TextSelection::collapsed(2));

        let res = edit_text(&mut value, &mut sel, &key_input("Backspace", "Backspace", false, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "a");
        assert_eq!(sel, TextSelection::collapsed(1));
    }

    static TEST_CLIPBOARD_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_select_all_and_copy_cut_paste() {
        let _lock = TEST_CLIPBOARD_MUTEX.lock().unwrap();
        let mut value = "hello world".to_string();
        let mut sel = TextSelection::collapsed(5);

        let res = edit_text(&mut value, &mut sel, &key_input("a", "KeyA", true, false), false, false);
        assert!(res.handled);
        assert_eq!(sel.range(), 0..11);

        let res = edit_text(&mut value, &mut sel, &key_input("c", "KeyC", true, false), false, false);
        assert!(res.handled && !res.changed);
        assert_eq!(get_clipboard(), "hello world");

        let res = edit_text(&mut value, &mut sel, &key_input("x", "KeyX", true, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "");
        assert_eq!(sel, TextSelection::collapsed(0));

        let res = edit_text(&mut value, &mut sel, &key_input("v", "KeyV", true, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "hello world");
        assert_eq!(sel, TextSelection::collapsed(11));
    }

    #[test]
    fn test_layout_independent_shortcuts() {
        let _lock = TEST_CLIPBOARD_MUTEX.lock().unwrap();
        let mut value = "hello test".to_string();
        let mut sel = TextSelection::collapsed(4);

        let res = edit_text(&mut value, &mut sel, &key_input("q", "KeyA", true, false), false, false);
        assert!(res.handled);
        assert_eq!(sel.range(), 0..10);

        let res = edit_text(&mut value, &mut sel, &key_input("j", "KeyC", true, false), false, false);
        assert!(res.handled);
        assert_eq!(get_clipboard(), "hello test");
    }

    #[test]
    fn test_undo_redo() {
        let mut history = UndoHistory::new();
        let mut value = "init".to_string();
        let mut sel = TextSelection::collapsed(4);

        edit_text_with_undo(&mut value, &mut sel, &key_input("!", "Digit1", false, false), false, false, &mut history);
        assert_eq!(value, "init!");

        let res = edit_text_with_undo(&mut value, &mut sel, &key_input("z", "KeyZ", true, false), false, false, &mut history);
        assert!(res.handled && res.changed);
        assert_eq!(value, "init");

        let res = edit_text_with_undo(&mut value, &mut sel, &key_input("y", "KeyY", true, false), false, false, &mut history);
        assert!(res.handled && res.changed);
        assert_eq!(value, "init!");
    }

    #[test]
    fn test_line_and_word_deletion() {
        let mut value = "first second third".to_string();
        let mut sel = TextSelection::collapsed(18);

        let res = edit_text(&mut value, &mut sel, &key_input("w", "KeyW", true, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "first second ");

        let res = edit_text(&mut value, &mut sel, &key_input("u", "KeyU", true, false), false, false);
        assert!(res.handled && res.changed);
        assert_eq!(value, "");
    }
}
