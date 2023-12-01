//! Buffer state for the fuse filesystem
use super::{
    empty_dir_stat, empty_file_stat, Message, Req, Result, BUFFERS_DIR, BUFFERS_QID,
    CURRENT_BUFFER, CURRENT_BUFFER_QID, E_UNKNOWN_FILE, QID_OFFSET,
};
use crate::{editor::InputEvent, ninep::fs::Stat};
use std::{
    collections::BTreeMap,
    sync::mpsc::{Receiver, Sender},
    time::SystemTime,
};

const FILENAME: &str = "filename";
const DOT: &str = "dot";
const ADDR: &str = "addr";
const BODY: &str = "body";
const EVENT: &str = "event";
const XDOT: &str = "xdot";
const XADDR: &str = "xaddr";

pub(super) const BUFFER_FILES: [(u64, &str); QID_OFFSET as usize - 1] = [
    (1, FILENAME),
    (2, DOT),
    (3, ADDR),
    (4, XDOT),
    (5, XADDR),
    (6, BODY),
    (7, EVENT),
];

fn parent_and_fname(qid: u64) -> (u64, &'static str) {
    assert!(qid > CURRENT_BUFFER_QID, "invalid buffer file qid");

    let (cur, off) = (CURRENT_BUFFER_QID, QID_OFFSET);
    let parent = cur + 1 + ((qid - cur - 1) / off) * off;
    let fname = BUFFER_FILES[((qid - cur - 2) % off) as usize].1;

    (parent, fname)
}

/// A message sent by the main editor thread to notify the fs thread that
/// the current buffer list has changed.
pub enum BufId {
    Add(usize),
    Remove(usize),
    Current(usize),
}

#[derive(Debug)]
pub(super) struct BufferNodes {
    pub(super) known: BTreeMap<u64, BufferNode>,
    next_qid: u64,
    current_buffid: usize,
    stat: Stat,
    current_buff_stat: Stat,
    tx: Sender<InputEvent>,
    brx: Receiver<BufId>,
}

impl BufferNodes {
    pub(super) fn new(tx: Sender<InputEvent>, brx: Receiver<BufId>) -> Self {
        Self {
            known: BTreeMap::default(),
            next_qid: CURRENT_BUFFER_QID + 1,
            current_buffid: 1,
            stat: empty_dir_stat(BUFFERS_QID, BUFFERS_DIR),
            current_buff_stat: empty_file_stat(CURRENT_BUFFER_QID, CURRENT_BUFFER),
            tx,
            brx,
        }
    }

    pub(super) fn stat(&self) -> Stat {
        self.stat.clone()
    }

    pub(super) fn top_level_stats(&self) -> Vec<Stat> {
        let mut stats: Vec<Stat> = self.known.values().map(|b| b.stat.clone()).collect();
        stats.push(self.current_buff_stat.clone());
        stats
    }

    pub(super) fn buffer_level_stats(&self, qid: u64) -> Option<Vec<Stat>> {
        self.known.get(&qid).map(|b| b.file_stats())
    }

    pub(super) fn is_known_buffer_qid(&self, qid: u64) -> bool {
        self.known.contains_key(&qid)
    }

    pub(super) fn lookup_file_stat(&mut self, parent: u64, name: &str) -> Option<Stat> {
        match (parent, name) {
            (BUFFERS_QID, CURRENT_BUFFER) => Some(self.current_buff_stat.clone()),
            (BUFFERS_QID, _) => self
                .known
                .values()
                .find(|b| b.str_id == name)
                .map(|b| b.stat()),
            _ => self
                .known
                .get_mut(&parent)?
                .refreshed_file_stat(name, &self.tx)
                .clone(),
        }
    }

    pub(super) fn get_stat_for_qid(&mut self, qid: u64) -> Option<Stat> {
        // If this is a known directory then we return the stat for it
        if qid == BUFFERS_QID {
            return Some(self.stat());
        } else if qid == CURRENT_BUFFER_QID {
            return Some(self.current_buff_stat.clone());
        } else if let Some(b) = self.known.get(&qid) {
            return Some(b.stat());
        }

        // Otherwise we see if this is known buffer file
        let (parent, fname) = parent_and_fname(qid);
        self.known
            .get_mut(&parent)?
            .refreshed_file_stat(fname, &self.tx)
    }

    pub(super) fn get_file_content(&mut self, qid: u64) -> Option<String> {
        if qid == CURRENT_BUFFER_QID {
            return Some(self.current_buffid.to_string());
        }

        let (parent, fname) = parent_and_fname(qid);
        self.known
            .get(&parent)?
            .current_file_content(fname, &self.tx)
    }

    pub(super) fn truncate(&mut self, qid: u64) {
        if qid == CURRENT_BUFFER_QID {
            return;
        }

        let (parent, fname) = parent_and_fname(qid);
        let b = match self.known.get_mut(&parent) {
            Some(b) => b,
            None => return,
        };
        b.stat.last_modified = SystemTime::now();
        b.stat.n_bytes = 0;
        let id = b.id;

        if fname == BODY {
            _ = Message::send(Req::ClearBufferBody { id }, &self.tx);
        }
    }

    pub(super) fn write(&mut self, qid: u64, s: String, offset: usize) -> Result<usize> {
        let (parent, fname) = parent_and_fname(qid);
        let b = match self.known.get_mut(&parent) {
            Some(b) => b,
            None => return Err(E_UNKNOWN_FILE.to_string()),
        };
        b.stat.last_modified = SystemTime::now();
        let id = b.id;

        let n_bytes = s.len();
        let req = match fname {
            DOT => Req::SetBufferDot { id, s },
            ADDR => Req::SetBufferAddr { id, s },
            BODY => Req::InsertBufferBody { id, s, offset },
            XDOT => Req::SetBufferXDot { id, s },
            XADDR => Req::SetBufferXAddr { id, s },
            FILENAME | EVENT => return Err(E_UNKNOWN_FILE.to_string()),
            _ => return Err(E_UNKNOWN_FILE.to_string()),
        };

        match Message::send(req, &self.tx) {
            Ok(_) => Ok(n_bytes),
            Err(e) => Err(format!("unable to execute control message: {e}")),
        }
    }

    /// Process any pending updates from the main thread for changes to the buffer set
    pub(super) fn update(&mut self) {
        for bid in self.brx.try_iter() {
            match bid {
                BufId::Add(id) => {
                    let qid = self.next_qid;
                    self.next_qid += QID_OFFSET;
                    self.known.insert(qid, BufferNode::new(id, qid));
                }

                BufId::Remove(id) => self.known.retain(|_, v| v.id != id),
                BufId::Current(id) => {
                    self.current_buffid = id;
                    self.current_buff_stat.n_bytes = id.to_string().len() as u64;
                }
            };
        }
    }
}

/// A BufferNode in the filesystem is a directory containing a fixed
/// set of control files
///
/// The qids generated for each of the control files are based on offsets
/// from the qid of the buffer directory itself (see QID_OFFSET above).
#[derive(Debug)]
pub(super) struct BufferNode {
    id: usize,
    str_id: String,
    stat: Stat,
    file_stats: BTreeMap<&'static str, Stat>,
}

impl BufferNode {
    fn new(id: usize, qid: u64) -> Self {
        Self {
            id,
            str_id: id.to_string(),
            stat: empty_dir_stat(qid, &id.to_string()),
            file_stats: stub_file_stats(qid),
        }
    }

    fn stat(&self) -> Stat {
        self.stat.clone()
    }

    fn file_stats(&self) -> Vec<Stat> {
        self.file_stats.values().cloned().collect()
    }

    fn refreshed_file_stat(&mut self, fname: &str, tx: &Sender<InputEvent>) -> Option<Stat> {
        let content = self.current_file_content(fname, tx)?;
        let stat = self.file_stats.get_mut(fname)?;
        stat.n_bytes = content.as_bytes().len() as u64;

        Some(stat.clone())
    }

    fn current_file_content(&self, fname: &str, tx: &Sender<InputEvent>) -> Option<String> {
        let req = match fname {
            FILENAME => Req::ReadBufferName { id: self.id },
            DOT => Req::ReadBufferDot { id: self.id },
            ADDR => Req::ReadBufferAddr { id: self.id },
            BODY => Req::ReadBufferBody { id: self.id },
            XDOT => Req::ReadBufferXDot { id: self.id },
            XADDR => Req::ReadBufferXAddr { id: self.id },
            EVENT => return Some("".to_string()), // TODO: sort out the event file
            _ => return None,
        };

        Message::send(req, tx).ok()
    }
}

fn stub_file_stats(qid: u64) -> BTreeMap<&'static str, Stat> {
    let mut m = BTreeMap::new();

    for (offset, name) in BUFFER_FILES.into_iter() {
        m.insert(name, empty_file_stat(qid + offset, name));
    }

    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    #[test_case(CURRENT_BUFFER_QID + 1 + 1, CURRENT_BUFFER_QID + 1, FILENAME; "filename first buffer")]
    #[test_case(6, 4, DOT; "dot second buffer")]
    #[test_case(14, 10, BODY; "body second buffer")]
    #[test]
    fn parent_and_fname_works(qid: u64, parent: u64, fname: &str) {
        let (p, f) = parent_and_fname(qid);

        assert_eq!(p, parent);
        assert_eq!(f, fname);
    }
}
