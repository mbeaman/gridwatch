//! Fixed-capacity ring over `VecDeque` — no `unsafe` anywhere in the workspace (§4.2).

use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> Ring<T> {
    pub fn new(cap: usize) -> Ring<T> {
        Ring {
            buf: VecDeque::with_capacity(cap.min(4096)),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, v: T) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(v);
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn back(&self) -> Option<&T> {
        self.buf.back()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.buf.iter()
    }

    /// Drop leading elements while `drop` returns true (age-based retention).
    pub fn prune_front(&mut self, mut drop: impl FnMut(&T) -> bool) {
        while let Some(front) = self.buf.front() {
            if drop(front) {
                self.buf.pop_front();
            } else {
                break;
            }
        }
    }
}
