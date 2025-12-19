//! Undo/redo operations.

use crate::types::{EditOperation, UndoEntry}; // undo/redo types
use super::Editor; // main editor logic
use anyhow::Result; // anyhow error handling

impl Editor {
    /// Record an edit for undo.
    pub fn record_edit(&mut self, op: EditOperation) {
        const CAP: usize = 1000;
        let entry = UndoEntry {
            op,
            cursor_before: self.cursor,
            anchor_before: self.anchor,
        };
        self.undo.push(entry);
        if self.undo.len() > CAP {
            self.undo.drain(0..(self.undo.len() - CAP));
        }
        self.redo.clear();
        self.highlighter.invalidate_all();
    }

    /// Undo the most recent edit.
    pub fn undo(&mut self) -> Result<()> {
        if let Some(entry) = self.undo.pop() {
            let redo_op = match &entry.op {
                EditOperation::Insert { pos, text } => {
                    let end = self.buf.calc_end_pos(*pos, text);
                    self.buf.delete_range(*pos, end);
                    EditOperation::Delete { start: *pos, _end: end, deleted_text: text.clone() }
                }
                EditOperation::Delete { start, _end: _, deleted_text } => {
                    self.buf.insert_str(*start, deleted_text);
                    EditOperation::Insert { pos: *start, text: deleted_text.clone() }
                }
            };

            self.redo.push(UndoEntry {
                op: redo_op,
                cursor_before: self.cursor,
                anchor_before: self.anchor,
            });

            self.cursor = entry.cursor_before;
            self.anchor = entry.anchor_before;
            self.dirty = true;
            self.highlighter.invalidate_all();
            self.mark_redraw();
            self.ensure_visible()?;
        }
        Ok(())
    }

    /// Redo the most recently undone edit.
    pub fn redo(&mut self) -> Result<()> {
        if let Some(entry) = self.redo.pop() {
            let undo_op = match &entry.op {
                EditOperation::Insert { pos, text } => {
                    let end = self.buf.calc_end_pos(*pos, text);
                    self.buf.delete_range(*pos, end);
                    EditOperation::Delete { start: *pos, _end: end, deleted_text: text.clone() }
                }
                EditOperation::Delete { start, _end: _, deleted_text } => {
                    self.buf.insert_str(*start, deleted_text);
                    EditOperation::Insert { pos: *start, text: deleted_text.clone() }
                }
            };

            self.undo.push(UndoEntry {
                op: undo_op,
                cursor_before: self.cursor,
                anchor_before: self.anchor,
            });

            self.cursor = entry.cursor_before;
            self.anchor = entry.anchor_before;
            self.dirty = true;
            self.highlighter.invalidate_all();
            self.mark_redraw();
            self.ensure_visible()?;
        }
        Ok(())
    }
}
