use anyhow::{bail, Result};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::ops::Range;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub inserted: String,
    pub deleted: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditTransaction {
    pub label: String,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Default)]
pub struct UndoRedo {
    undo: Vec<EditTransaction>,
    redo: Vec<EditTransaction>,
}

impl UndoRedo {
    pub fn push(&mut self, tx: EditTransaction) {
        if !tx.edits.is_empty() {
            self.undo.push(tx);
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[derive(Debug)]
pub struct EditBuffer {
    rope: Rope,
    undo_redo: UndoRedo,
    dirty: bool,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EditBuffer {
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            undo_redo: UndoRedo::default(),
            dirty: false,
        }
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            undo_redo: UndoRedo::default(),
            dirty: false,
        }
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn slice(&self, range: Range<usize>) -> Result<String> {
        self.validate_range(range.clone())?;
        Ok(self.rope.slice(range).to_string())
    }

    pub fn insert(&mut self, char_idx: usize, text: &str) -> Result<()> {
        self.validate_index(char_idx)?;
        let edit = Edit {
            range: char_idx..char_idx,
            inserted: text.to_string(),
            deleted: String::new(),
        };
        self.apply_forward_edit(&edit)?;
        self.undo_redo.push(EditTransaction {
            label: "insert".into(),
            edits: vec![edit],
        });
        self.dirty = true;
        Ok(())
    }

    pub fn delete(&mut self, range: Range<usize>) -> Result<()> {
        self.validate_range(range.clone())?;
        let deleted = self.rope.slice(range.clone()).to_string();
        let edit = Edit {
            range,
            inserted: String::new(),
            deleted,
        };
        self.apply_forward_edit(&edit)?;
        self.undo_redo.push(EditTransaction {
            label: "delete".into(),
            edits: vec![edit],
        });
        self.dirty = true;
        Ok(())
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<()> {
        self.validate_range(range.clone())?;
        let deleted = self.rope.slice(range.clone()).to_string();
        let edit = Edit {
            range,
            inserted: text.to_string(),
            deleted,
        };
        self.apply_forward_edit(&edit)?;
        self.undo_redo.push(EditTransaction {
            label: "replace".into(),
            edits: vec![edit],
        });
        self.dirty = true;
        Ok(())
    }

    pub fn apply_transaction(
        &mut self,
        label: impl Into<String>,
        mut edits: Vec<Edit>,
    ) -> Result<()> {
        if edits.is_empty() {
            return Ok(());
        }
        edits.sort_by_key(|edit| Reverse(edit.range.start));
        for edit in &edits {
            self.validate_range(edit.range.clone())?;
        }
        for edit in &edits {
            self.apply_forward_edit(edit)?;
        }
        self.undo_redo.push(EditTransaction {
            label: label.into(),
            edits,
        });
        self.dirty = true;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool> {
        let Some(tx) = self.undo_redo.undo.pop() else {
            return Ok(false);
        };
        for edit in tx.edits.iter().rev() {
            self.apply_inverse_edit(edit)?;
        }
        self.undo_redo.redo.push(tx);
        self.dirty = true;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool> {
        let Some(tx) = self.undo_redo.redo.pop() else {
            return Ok(false);
        };
        for edit in &tx.edits {
            self.apply_forward_edit(edit)?;
        }
        self.undo_redo.undo.push(tx);
        self.dirty = true;
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        self.undo_redo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo_redo.can_redo()
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn trim_trailing_whitespace(&mut self) -> Result<()> {
        let text = self.text();
        let mut edits = Vec::new();
        let mut char_cursor = 0usize;
        for line in text.split_inclusive('\n') {
            let without_newline = line.strip_suffix('\n').unwrap_or(line);
            let trimmed = without_newline.trim_end_matches([' ', '\t']);
            if trimmed.len() != without_newline.len() {
                let delete_start = char_cursor + trimmed.chars().count();
                let delete_end = char_cursor + without_newline.chars().count();
                edits.push(Edit {
                    range: delete_start..delete_end,
                    inserted: String::new(),
                    deleted: without_newline[trimmed.len()..].to_string(),
                });
            }
            char_cursor += line.chars().count();
        }
        self.apply_transaction("trim trailing whitespace", edits)
    }

    fn apply_forward_edit(&mut self, edit: &Edit) -> Result<()> {
        let deleted_chars = edit.deleted.chars().count();
        if deleted_chars > 0 {
            self.validate_range(edit.range.start..edit.range.start + deleted_chars)?;
            self.rope
                .remove(edit.range.start..edit.range.start + deleted_chars);
        }
        if !edit.inserted.is_empty() {
            self.validate_index(edit.range.start)?;
            self.rope.insert(edit.range.start, &edit.inserted);
        }
        Ok(())
    }

    fn apply_inverse_edit(&mut self, edit: &Edit) -> Result<()> {
        let inserted_chars = edit.inserted.chars().count();
        if inserted_chars > 0 {
            self.validate_range(edit.range.start..edit.range.start + inserted_chars)?;
            self.rope
                .remove(edit.range.start..edit.range.start + inserted_chars);
        }
        if !edit.deleted.is_empty() {
            self.validate_index(edit.range.start)?;
            self.rope.insert(edit.range.start, &edit.deleted);
        }
        Ok(())
    }

    fn validate_index(&self, idx: usize) -> Result<()> {
        if idx > self.len_chars() {
            bail!("char index {idx} out of bounds {}", self.len_chars());
        }
        Ok(())
    }

    fn validate_range(&self, range: Range<usize>) -> Result<()> {
        if range.start > range.end || range.end > self.len_chars() {
            bail!(
                "char range {}..{} out of bounds {}",
                range.start,
                range.end,
                self.len_chars()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_undo_and_redo() {
        let mut buffer = EditBuffer::from_text("hello world");
        buffer.replace(6..11, "FastPad").unwrap();
        assert_eq!(buffer.text(), "hello FastPad");

        assert!(buffer.undo().unwrap());
        assert_eq!(buffer.text(), "hello world");
        assert!(buffer.redo().unwrap());
        assert_eq!(buffer.text(), "hello FastPad");
    }

    #[test]
    fn handles_unicode_char_indices() {
        let mut buffer = EditBuffer::from_text("aé🙂");
        buffer.insert(2, "x").unwrap();
        assert_eq!(buffer.text(), "aéx🙂");
    }

    #[test]
    fn transaction_applies_original_ranges_from_end_to_start() {
        let mut buffer = EditBuffer::from_text("alpha beta gamma");
        buffer
            .apply_transaction(
                "multi replace",
                vec![
                    Edit {
                        range: 0..5,
                        inserted: "one".into(),
                        deleted: "alpha".into(),
                    },
                    Edit {
                        range: 11..16,
                        inserted: "three".into(),
                        deleted: "gamma".into(),
                    },
                ],
            )
            .unwrap();

        assert_eq!(buffer.text(), "one beta three");
        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "alpha beta gamma");
    }
}
