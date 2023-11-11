use crate::buffer::{Buffer, BufferKind};
use std::{
    collections::VecDeque,
    io::{self, ErrorKind},
    path::Path,
};

/// A non-empty vec of buffers where the active buffer is accessible and default
/// buffers are inserted where needed to maintain invariants
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffers {
    next_id: usize,
    inner: VecDeque<Buffer>,
}

impl Default for Buffers {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffers {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            inner: vec![Buffer::new_unnamed(0, "")].into(),
        }
    }

    /// Returns the id of a newly created buffer, None if the buffer already existed
    pub fn open_or_focus<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Option<usize>> {
        let path = match path.as_ref().canonicalize() {
            Ok(p) => p,
            Err(e) if e.kind() == ErrorKind::NotFound => path.as_ref().to_path_buf(),
            Err(e) => return Err(e),
        };
        let idx = self.inner.iter().position(|b| match &b.kind {
            BufferKind::File(p) => p == &path,
            _ => false,
        });

        if let Some(idx) = idx {
            self.inner.swap(0, idx);
            return Ok(None);
        }

        // Remove an empty scratch buffer if the user has now opened a file
        if self.inner.len() == 1 && self.inner[0].is_unnamed() && !self.inner[0].dirty {
            self.inner.remove(0);
        }

        let id = self.next_id;
        self.next_id += 1;
        let b = Buffer::new_from_canonical_file_path(id, path)?;
        self.inner.insert(0, b);

        Ok(Some(id))
    }

    /// Used to seed the buffer selection mini-buffer
    pub(crate) fn as_buf_list(&self) -> Vec<String> {
        let focused = self.inner[0].id;
        self.inner
            .iter()
            .map(|b| {
                format!(
                    "{:<4} {} {}",
                    b.id,
                    if b.id == focused { '*' } else { ' ' },
                    b.full_name()
                )
            })
            .collect()
    }

    pub(crate) fn focus_id(&mut self, id: usize) {
        if let Some(idx) = self.inner.iter().position(|b| b.id == id) {
            self.inner.swap(0, idx);
        }
    }

    pub(crate) fn with_id(&self, id: usize) -> Option<&Buffer> {
        self.inner.iter().find(|b| b.id == id)
    }

    pub(crate) fn with_id_mut(&mut self, id: usize) -> Option<&mut Buffer> {
        self.inner.iter_mut().find(|b| b.id == id)
    }

    pub fn dirty_buffers(&self) -> Vec<&str> {
        self.inner
            .iter()
            .filter(|b| b.dirty)
            .map(|b| b.full_name())
            .collect()
    }

    #[inline]
    pub fn active(&self) -> &Buffer {
        &self.inner[0]
    }

    #[inline]
    pub fn active_mut(&mut self) -> &mut Buffer {
        &mut self.inner[0]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty_scratch(&self) -> bool {
        self.inner.len() == 1 && self.inner[0].is_unnamed() && !self.inner[0].dirty
    }

    pub fn next(&mut self) {
        self.inner.rotate_right(1)
    }

    pub fn previous(&mut self) {
        self.inner.rotate_left(1)
    }

    pub fn close_active(&mut self) {
        self.inner.remove(0);
        if self.inner.is_empty() {
            self.inner.push_back(Buffer::new_unnamed(self.next_id, ""));
            self.next_id += 1;
        }
    }
}
