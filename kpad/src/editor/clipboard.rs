//! Clipboard operations: copy, cut, paste.

use crate::types::EditOperation; // document edit operations
use super::Editor; // main editor logic
use anyhow::Result; // anyhow error handling
use std::time::Duration; // timing for status messages

impl Editor {
    /// Copy to clipboard.
    pub fn copy(&mut self) -> Result<()> {
        let text = self.selected_text();
        if text.is_empty() {
            self.set_status("Nothing selected to copy.", Duration::from_secs(2));
            return Ok(());
        }
        if let Some(cb) = &mut self.clipboard {
            cb.set_text(text).ok();
            self.set_status("Copied selection.", Duration::from_secs(2));
        } else {
            self.set_status("Clipboard unavailable.", Duration::from_secs(2));
        }
        Ok(())
    }

    /// Cut to clipboard.
    pub fn cut(&mut self) -> Result<()> {
        let text = self.selected_text();
        if text.is_empty() {
            self.set_status("Nothing selected to cut.", Duration::from_secs(2));
            return Ok(());
        }

        let (a, b) = self.selection_range().unwrap();
        let op = EditOperation::Delete { start: a, _end: b, deleted_text: text.clone() };
        self.record_edit(op);

        if let Some(cb) = &mut self.clipboard {
            cb.set_text(text).ok();
        }
        self.delete_selection();
        self.ensure_visible()?;
        self.set_status("Cut selection.", Duration::from_secs(2));
        Ok(())
    }

    /// Paste from clipboard.
    pub fn paste(&mut self) -> Result<()> {
        if let Some(cb) = &mut self.clipboard {
            if let Ok(text) = cb.get_text() {
                if let Some((a, b)) = self.selection_range() {
                    let deleted_text = self.buf.get_range(a, b);
                    let op = EditOperation::Delete { start: a, _end: b, deleted_text };
                    self.record_edit(op);
                }

                let op = EditOperation::Insert { pos: self.cursor, text: text.clone() };
                self.record_edit(op);

                self.replace_selection_or_insert(&text);
                self.ensure_visible()?;
                self.set_status("Pasted.", Duration::from_secs(2));
                return Ok(());
            }
        }
        self.set_status("Clipboard unavailable.", Duration::from_secs(2));
        Ok(())
    }
}
