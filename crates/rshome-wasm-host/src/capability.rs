//! Handle + Rights + ObjectRegistry — capability model for WASM integrations.
//!
//! Handle layout (u64):
//!   bits [63..32] = object_id: u32
//!   bits [31..16] = generation: u16  (bumped on revoke — invalidates old refs)
//!   bits [15..0]  = rights: u16      (READ=0x01, WRITE=0x02, INVOKE=0x04, ADMIN=0x08)

use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

// ── Handle ────────────────────────────────────────────────────────────────────

/// Compact capability handle passed across the host/guest boundary.
pub type Handle = u64;

pub const INVALID_HANDLE: Handle = 0;

/// Encode a handle from its component parts.
#[inline]
pub fn encode_handle(object_id: u32, generation: u16, rights: Rights) -> Handle {
    ((object_id as u64) << 32) | ((generation as u64) << 16) | (rights.bits() as u64)
}

/// Decode a handle into its component parts.
#[inline]
pub fn decode_handle(handle: Handle) -> (u32, u16, Rights) {
    let object_id = (handle >> 32) as u32;
    let generation = ((handle >> 16) & 0xFFFF) as u16;
    let rights = Rights::from_bits_truncate((handle & 0xFFFF) as u16);
    (object_id, generation, rights)
}

// ── Rights ────────────────────────────────────────────────────────────────────

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Rights: u16 {
        const READ   = 0x0001;
        const WRITE  = 0x0002;
        const INVOKE = 0x0004;
        const ADMIN  = 0x0008;
        const ALL    = 0x000F;
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CapError {
    #[error("object not found")]
    NotFound,
    #[error("handle generation mismatch — object was revoked or replaced")]
    Revoked,
    #[error("insufficient rights: required {need:?}, held {have:?}")]
    InsufficientRights { need: Rights, have: Rights },
}

// ── ObjectEntry ───────────────────────────────────────────────────────────────

pub struct ObjectEntry<T> {
    pub value: T,
    generation: AtomicU16,
}

impl<T: std::fmt::Debug> std::fmt::Debug for ObjectEntry<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectEntry")
            .field("value", &self.value)
            .field(
                "generation",
                &self.generation.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}

impl<T> ObjectEntry<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            generation: AtomicU16::new(1),
        }
    }

    pub fn generation(&self) -> u16 {
        self.generation.load(Ordering::Acquire)
    }
}

// ── ObjectRegistry ────────────────────────────────────────────────────────────

/// Thread-safe registry that issues capability handles for objects of type `T`.
pub struct ObjectRegistry<T: Send + Sync + 'static> {
    inner: DashMap<u32, Arc<ObjectEntry<T>>>,
    next_id: AtomicU32,
}

impl<T: Send + Sync + 'static> Default for ObjectRegistry<T> {
    fn default() -> Self {
        Self {
            inner: DashMap::new(),
            next_id: AtomicU32::new(1),
        }
    }
}

impl<T: Send + Sync + 'static> ObjectRegistry<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new entry, returning a capability handle.
    pub fn alloc(&self, value: T, rights: Rights) -> Handle {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let entry = Arc::new(ObjectEntry::new(value));
        self.inner.insert(id, entry);
        encode_handle(id, 1, rights)
    }

    /// Retrieve an entry, checking generation and required rights.
    pub fn get(&self, handle: Handle, required: Rights) -> Result<Arc<ObjectEntry<T>>, CapError> {
        let (id, gen, have) = decode_handle(handle);
        let entry = self.inner.get(&id).ok_or(CapError::NotFound)?;
        if entry.generation.load(Ordering::Acquire) != gen {
            return Err(CapError::Revoked);
        }
        if !have.contains(required) {
            return Err(CapError::InsufficientRights {
                need: required,
                have,
            });
        }
        Ok(entry.clone())
    }

    /// Retrieve an entry without checking rights (useful for internal host code).
    pub fn get_any(&self, handle: Handle) -> Result<Arc<ObjectEntry<T>>, CapError> {
        let (id, gen, _) = decode_handle(handle);
        let entry = self.inner.get(&id).ok_or(CapError::NotFound)?;
        if entry.generation.load(Ordering::Acquire) != gen {
            return Err(CapError::Revoked);
        }
        Ok(entry.clone())
    }

    /// Revoke a handle by bumping the generation.  Existing handles become invalid.
    pub fn revoke(&self, handle: Handle) -> Result<(), CapError> {
        let (id, gen, _) = decode_handle(handle);
        let entry = self.inner.get(&id).ok_or(CapError::NotFound)?;
        let current = entry.generation.load(Ordering::Acquire);
        if current != gen {
            return Err(CapError::Revoked);
        }
        entry.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// Revoke and remove an entry.
    pub fn remove(&self, handle: Handle) -> Result<(), CapError> {
        let (id, gen, _) = decode_handle(handle);
        let pair = self.inner.remove(&id).ok_or(CapError::NotFound)?;
        let current = pair.1.generation.load(Ordering::Acquire);
        if current != gen {
            // Re-insert is not practical; just report revoked
            return Err(CapError::Revoked);
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Handle encoding / decoding ────────────────────────────────────────────

    #[test]
    fn encode_decode_roundtrip() {
        let h = encode_handle(0xDEAD_BEEF, 42, Rights::READ | Rights::WRITE);
        let (id, gen, rights) = decode_handle(h);
        assert_eq!(id, 0xDEAD_BEEF);
        assert_eq!(gen, 42);
        assert_eq!(rights, Rights::READ | Rights::WRITE);
    }

    #[test]
    fn encode_decode_zero_generation() {
        let h = encode_handle(1, 0, Rights::READ);
        let (id, gen, rights) = decode_handle(h);
        assert_eq!(id, 1);
        assert_eq!(gen, 0);
        assert_eq!(rights, Rights::READ);
    }

    #[test]
    fn encode_decode_max_values() {
        let h = encode_handle(u32::MAX, u16::MAX, Rights::ALL);
        let (id, gen, rights) = decode_handle(h);
        assert_eq!(id, u32::MAX);
        assert_eq!(gen, u16::MAX);
        assert_eq!(rights, Rights::ALL);
    }

    #[test]
    fn rights_subset_check() {
        let admin = Rights::ALL;
        assert!(admin.contains(Rights::READ));
        assert!(admin.contains(Rights::WRITE));
        assert!(admin.contains(Rights::INVOKE));
        assert!(admin.contains(Rights::ADMIN));

        let read_only = Rights::READ;
        assert!(!read_only.contains(Rights::WRITE));
    }

    // ── ObjectRegistry ────────────────────────────────────────────────────────

    #[test]
    fn alloc_returns_valid_handle() {
        let reg: ObjectRegistry<String> = ObjectRegistry::new();
        let h = reg.alloc("hello".into(), Rights::READ);
        assert_ne!(h, INVALID_HANDLE);
        let (_, gen, _) = decode_handle(h);
        assert_eq!(gen, 1);
    }

    #[test]
    fn alloc_two_objects_different_ids() {
        let reg: ObjectRegistry<i32> = ObjectRegistry::new();
        let h1 = reg.alloc(1, Rights::READ);
        let h2 = reg.alloc(2, Rights::READ);
        let (id1, _, _) = decode_handle(h1);
        let (id2, _, _) = decode_handle(h2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn get_correct_rights_succeeds() {
        let reg: ObjectRegistry<&str> = ObjectRegistry::new();
        let h = reg.alloc("value", Rights::READ | Rights::WRITE);
        assert!(reg.get(h, Rights::READ).is_ok());
        assert!(reg.get(h, Rights::WRITE).is_ok());
    }

    #[test]
    fn get_insufficient_rights_errors() {
        let reg: ObjectRegistry<u32> = ObjectRegistry::new();
        let h = reg.alloc(42, Rights::READ);
        let err = reg.get(h, Rights::WRITE).unwrap_err();
        assert!(matches!(err, CapError::InsufficientRights { .. }));
    }

    #[test]
    fn get_missing_object_errors() {
        let reg: ObjectRegistry<u32> = ObjectRegistry::new();
        let fake = encode_handle(999, 1, Rights::READ);
        assert_eq!(reg.get(fake, Rights::READ).unwrap_err(), CapError::NotFound);
    }

    #[test]
    fn revoke_invalidates_handle() {
        let reg: ObjectRegistry<u32> = ObjectRegistry::new();
        let h = reg.alloc(7, Rights::READ);
        reg.revoke(h).unwrap();
        assert_eq!(reg.get(h, Rights::READ).unwrap_err(), CapError::Revoked);
    }

    #[test]
    fn revoke_already_revoked_handle_errors() {
        let reg: ObjectRegistry<u32> = ObjectRegistry::new();
        let h = reg.alloc(7, Rights::READ);
        reg.revoke(h).unwrap();
        assert_eq!(reg.revoke(h).unwrap_err(), CapError::Revoked);
    }

    #[test]
    fn revoke_then_alloc_new_handle_still_works() {
        let reg: ObjectRegistry<String> = ObjectRegistry::new();
        let h1 = reg.alloc("first".into(), Rights::READ);
        reg.revoke(h1).unwrap();
        let h2 = reg.alloc("second".into(), Rights::READ);
        // h2 should be accessible
        assert!(reg.get(h2, Rights::READ).is_ok());
        // h1 should still be revoked
        assert_eq!(reg.get(h1, Rights::READ).unwrap_err(), CapError::Revoked);
    }

    #[test]
    fn get_any_skips_rights_check() {
        let reg: ObjectRegistry<i32> = ObjectRegistry::new();
        // Allocate with only READ rights
        let h = reg.alloc(100, Rights::READ);
        // get_any should still succeed (bypasses rights)
        assert!(reg.get_any(h).is_ok());
    }

    #[test]
    fn multiple_independent_registries() {
        let reg_a: ObjectRegistry<u32> = ObjectRegistry::new();
        let reg_b: ObjectRegistry<u32> = ObjectRegistry::new();
        let h_a = reg_a.alloc(10, Rights::READ);
        // reg_b gets a second entry — id=2 won't exist in reg_a
        let _ = reg_b.alloc(20, Rights::READ);
        let h_b2 = reg_b.alloc(30, Rights::READ);
        let (id_b2, _, _) = decode_handle(h_b2);
        assert_eq!(id_b2, 2); // reg_b has 2 entries; reg_a has 1

        // reg_a has no entry with id=2 → NotFound
        assert_eq!(
            reg_a.get(h_b2, Rights::READ).unwrap_err(),
            CapError::NotFound
        );

        // Revoke h_a only affects reg_a
        reg_a.revoke(h_a).unwrap();
        assert_eq!(reg_a.get(h_a, Rights::READ).unwrap_err(), CapError::Revoked);
        // reg_b's handles are unaffected
        assert!(reg_b.get(h_b2, Rights::READ).is_ok());
    }

    #[test]
    fn len_tracks_allocations() {
        let reg: ObjectRegistry<i32> = ObjectRegistry::new();
        assert_eq!(reg.len(), 0);
        let h1 = reg.alloc(1, Rights::READ);
        assert_eq!(reg.len(), 1);
        let _h2 = reg.alloc(2, Rights::READ);
        assert_eq!(reg.len(), 2);
        // revoke doesn't remove from map (just bumps generation)
        reg.revoke(h1).unwrap();
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn remove_decrements_len() {
        let reg: ObjectRegistry<i32> = ObjectRegistry::new();
        let h = reg.alloc(99, Rights::READ);
        assert_eq!(reg.len(), 1);
        reg.remove(h).unwrap();
        assert_eq!(reg.len(), 0);
    }
}
