use serde::ser::{SerializeMap, SerializeSeq};
use std::collections::HashMap;
use std::fmt;
use std::{cell::RefCell, collections::BTreeMap};

// for bug!
use log::{error, log};

pub enum Data {
    Null,
    String(String),
    Bool(bool),
    Vec(Vec<Data>),
    Map(HashMap<String, Data>),
    Fun(RefCell<Box<dyn FnMut(String) -> String + Send>>),
}

impl serde::Serialize for Data {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {
            Data::Null => serializer.serialize_none(),
            Data::String(ref v) => serializer.serialize_str(v),
            Data::Bool(v) => serializer.serialize_bool(v),
            Data::Vec(ref v) => {
                let mut seq = serializer.serialize_seq(Some(v.len()))?;
                for e in v {
                    seq.serialize_element(e)?;
                }
                seq.end()
            }
            Data::Map(ref v) => {
                let v: BTreeMap<_, _> = v.iter().collect();
                let mut map = serializer.serialize_map(Some(v.len()))?;
                for (k, va) in v {
                    map.serialize_entry(k, va)?;
                }
                map.end()
            }
            Data::Fun(_) => serializer.serialize_unit(),
        }
    }
}

impl PartialEq for Data {
    #[inline]
    fn eq(&self, other: &Data) -> bool {
        match (self, other) {
            (Data::Null, Data::Null) => true,
            (Data::String(v0), Data::String(v1)) => v0 == v1,
            (Data::Bool(v0), Data::Bool(v1)) => v0 == v1,
            (Data::Vec(v0), Data::Vec(v1)) => v0 == v1,
            (Data::Map(v0), Data::Map(v1)) => v0 == v1,
            (Data::Fun(_), &Data::Fun(_)) => {
                bug!("Cannot compare closures");
                false
            }
            (_, _) => false,
        }
    }
}

impl fmt::Debug for Data {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Data::Null => write!(f, "Null"),
            Data::String(ref v) => write!(f, "StrVal({v})"),
            Data::Bool(v) => write!(f, "Bool({v:?})"),
            Data::Vec(ref v) => write!(f, "VecVal({v:?})"),
            Data::Map(ref v) => write!(f, "Map({v:?})"),
            Data::Fun(_) => write!(f, "Fun(...)"),
        }
    }
}
