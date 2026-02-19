//! Arc-backed interned string. Clone is O(1) via atomic refcount bump.
//! Used for CachedEntry.topic to eliminate ~955 redundant String allocations
//! across ~45 unique topics × ~1000 entries.

use std::sync::Arc;

/// Shared-ownership string. Clone costs one atomic increment, no heap alloc.
#[derive(Clone)]
pub struct InternedStr(Arc<str>);

impl InternedStr {
    #[inline]
    pub fn new(s: &str) -> Self { Self(Arc::from(s)) }
    #[inline]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::ops::Deref for InternedStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str { &self.0 }
}

// --- Equality: pointer-fast path then content fallback ---

impl PartialEq for InternedStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}
impl Eq for InternedStr {}

impl PartialEq<str> for InternedStr {
    #[inline]
    fn eq(&self, other: &str) -> bool { &*self.0 == other }
}
impl PartialEq<&str> for InternedStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool { &*self.0 == *other }
}
impl PartialEq<String> for InternedStr {
    #[inline]
    fn eq(&self, other: &String) -> bool { &*self.0 == other.as_str() }
}

// --- Hashing, ordering, borrowing ---

impl std::hash::Hash for InternedStr {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) { (*self.0).hash(state) }
}

impl Ord for InternedStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { (*self.0).cmp(&*other.0) }
}
impl PartialOrd for InternedStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}

impl std::borrow::Borrow<str> for InternedStr {
    #[inline]
    fn borrow(&self) -> &str { &self.0 }
}
impl AsRef<str> for InternedStr {
    #[inline]
    fn as_ref(&self) -> &str { &self.0 }
}

// --- Display / Debug ---

impl std::fmt::Display for InternedStr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { (*self.0).fmt(f) }
}
impl std::fmt::Debug for InternedStr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{:?}", &*self.0) }
}
