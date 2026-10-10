//! Thinker list and ticking, port of `p_tick.c`.
//!
//! The original keeps an intrusive, circular doubly-linked list of `thinker_t`
//! around a global `thinkercap` sentinel and dispatches through function
//! pointers (`thinker->function.acp1`). Because the core forbids `unsafe`, the
//! nodes here live in a slot arena addressed by [`ThinkerId`] and dispatch is
//! delegated to a [`ThinkHandler`]. The owning system keeps the payloads (mobj,
//! specials, …); a slot only carries a [`ThinkerKind`] discriminator.
//!
//! Semantics preserved from vanilla:
//! * `P_AddThinker` appends at the tail (new thinkers added during a run are
//!   visited in the same run, like the C loop re-reading `->next`).
//! * `P_RemoveThinker` is lazy: it only marks the node, which is unlinked when
//!   the iteration reaches it — possibly on a later tic for self-removals.

use alloc::vec::Vec;

/// Handle to a node in [`Thinkers`].
pub type ThinkerId = usize;

/// What a thinker slot represents. The payload lives in the owning system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkerKind {
    /// A map object, identified by its index in the mobj arena.
    Mobj(usize),
    /// A sector door (`p_doors` / `T_VerticalDoor`).
    Door(usize),
    /// A sector ceiling mover (`p_ceilng` / `T_MoveCeiling`).
    Ceiling(usize),
    /// A platform (`p_plats` / `T_PlatRaise`).
    Plat(usize),
    /// A floor mover / elevator (`p_floor` / `T_MoveFloor`).
    Floor(usize),
    /// A light flasher/strobe (`p_spec`).
    Light(usize),
    /// Any other thinker type (misc.), identified by a tag.
    Other(u32),
}

/// Receiver of thinker dispatches during [`Thinkers::run`].
///
/// The handler is a separate object from the list (the standard ownership
/// pattern is `core::mem::take`-ing the [`Thinkers`] out of the world so the
/// world can be borrowed mutably as the handler).
pub trait ThinkHandler {
    fn think(&mut self, id: ThinkerId, kind: ThinkerKind, thinkers: &mut Thinkers);
}

#[derive(Clone, Copy, Default)]
struct Slot {
    prev: Option<ThinkerId>,
    next: Option<ThinkerId>,
    kind: Option<ThinkerKind>,
}

/// The thinker list (`thinkercap` + arena).
#[derive(Default)]
pub struct Thinkers {
    slots: Vec<Slot>,
    head: Option<ThinkerId>,
    tail: Option<ThinkerId>,
    free: Vec<ThinkerId>,
}

impl Thinkers {
    pub fn new() -> Self {
        Self::default()
    }

    /// `P_InitThinkers`: start an empty list.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.head = None;
        self.tail = None;
    }

    /// Savegame restore: adopt a snapshot's (id, kind) pairs wholesale,
    /// rebuilding the arena, the free list and the links in order.
    pub fn restore(&mut self, pairs: &[(ThinkerId, ThinkerKind)]) {
        self.clear();
        let mut max_id = 0usize;
        for &(id, _) in pairs {
            max_id = max_id.max(id + 1);
        }
        self.slots = Vec::new();
        self.free.clear();
        for &(id, kind) in pairs {
            while self.slots.len() <= id {
                self.slots.push(Slot::default());
                self.free.push(self.slots.len() - 1);
            }
            self.slots[id] = Slot {
                prev: None,
                next: None,
                kind: Some(kind),
            };
            self.free.retain(|&f| f != id);
        }
        // Link in ascending id order (the snapshot's iteration order).
        self.head = None;
        self.tail = None;
        let mut prev: Option<ThinkerId> = None;
        for id in 0..self.slots.len() {
            if self.slots[id].kind.is_some() {
                self.slots[id].prev = prev;
                self.slots[id].next = None;
                if let Some(p) = prev {
                    self.slots[p].next = Some(id);
                } else {
                    self.head = Some(id);
                }
                prev = Some(id);
                self.tail = Some(id);
            }
        }
    }

    /// The full (id, kind) snapshot for the savegame.
    pub fn snapshot(&self) -> Vec<(ThinkerId, ThinkerKind)> {
        let mut out = Vec::new();
        let mut cur = self.head;
        while let Some(id) = cur {
            if let Some(kind) = self.kind(id) {
                out.push((id, kind));
            }
            cur = self.next_of(id);
        }
        out
    }

    /// `P_AddThinker`: append at the end of the list, returning its handle.
    pub fn add(&mut self, kind: ThinkerKind) -> ThinkerId {
        let id = self.free.pop().unwrap_or_else(|| {
            self.slots.push(Slot::default());
            self.slots.len() - 1
        });
        self.slots[id] = Slot {
            prev: self.tail,
            next: None,
            kind: Some(kind),
        };
        match self.tail {
            Some(t) => self.slots[t].next = Some(id),
            None => self.head = Some(id),
        }
        self.tail = Some(id);
        id
    }

    /// `P_RemoveThinker`: lazy removal — the node stays linked until visited.
    pub fn remove(&mut self, id: ThinkerId) {
        self.slots[id].kind = None;
    }

    /// The kind of a live thinker, or `None` if removed/freed.
    pub fn kind(&self, id: ThinkerId) -> Option<ThinkerKind> {
        self.slots[id].kind
    }

    /// Whether `id` has been marked for removal.
    pub fn is_removed(&self, id: ThinkerId) -> bool {
        self.slots[id].kind.is_none()
    }

    /// First (oldest) live-or-marked thinker.
    pub fn head(&self) -> Option<ThinkerId> {
        self.head
    }

    /// Next node after `id` (used by callers that walk the list manually).
    pub fn next_of(&self, id: ThinkerId) -> Option<ThinkerId> {
        self.slots[id].next
    }

    /// Iterate the list from head to tail (read-only).
    pub fn iter(&self) -> ThinkerIter<'_> {
        ThinkerIter {
            thinkers: self,
            current: self.head,
        }
    }

    /// Unlink and free a node (used by the manual run loops in the gameplay
    /// layer; [`Thinkers::run`] calls this internally for marked nodes).
    pub fn unlink(&mut self, id: ThinkerId) {
        let slot = self.slots[id];
        match slot.prev {
            Some(p) => self.slots[p].next = slot.next,
            None => self.head = slot.next,
        }
        match slot.next {
            Some(n) => self.slots[n].prev = slot.prev,
            None => self.tail = slot.prev,
        }
        self.slots[id] = Slot::default();
        self.free.push(id);
    }

    /// `P_RunThinkers`: visit every thinker once, running live ones and
    /// unlinking the marked ones. New thinkers appended by a handler are
    /// visited in the same pass.
    pub fn run(&mut self, handler: &mut dyn ThinkHandler) {
        let mut current = self.head;
        while let Some(id) = current {
            let mut next = self.slots[id].next;
            match self.slots[id].kind {
                None => self.unlink(id),
                Some(kind) => {
                    handler.think(id, kind, self);
                    // Re-read after the callback so thinkers appended at the
                    // tail during the run are visited too (C reads ->next here).
                    next = self.slots[id].next;
                }
            }
            current = next;
        }
    }
}

/// Read-only iterator over the thinker list.
pub struct ThinkerIter<'a> {
    thinkers: &'a Thinkers,
    current: Option<ThinkerId>,
}

impl Iterator for ThinkerIter<'_> {
    type Item = ThinkerId;

    fn next(&mut self) -> Option<ThinkerId> {
        let id = self.current?;
        self.current = self.thinkers.slots[id].next;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        seen: Vec<usize>,
        remove_self: Option<ThinkerId>,
    }

    impl ThinkHandler for Recorder {
        fn think(&mut self, id: ThinkerId, kind: ThinkerKind, thinkers: &mut Thinkers) {
            let ThinkerKind::Other(tag) = kind else {
                return;
            };
            self.seen.push(tag as usize);
            if self.remove_self == Some(id) {
                thinkers.remove(id);
            }
        }
    }

    #[test]
    fn visits_in_insertion_order() {
        let mut t = Thinkers::new();
        t.add(ThinkerKind::Other(0));
        t.add(ThinkerKind::Other(1));
        t.add(ThinkerKind::Other(2));

        let mut rec = Recorder::default();
        t.run(&mut rec);
        assert_eq!(rec.seen, [0, 1, 2]);
        assert_eq!(t.iter().count(), 3);
    }

    #[test]
    fn removed_is_skipped_and_freed() {
        let mut t = Thinkers::new();
        t.add(ThinkerKind::Other(0));
        let mid = t.add(ThinkerKind::Other(1));
        t.add(ThinkerKind::Other(2));
        t.remove(mid);

        let mut rec = Recorder::default();
        t.run(&mut rec);
        assert_eq!(rec.seen, [0, 2]);
        assert!(t.is_removed(mid));
        assert_eq!(t.iter().count(), 2);
    }

    #[test]
    fn self_removal_is_freed_on_next_run() {
        let mut t = Thinkers::new();
        let a = t.add(ThinkerKind::Other(0));
        t.add(ThinkerKind::Other(1));

        let mut rec = Recorder {
            remove_self: Some(a),
            ..Recorder::default()
        };
        t.run(&mut rec);
        assert_eq!(rec.seen, [0, 1]);
        // The self-removed node stays linked (marked) after its own run.
        assert!(t.is_removed(a));
        assert_eq!(t.iter().count(), 2);

        let mut rec2 = Recorder::default();
        t.run(&mut rec2);
        assert_eq!(rec2.seen, [1]);
        assert_eq!(t.iter().count(), 1);

        // The freed slot is recycled on the next add.
        let reused = t.add(ThinkerKind::Other(9));
        assert_eq!(reused, a);
    }

    #[test]
    fn append_during_run_is_visited_same_pass() {
        struct Spawner {
            spawned: bool,
            seen: Vec<usize>,
        }
        impl ThinkHandler for Spawner {
            fn think(&mut self, _id: ThinkerId, kind: ThinkerKind, thinkers: &mut Thinkers) {
                let ThinkerKind::Other(tag) = kind else {
                    return;
                };
                self.seen.push(tag as usize);
                if !self.spawned && tag == 0 {
                    self.spawned = true;
                    thinkers.add(ThinkerKind::Other(42));
                }
            }
        }

        let mut t = Thinkers::new();
        t.add(ThinkerKind::Other(0));
        let mut sp = Spawner {
            spawned: false,
            seen: Vec::new(),
        };
        t.run(&mut sp);
        assert_eq!(sp.seen, [0, 42]);
        assert_eq!(t.iter().count(), 2);
    }
}
