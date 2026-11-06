//! The format-neutral data tree.
//!
//! One `Value` enum serves both JSON and XML data (see DESIGN.md, "Core
//! model"). The tree is made of reference-counted nodes with copy-on-write:
//! cloning a [`Node`] is a pointer copy, so the evaluation thread can take a
//! consistent snapshot by cloning the root, while the editing thread, when it
//! changes a node, copies only the nodes on the path from the root to it.

use gossia_i18n::tr;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
use indexmap::IndexMap;

use crate::path::{Path, Segment};

/// A stable identity for a node, used for selection, expansion state, and undo.
/// Formulas never use it (see DESIGN.md, "References are literal"). Ids are not
/// saved: they are handed out afresh when a tree is created or loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u64);

impl NodeId {
    fn fresh() -> NodeId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NodeId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// A date and time, which keeps its offset if it had one and stays naive if not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DateTimeValue {
    Naive(NaiveDateTime),
    Zoned(DateTime<FixedOffset>),
}

/// The kind of failure an error value records (see DESIGN.md, "Error values").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ErrorKind {
    Type,
    Ref,
    Name,
    Value,
    DivZero,
    Circular,
    Syntax,
    PyErr,
    Timeout,
    Source,
    Untrusted,
    Na,
}

impl ErrorKind {
    /// The code shown in a cell, for example `#DIV/0!`.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Type => "#TYPE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::DivZero => "#DIV/0!",
            ErrorKind::Circular => "#CIRCULAR!",
            ErrorKind::Syntax => "#SYNTAX!",
            ErrorKind::PyErr => "#PYERR!",
            ErrorKind::Timeout => "#TIMEOUT!",
            ErrorKind::Source => "#SOURCE!",
            ErrorKind::Untrusted => "#UNTRUSTED!",
            ErrorKind::Na => "#N/A",
        }
    }

    /// Every kind, in a fixed order.
    pub const ALL: [ErrorKind; 12] = [
        ErrorKind::Type,
        ErrorKind::Ref,
        ErrorKind::Name,
        ErrorKind::Value,
        ErrorKind::DivZero,
        ErrorKind::Circular,
        ErrorKind::Syntax,
        ErrorKind::PyErr,
        ErrorKind::Timeout,
        ErrorKind::Source,
        ErrorKind::Untrusted,
        ErrorKind::Na,
    ];

    /// The kind whose [`code`](ErrorKind::code) this is.
    pub fn from_code(code: &str) -> Option<ErrorKind> {
        ErrorKind::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// What a failed formula leaves in its node. It exists only in memory and is
/// never written to a data file. The Python exception and traceback that caused
/// it are kept by the calculation layer, which owns the Python side.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorValue {
    pub kind: ErrorKind,
    pub message: String,
    /// The node where the error originated, which may be an upstream node.
    pub origin: Option<Path>,
}

/// The content of a node.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date(NaiveDate),
    DateTime(DateTimeValue),
    Array(Vec<Node>),
    Object(IndexMap<String, Node>),
    Error(ErrorValue),
}

impl Value {
    /// The name of this kind of value, as used in messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Date(_) => "date",
            Value::DateTime(_) => "dateTime",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Error(_) => "error",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Node]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&IndexMap<String, Node>> {
        match self {
            Value::Object(fields) => Some(fields),
            _ => None,
        }
    }
}

/// A node of the data tree. Cloning is cheap and shares the content.
///
/// Equality compares content and ignores identity, so two nodes with the same
/// value are equal. [`Node::ptr_eq`] tells whether two nodes are the same shared
/// content.
#[derive(Debug, Clone)]
pub struct Node(Arc<NodeData>);

#[derive(Debug, Clone)]
struct NodeData {
    id: NodeId,
    value: Value,
    /// A comment on the node (XML tabs only; DESIGN.md, "Comments"). It belongs
    /// to the node, so it goes where the node goes. Formulas cannot see it.
    comment: Option<String>,
    /// The comments inside the node's element, at its start and at its end
    /// (rare, so kept apart to keep nodes small).
    inner: Option<Box<[Option<String>; 2]>>,
    /// An empty line before the node, above its comment, in a YAML or TOML
    /// file (DESIGN.md, "Other file formats"). Like a comment, it goes where
    /// the node goes.
    blank: bool,
}

/// A node as the Web Worker of the wasm version receives it (DESIGN.md, Web
/// port, 0.3.0): everything but the id. A node that is read back gets a fresh
/// id, since ids are never saved and each process numbers its own.
#[derive(serde::Serialize, serde::Deserialize)]
struct NodeWire {
    value: Value,
    comment: Option<String>,
    inner: Option<Box<[Option<String>; 2]>>,
    blank: bool,
}

impl serde::Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("NodeWire", 4)?;
        s.serialize_field("value", &self.0.value)?;
        s.serialize_field("comment", &self.0.comment)?;
        s.serialize_field("inner", &self.0.inner)?;
        s.serialize_field("blank", &self.0.blank)?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Node, D::Error> {
        let wire = NodeWire::deserialize(deserializer)?;
        Ok(Node(Arc::new(NodeData {
            id: NodeId::fresh(),
            value: wire.value,
            comment: wire.comment,
            inner: wire.inner,
            blank: wire.blank,
        })))
    }
}

/// Where a comment on a node is written in XML (DESIGN.md, "Comments").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CommentPlace {
    /// Before the node's element: the node's comment.
    Before,
    /// Inside the element, before its text.
    Start,
    /// Inside the element, after the rest of it.
    End,
}

impl CommentPlace {
    pub const ALL: [CommentPlace; 3] =
        [CommentPlace::Before, CommentPlace::Start, CommentPlace::End];
}

/// Why a path could not be followed by an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// There is no node at this path.
    Missing(Path),
    /// The path steps into a node that cannot be stepped into that way, for
    /// example an index into an object.
    WrongKind(Path),
    /// An array index is past the end (an insert may be exactly at the end).
    OutOfRange(Path),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Missing(p) => f.write_str(&tr!("no node at `{p}`", p = p)),
            PathError::WrongKind(p) => f.write_str(&tr!("cannot step into `{p}` this way", p = p)),
            PathError::OutOfRange(p) => f.write_str(&tr!("index out of range at `{p}`", p = p)),
        }
    }
}

impl std::error::Error for PathError {}

impl Node {
    /// A new node with a fresh id.
    pub fn new(value: Value) -> Node {
        Node(Arc::new(NodeData {
            id: NodeId::fresh(),
            value,
            comment: None,
            inner: None,
            blank: false,
        }))
    }

    /// Whether an empty line comes before the node in a YAML or TOML file.
    pub fn blank_before(&self) -> bool {
        self.0.blank
    }

    pub fn set_blank_before(&mut self, blank: bool) {
        if self.0.blank != blank {
            Arc::make_mut(&mut self.0).blank = blank;
        }
    }

    /// The same node with a comment (or with none), keeping its id.
    pub fn with_comment(mut self, comment: Option<String>) -> Node {
        Arc::make_mut(&mut self.0).comment = comment;
        self
    }

    pub fn comment(&self) -> Option<&str> {
        self.0.comment.as_deref()
    }

    pub fn set_comment(&mut self, comment: Option<String>) {
        Arc::make_mut(&mut self.0).comment = comment;
    }

    /// The comment at `place`, if there is one.
    pub fn comment_at(&self, place: CommentPlace) -> Option<&str> {
        match place {
            CommentPlace::Before => self.comment(),
            CommentPlace::Start => self.0.inner.as_ref()?[0].as_deref(),
            CommentPlace::End => self.0.inner.as_ref()?[1].as_deref(),
        }
    }

    /// Puts (or with `None` takes away) the comment at `place`.
    pub fn set_comment_at(&mut self, place: CommentPlace, comment: Option<String>) {
        let i = match place {
            CommentPlace::Before => return self.set_comment(comment),
            CommentPlace::Start => 0,
            CommentPlace::End => 1,
        };
        let data = Arc::make_mut(&mut self.0);
        match (&mut data.inner, comment) {
            (Some(inner), comment) => {
                inner[i] = comment;
                if inner.iter().all(Option::is_none) {
                    data.inner = None;
                }
            }
            (None, None) => {}
            (None, Some(comment)) => {
                let mut inner = [None, None];
                inner[i] = Some(comment);
                data.inner = Some(Box::new(inner));
            }
        }
    }

    /// Whether the node has a comment at any place.
    pub fn has_comments(&self) -> bool {
        self.0.comment.is_some() || self.0.inner.is_some()
    }

    /// The same node with the comments of `other`, at every place, and its
    /// empty line before.
    pub fn with_comments_of(mut self, other: &Node) -> Node {
        let data = Arc::make_mut(&mut self.0);
        data.comment = other.0.comment.clone();
        data.inner = other.0.inner.clone();
        data.blank = other.0.blank;
        self
    }

    pub fn id(&self) -> NodeId {
        self.0.id
    }

    pub fn value(&self) -> &Value {
        &self.0.value
    }

    /// Mutable access to the content. If the node is shared with a snapshot,
    /// this copies this one node first (not its children), and keeps its id.
    pub fn value_mut(&mut self) -> &mut Value {
        &mut Arc::make_mut(&mut self.0).value
    }

    /// Whether both are the same shared content (a snapshot has not diverged).
    pub fn ptr_eq(&self, other: &Node) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// The direct child at `segment`, if there is one.
    pub fn child(&self, segment: &Segment) -> Option<&Node> {
        match (&self.0.value, segment) {
            (Value::Object(fields), Segment::Key(k)) => fields.get(k),
            (Value::Array(items), Segment::Index(i)) => items.get(*i),
            _ => None,
        }
    }

    /// The node at `path` below this one.
    pub fn get(&self, path: &Path) -> Option<&Node> {
        path.segments()
            .iter()
            .try_fold(self, |node, segment| node.child(segment))
    }

    /// Sets the value at `path`. An existing node keeps its id. A missing
    /// object field is added at the end, and an array index equal to the length
    /// appends. Anything else that does not exist is an error: nothing is
    /// created implicitly along the way.
    ///
    /// Only the nodes on the path are copied; the rest stay shared with any
    /// snapshot. A failed edit copies nothing.
    pub fn set(&mut self, path: &Path, value: Value) -> Result<(), PathError> {
        let segments = path.segments();
        let prefix = |n: usize| Path::from(segments[..n].to_vec());
        // Check the whole path before taking mutable access to anything.
        let mut node: &Node = self;
        for (depth, segment) in segments.iter().enumerate() {
            let last = depth + 1 == segments.len();
            match (node.value(), segment) {
                (Value::Object(fields), Segment::Key(k)) => match fields.get(k) {
                    Some(child) => node = child,
                    None if last => break,
                    None => return Err(PathError::Missing(prefix(depth + 1))),
                },
                (Value::Array(items), Segment::Index(i)) => match items.get(*i) {
                    Some(child) => node = child,
                    None if last && *i == items.len() => break,
                    None => return Err(PathError::OutOfRange(prefix(depth + 1))),
                },
                _ => return Err(PathError::WrongKind(prefix(depth))),
            }
        }
        let Some((last, parent_segments)) = segments.split_last() else {
            *self.value_mut() = value;
            return Ok(());
        };
        let parent = self
            .get_mut(&Path::from(parent_segments.to_vec()))
            .expect("checked above");
        match (parent.value_mut(), last) {
            (Value::Object(fields), Segment::Key(k)) => match fields.get_mut(k) {
                Some(existing) => *existing.value_mut() = value,
                None => {
                    fields.insert(k.clone(), Node::new(value));
                }
            },
            (Value::Array(items), Segment::Index(i)) => match items.get_mut(*i) {
                Some(existing) => *existing.value_mut() = value,
                None => items.push(Node::new(value)),
            },
            _ => unreachable!("checked above"),
        }
        Ok(())
    }

    /// Removes and returns the node at `path`. Removing an array item shifts
    /// the later items down. The root cannot be removed.
    pub fn remove(&mut self, path: &Path) -> Result<Node, PathError> {
        let Some((last, parent_segments)) = path.segments().split_last() else {
            return Err(PathError::WrongKind(Path::root()));
        };
        let parent_path = Path::from(parent_segments.to_vec());
        match self.get(&parent_path) {
            None => return Err(PathError::Missing(parent_path)),
            Some(parent) => match (parent.value(), last) {
                (Value::Object(fields), Segment::Key(k)) if fields.contains_key(k) => {}
                (Value::Object(_), Segment::Key(_)) => {
                    return Err(PathError::Missing(path.clone()));
                }
                (Value::Array(items), Segment::Index(i)) if *i < items.len() => {}
                (Value::Array(_), Segment::Index(_)) => {
                    return Err(PathError::OutOfRange(path.clone()));
                }
                _ => return Err(PathError::WrongKind(parent_path)),
            },
        }
        let parent = self.get_mut(&parent_path).expect("checked above");
        Ok(match (parent.value_mut(), last) {
            (Value::Object(fields), Segment::Key(k)) => {
                fields.shift_remove(k).expect("checked above")
            }
            (Value::Array(items), Segment::Index(i)) => items.remove(*i),
            _ => unreachable!("checked above"),
        })
    }

    /// Where the node at `path` sits among its parent's children: its index in
    /// an array, or its place in the order of an object's fields. `None` for
    /// the root and for a path that leads nowhere.
    pub fn position_of(&self, path: &Path) -> Option<usize> {
        let (last, parent_segments) = path.segments().split_last()?;
        let parent = self.get(&Path::from(parent_segments.to_vec()))?;
        match (parent.value(), last) {
            (Value::Array(items), Segment::Index(i)) if *i < items.len() => Some(*i),
            (Value::Object(fields), Segment::Key(k)) => fields.get_index_of(k),
            _ => None,
        }
    }

    /// Puts `node` in place of the existing node at `path`, keeping the new
    /// node's own id, and returns the node it replaced. This is how an edit is
    /// undone without losing the identity of what was there.
    pub fn replace(&mut self, path: &Path, node: Node) -> Result<Node, PathError> {
        let Some((last, parent_segments)) = path.segments().split_last() else {
            return Ok(std::mem::replace(self, node));
        };
        let parent_path = Path::from(parent_segments.to_vec());
        match self.get(path) {
            Some(_) => {}
            None => return Err(PathError::Missing(path.clone())),
        }
        let parent = self.get_mut(&parent_path).expect("the node has a parent");
        Ok(match (parent.value_mut(), last) {
            (Value::Object(fields), Segment::Key(k)) => {
                std::mem::replace(fields.get_mut(k).expect("checked above"), node)
            }
            (Value::Array(items), Segment::Index(i)) => std::mem::replace(&mut items[*i], node),
            _ => unreachable!("checked above"),
        })
    }

    /// Inserts `node` as the child of the node at `parent` that `segment` names,
    /// at `position` among the children: an array item is inserted there and
    /// later items move up; an object field is placed there in the order of the
    /// fields. The node keeps its own id.
    pub fn insert_child(
        &mut self,
        parent: &Path,
        position: usize,
        segment: &Segment,
        node: Node,
    ) -> Result<(), PathError> {
        let Some(target) = self.get_mut(parent) else {
            return Err(PathError::Missing(parent.clone()));
        };
        match (target.value_mut(), segment) {
            (Value::Array(items), Segment::Index(_)) => {
                if position > items.len() {
                    return Err(PathError::OutOfRange(parent.clone()));
                }
                items.insert(position, node);
                Ok(())
            }
            (Value::Object(fields), Segment::Key(k)) => {
                if position > fields.len() {
                    return Err(PathError::OutOfRange(parent.clone()));
                }
                fields.shift_insert(position, k.clone(), node);
                Ok(())
            }
            _ => Err(PathError::WrongKind(parent.clone())),
        }
    }

    /// Mutable access to the node at `path`, copying only the nodes on the way.
    pub fn get_mut(&mut self, path: &Path) -> Option<&mut Node> {
        let mut node = self;
        for segment in path.segments() {
            node = match (node.value_mut(), segment) {
                (Value::Object(fields), Segment::Key(k)) => fields.get_mut(k)?,
                (Value::Array(items), Segment::Index(i)) => items.get_mut(*i)?,
                _ => return None,
            };
        }
        Some(node)
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Node) -> bool {
        self.ptr_eq(other) || self.0.value == other.0.value
    }
}

impl From<Value> for Node {
    fn from(value: Value) -> Node {
        Node::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are never saved: a node read back from the worker's encoding
    /// (DESIGN.md, Web port, 0.3.0) gets a new one, and keeps the rest.
    #[test]
    fn an_encoded_node_comes_back_with_a_fresh_id() {
        let node = Node::new(Value::Array(vec![Node::new(Value::Int(1))]))
            .with_comment(Some("note".to_owned()));
        let back: Node = serde_json::from_str(&serde_json::to_string(&node).unwrap()).unwrap();
        assert_ne!(back.id(), node.id());
        assert_eq!(back.comment(), Some("note"));
        let Value::Array(items) = back.value() else {
            panic!("not an array");
        };
        assert_eq!(items[0].value(), &Value::Int(1));
    }

    #[test]
    fn position_replace_and_insert_child_restore_what_remove_took_out() {
        let mut root = Node::new(object(vec![
            ("a", Value::Int(1)),
            ("b", Value::Int(2)),
            (
                "list",
                Value::Array(vec![Node::new(Value::Int(10)), Node::new(Value::Int(20))]),
            ),
        ]));
        let b: Path = "b".parse().unwrap();
        let item: Path = "list[0]".parse().unwrap();
        // Take a field and an item out, remembering where they were, and the
        // nodes themselves.
        let b_at = root.position_of(&b).unwrap();
        let b_node = root.remove(&b).unwrap();
        let item_node = root.remove(&item).unwrap();
        assert_eq!(b_at, 1);
        // Put them back: same place, same ids.
        root.insert_child(
            &Path::root(),
            b_at,
            &Segment::Key("b".into()),
            b_node.clone(),
        )
        .unwrap();
        root.insert_child(
            &"list".parse().unwrap(),
            0,
            &Segment::Index(0),
            item_node.clone(),
        )
        .unwrap();
        let keys: Vec<_> = root.value().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["a", "b", "list"]);
        assert_eq!(root.get(&b).unwrap().id(), b_node.id());
        assert_eq!(root.get(&item).unwrap().id(), item_node.id());
        assert_eq!(root.get(&item).unwrap().value(), &Value::Int(10));

        // Replace gives back the node it replaced.
        let old = root.replace(&b, Node::new(Value::Int(99))).unwrap();
        assert_eq!(old.id(), b_node.id());
        assert_eq!(root.get(&b).unwrap().value(), &Value::Int(99));
        let again = root.replace(&b, old.clone()).unwrap();
        assert_eq!(again.value(), &Value::Int(99));
        assert_eq!(root.get(&b).unwrap().id(), b_node.id());
    }

    #[test]
    fn position_replace_and_insert_child_refuse_what_does_not_fit() {
        let mut root = Node::new(object(vec![("a", Value::Int(1))]));
        let missing: Path = "zzz".parse().unwrap();
        assert_eq!(root.position_of(&missing), None);
        assert_eq!(root.position_of(&Path::root()), None);
        assert!(matches!(
            root.replace(&missing, Node::new(Value::Int(0))),
            Err(PathError::Missing(_))
        ));
        assert!(matches!(
            root.insert_child(
                &Path::root(),
                5,
                &Segment::Key("k".into()),
                Node::new(Value::Null)
            ),
            Err(PathError::OutOfRange(_))
        ));
        assert!(matches!(
            root.insert_child(&Path::root(), 0, &Segment::Index(0), Node::new(Value::Null)),
            Err(PathError::WrongKind(_))
        ));
        assert!(matches!(
            root.insert_child(
                &"nope".parse().unwrap(),
                0,
                &Segment::Index(0),
                Node::new(Value::Null)
            ),
            Err(PathError::Missing(_))
        ));
    }

    fn s(text: &str) -> Value {
        Value::Str(text.to_string())
    }

    fn object(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), Node::new(v)))
                .collect(),
        )
    }

    fn array(items: Vec<Value>) -> Value {
        Value::Array(items.into_iter().map(Node::new).collect())
    }

    fn sample() -> Node {
        Node::new(object(vec![
            ("name", s("Acme")),
            (
                "items",
                array(vec![
                    object(vec![("price", Value::Int(1))]),
                    object(vec![("price", Value::Int(2))]),
                ]),
            ),
            ("notes", array(vec![s("a"), s("b")])),
        ]))
    }

    fn p(text: &str) -> Path {
        text.parse().unwrap()
    }

    #[test]
    fn get_follows_paths() {
        let root = sample();
        assert_eq!(
            root.get(&p("items[1].price")).unwrap().value(),
            &Value::Int(2)
        );
        assert_eq!(root.get(&Path::root()).unwrap().id(), root.id());
        assert!(root.get(&p("items[2]")).is_none());
        assert!(root.get(&p("name.x")).is_none());
        assert!(root.get(&p("[0]")).is_none());
    }

    #[test]
    fn a_snapshot_is_unaffected_by_later_edits() {
        let mut root = sample();
        let snapshot = root.clone();
        root.set(&p("items[0].price"), Value::Int(99)).unwrap();
        assert_eq!(
            root.get(&p("items[0].price")).unwrap().value(),
            &Value::Int(99)
        );
        assert_eq!(
            snapshot.get(&p("items[0].price")).unwrap().value(),
            &Value::Int(1)
        );
    }

    #[test]
    fn an_edit_copies_only_the_path_to_the_node() {
        let mut root = sample();
        let snapshot = root.clone();
        root.set(&p("items[0].price"), Value::Int(99)).unwrap();
        // On the path: copied.
        assert!(!root.ptr_eq(&snapshot));
        assert!(
            !root
                .get(&p("items"))
                .unwrap()
                .ptr_eq(snapshot.get(&p("items")).unwrap())
        );
        assert!(
            !root
                .get(&p("items[0]"))
                .unwrap()
                .ptr_eq(snapshot.get(&p("items[0]")).unwrap())
        );
        // Off the path: still shared.
        assert!(
            root.get(&p("items[1]"))
                .unwrap()
                .ptr_eq(snapshot.get(&p("items[1]")).unwrap())
        );
        assert!(
            root.get(&p("name"))
                .unwrap()
                .ptr_eq(snapshot.get(&p("name")).unwrap())
        );
        assert!(
            root.get(&p("notes"))
                .unwrap()
                .ptr_eq(snapshot.get(&p("notes")).unwrap())
        );
    }

    #[test]
    fn editing_keeps_node_ids() {
        let mut root = sample();
        let before = (
            root.id(),
            root.get(&p("items[0]")).unwrap().id(),
            root.get(&p("items[0].price")).unwrap().id(),
        );
        root.set(&p("items[0].price"), Value::Int(5)).unwrap();
        let after = (
            root.id(),
            root.get(&p("items[0]")).unwrap().id(),
            root.get(&p("items[0].price")).unwrap().id(),
        );
        assert_eq!(before, after);
    }

    #[test]
    fn set_adds_a_field_and_appends_an_item() {
        let mut root = sample();
        root.set(&p("extra"), Value::Bool(true)).unwrap();
        assert_eq!(
            root.value().as_object().unwrap().keys().last().unwrap(),
            "extra"
        );
        root.set(&p("notes[2]"), s("c")).unwrap();
        assert_eq!(
            root.get(&p("notes"))
                .unwrap()
                .value()
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn set_does_not_create_missing_parents_or_gaps() {
        let mut root = sample();
        let snapshot = root.clone();
        assert_eq!(
            root.set(&p("nope.x"), Value::Null),
            Err(PathError::Missing(p("nope")))
        );
        assert_eq!(
            root.set(&p("notes[5]"), Value::Null),
            Err(PathError::OutOfRange(p("notes[5]")))
        );
        assert_eq!(
            root.set(&p("name[0]"), Value::Null),
            Err(PathError::WrongKind(p("name")))
        );
        assert_eq!(
            root.set(&p("notes.x"), Value::Null),
            Err(PathError::WrongKind(p("notes")))
        );
        // A failed edit copies nothing.
        assert!(root.ptr_eq(&snapshot));
    }

    #[test]
    fn remove_takes_a_node_out_and_shifts_items() {
        let mut root = sample();
        let snapshot = root.clone();
        let removed = root.remove(&p("notes[0]")).unwrap();
        assert_eq!(removed.value(), &s("a"));
        assert_eq!(root.get(&p("notes[0]")).unwrap().value(), &s("b"));
        assert_eq!(snapshot.get(&p("notes[0]")).unwrap().value(), &s("a"));
        let removed = root.remove(&p("name")).unwrap();
        assert_eq!(removed.value(), &s("Acme"));
        assert!(root.get(&p("name")).is_none());
        // Field order of the rest is kept.
        let keys: Vec<_> = root.value().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["items", "notes"]);
    }

    #[test]
    fn remove_reports_what_is_wrong() {
        let mut root = sample();
        assert_eq!(
            root.remove(&Path::root()),
            Err(PathError::WrongKind(Path::root()))
        );
        assert_eq!(root.remove(&p("nope")), Err(PathError::Missing(p("nope"))));
        assert_eq!(
            root.remove(&p("nope.x")),
            Err(PathError::Missing(p("nope")))
        );
        assert_eq!(
            root.remove(&p("notes[9]")),
            Err(PathError::OutOfRange(p("notes[9]")))
        );
        assert_eq!(
            root.remove(&p("name.x")),
            Err(PathError::WrongKind(p("name")))
        );
    }

    #[test]
    fn equality_compares_content_not_identity() {
        let a = Node::new(Value::Int(1));
        let b = Node::new(Value::Int(1));
        assert_ne!(a.id(), b.id());
        assert_eq!(a, b);
        assert_ne!(a, Node::new(Value::Int(2)));
        // Nested.
        assert_eq!(sample(), sample());
    }

    #[test]
    fn error_codes() {
        assert_eq!(ErrorKind::DivZero.code(), "#DIV/0!");
        assert_eq!(ErrorKind::Untrusted.to_string(), "#UNTRUSTED!");
    }
}
