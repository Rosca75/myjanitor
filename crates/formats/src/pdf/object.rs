//! PDF object model (ISO 32000-1 §7.3).
//!
//! Values are kept in a form that writes back *semantically identical*:
//! names and strings hold their decoded bytes (escapes resolved), reals keep
//! their original token text (no float round trip), dictionaries keep their
//! key order and any duplicate keys exactly as read.

/// An indirect reference `num gen R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjRef {
    pub num: u32,
    pub gen: u16,
}

impl ObjRef {
    /// Reference with generation 0.
    pub fn new(num: u32) -> Self {
        ObjRef { num, gen: 0 }
    }
}

/// One PDF value.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    /// A real (or an integer too large for `i64`), as its original token text.
    Real(Vec<u8>),
    /// A name, decoded (`#20` -> space), without the leading `/`.
    Name(Vec<u8>),
    /// A literal or hex string, decoded.
    Str(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Ref(ObjRef),
    /// Only ever the direct value of an indirect object.
    Stream(Stream),
}

impl Object {
    /// The name bytes, if this is a name.
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    /// The integer, if this is an integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The dictionary of a dictionary or a stream.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    /// Mutable dictionary of a dictionary or a stream.
    pub fn as_dict_mut(&mut self) -> Option<&mut Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&mut s.dict),
            _ => None,
        }
    }

    /// The reference, if this is one.
    pub fn as_ref(&self) -> Option<ObjRef> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }

    /// True for `name` equal to `n`.
    pub fn is_name(&self, n: &[u8]) -> bool {
        self.as_name() == Some(n)
    }
}

/// A dictionary: ordered key/value pairs (keys without the leading `/`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Object)>);

impl Dict {
    /// Empty dictionary.
    pub fn new() -> Self {
        Dict(Vec::new())
    }

    /// Value of the last entry named `key` (readers keep the last duplicate).
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Mutable value of the last entry named `key`.
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Object> {
        self.0.iter_mut().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// True when `key` is present.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    /// Set `key` (replaces every existing entry of that name, keeps the first position).
    pub fn set(&mut self, key: &[u8], value: Object) {
        if let Some(pos) = self.0.iter().position(|(k, _)| k == key) {
            self.0[pos].1 = value;
            let mut i = pos + 1;
            while i < self.0.len() {
                if self.0[i].0 == key {
                    self.0.remove(i);
                } else {
                    i += 1;
                }
            }
        } else {
            self.0.push((key.to_vec(), value));
        }
    }

    /// Remove every entry named `key`; returns the last removed value.
    pub fn remove(&mut self, key: &[u8]) -> Option<Object> {
        let mut out = None;
        let mut i = 0;
        while i < self.0.len() {
            if self.0[i].0 == key {
                out = Some(self.0.remove(i).1);
            } else {
                i += 1;
            }
        }
        out
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The `/Type` name, if any.
    pub fn type_name(&self) -> Option<&[u8]> {
        self.get(b"Type").and_then(Object::as_name)
    }
}

/// A stream: its dictionary and its raw (still encoded) bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
}

/// Call `f` on every reference inside a direct value (iterative, no recursion).
pub fn for_each_ref(obj: &Object, mut f: impl FnMut(ObjRef)) {
    let mut stack: Vec<&Object> = vec![obj];
    while let Some(o) = stack.pop() {
        match o {
            Object::Ref(r) => f(*r),
            Object::Array(a) => stack.extend(a.iter()),
            Object::Dict(d) => stack.extend(d.0.iter().map(|(_, v)| v)),
            Object::Stream(s) => stack.extend(s.dict.0.iter().map(|(_, v)| v)),
            _ => {}
        }
    }
}

/// Rewrite every reference inside a direct value through `f` (iterative).
/// `f` returns `None` to replace a dangling reference by `null`.
pub fn map_refs(obj: &mut Object, f: &mut dyn FnMut(ObjRef) -> Option<ObjRef>) {
    let mut stack: Vec<&mut Object> = vec![obj];
    while let Some(o) = stack.pop() {
        match o {
            Object::Ref(r) => match f(*r) {
                Some(n) => *r = n,
                None => *o = Object::Null,
            },
            Object::Array(a) => stack.extend(a.iter_mut()),
            Object::Dict(d) => stack.extend(d.0.iter_mut().map(|(_, v)| v)),
            Object::Stream(s) => stack.extend(s.dict.0.iter_mut().map(|(_, v)| v)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_set_get_remove() {
        let mut d = Dict::new();
        d.set(b"A", Object::Int(1));
        d.0.push((b"A".to_vec(), Object::Int(2)));
        assert_eq!(d.get(b"A"), Some(&Object::Int(2)));
        d.set(b"A", Object::Int(3));
        assert_eq!(d.len(), 1);
        assert_eq!(d.remove(b"A"), Some(Object::Int(3)));
        assert!(d.is_empty());
    }

    #[test]
    fn refs_are_mapped_and_dangling_become_null() {
        let mut o = Object::Array(vec![Object::Ref(ObjRef::new(1)), Object::Ref(ObjRef::new(2))]);
        map_refs(&mut o, &mut |r| if r.num == 1 { Some(ObjRef::new(9)) } else { None });
        assert_eq!(o, Object::Array(vec![Object::Ref(ObjRef::new(9)), Object::Null]));
        let mut seen = Vec::new();
        for_each_ref(&o, |r| seen.push(r.num));
        assert_eq!(seen, vec![9]);
    }
}
