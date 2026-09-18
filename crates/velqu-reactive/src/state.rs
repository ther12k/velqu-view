//! Constrained reactive state (M5c, ADR 0017).
//!
//! Reactive scope state is **plain data**, not arbitrary JS heap objects:
//! `null`, booleans, numbers, strings, arrays, and plain objects. This is
//! what makes a reactive turn transactional without heap snapshots —
//! committed state lives Rust-side, each turn builds a fresh candidate
//! JS object from it, and a failed turn simply never extracts the
//! candidate back.
//!
//! Functions, symbols, bigints, and exotic prototypes are rejected on
//! capture; depth and string-length budgets bound recursion and memory
//! independently of the QuickJS heap limit.

use rquickjs::{Ctx, FromJs, IntoJs, Object, Value};

/// Maximum state nesting depth on capture. Rust-side recursion must be
/// bounded explicitly; this also catches cycles a handler created
/// (`state.self = state`) that the heap limit alone would not refuse.
pub const MAX_STATE_DEPTH: usize = 32;

/// Plain reactive data.
#[derive(Debug, Clone, PartialEq)]
pub enum ReactiveValue {
    /// `null` (and `undefined`, normalized on capture).
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number (JS numbers are f64; `Int` coerces on capture).
    Number(f64),
    /// A string, already length-capped by [`crate::JsLimits`].
    String(String),
    /// An array of values.
    Array(Vec<ReactiveValue>),
    /// A plain object; keys in insertion (own enumerable) order.
    Object(Vec<(String, ReactiveValue)>),
}

impl ReactiveValue {
    /// ECMAScript `ToBoolean` for plain data: `false` for null, false,
    /// `0`/`NaN`, and `""`; `true` otherwise (nonempty arrays/objects
    /// included).
    pub fn truthy(&self) -> bool {
        match self {
            ReactiveValue::Null => false,
            ReactiveValue::Bool(value) => *value,
            ReactiveValue::Number(value) => *value != 0.0 && !value.is_nan(),
            ReactiveValue::String(value) => !value.is_empty(),
            ReactiveValue::Array(_) | ReactiveValue::Object(_) => true,
        }
    }

    /// Reads a dotted path (`a.b.c`); missing segments are `undefined`
    /// (represented as [`ReactiveValue::Null`]).
    pub fn get_path(&self, path: &str) -> ReactiveValue {
        let mut current = self;
        for segment in path.split('.') {
            let ReactiveValue::Object(entries) = current else {
                return ReactiveValue::Null;
            };
            match entries.iter().find(|(key, _)| key == segment) {
                Some((_, value)) => current = value,
                None => return ReactiveValue::Null,
            }
        }
        current.clone()
    }

    /// Writes a dotted path, creating intermediate objects; returns the
    /// previous value at the path.
    pub fn set_path(&mut self, path: &str, value: ReactiveValue) -> ReactiveValue {
        let mut segments = path.split('.');
        let Some(first) = segments.next() else {
            return ReactiveValue::Null;
        };
        if !matches!(self, ReactiveValue::Object(_)) {
            *self = ReactiveValue::Object(Vec::new());
        }
        let ReactiveValue::Object(entries) = self else {
            unreachable!("just normalized to an object");
        };
        if !entries.iter().any(|(key, _)| key == first) {
            entries.push((first.to_string(), ReactiveValue::Null));
        }
        let position = entries
            .iter()
            .position(|(key, _)| key == first)
            .expect("just inserted");
        let rest: Vec<&str> = segments.collect();
        if rest.is_empty() {
            let previous = entries.remove(position).1;
            entries.push((first.to_string(), value));
            return previous;
        }
        let next = &mut entries[position].1;
        if matches!(next, ReactiveValue::Null) {
            *next = ReactiveValue::Object(Vec::new());
        }
        next.set_path(&rest.join("."), value)
    }
}

impl<'js> IntoJs<'js> for ReactiveValue {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            ReactiveValue::Null => rquickjs::Null.into_js(ctx),
            ReactiveValue::Bool(value) => value.into_js(ctx),
            ReactiveValue::Number(value) => value.into_js(ctx),
            ReactiveValue::String(value) => value.into_js(ctx),
            ReactiveValue::Array(items) => {
                let array = rquickjs::Array::new(ctx.clone())?;
                for (index, item) in items.into_iter().enumerate() {
                    array.set(index, item.into_js(ctx)?)?;
                }
                array.into_js(ctx)
            }
            ReactiveValue::Object(entries) => {
                let object = Object::new(ctx.clone())?;
                for (key, value) in entries {
                    object.set(key.as_str(), value.into_js(ctx)?)?;
                }
                object.into_js(ctx)
            }
        }
    }
}

/// Why a candidate state failed capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// A value outside the plain-data profile (function, symbol, bigint,
    /// exotic object).
    UnsupportedType(String),
    /// Nesting exceeded [`MAX_STATE_DEPTH`] (also the cycle backstop).
    TooDeep,
    /// A string exceeded the output-string budget.
    StringTooLong {
        /// The offending length in bytes.
        size: usize,
        /// The configured maximum.
        max: usize,
    },
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::UnsupportedType(kind) => {
                write!(
                    f,
                    "reactive state must be plain data; {kind} is not allowed"
                )
            }
            StateError::TooDeep => write!(
                f,
                "reactive state nested deeper than {MAX_STATE_DEPTH} levels"
            ),
            StateError::StringTooLong { size, max } => {
                write!(
                    f,
                    "state string of {size} bytes exceeds the {max}-byte budget"
                )
            }
        }
    }
}

/// Captures a JS value into plain reactive data. `is_plain` is the
/// profile's plain-object predicate (`Object.getPrototypeOf(v) ===
/// Object.prototype || null`); `max_string` is the per-string budget.
pub(crate) fn capture<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    is_plain: &rquickjs::Function<'js>,
    max_string: usize,
) -> Result<ReactiveValue, StateError> {
    capture_at(ctx, value, is_plain, max_string, 0)
}

fn capture_at<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    is_plain: &rquickjs::Function<'js>,
    max_string: usize,
    depth: usize,
) -> Result<ReactiveValue, StateError> {
    use rquickjs::Type;
    if depth > MAX_STATE_DEPTH {
        return Err(StateError::TooDeep);
    }
    match value.type_of() {
        Type::Uninitialized | Type::Undefined | Type::Null => Ok(ReactiveValue::Null),
        Type::Bool => bool::from_js(ctx, value.clone())
            .map(ReactiveValue::Bool)
            .map_err(|_| StateError::UnsupportedType("boolean".to_owned())),
        Type::Int | Type::Float => f64::from_js(ctx, value.clone())
            .map(ReactiveValue::Number)
            .map_err(|_| StateError::UnsupportedType("number".to_owned())),
        Type::String => {
            let text = rquickjs::String::from_js(ctx, value.clone())
                .map_err(|_| StateError::UnsupportedType("string".to_owned()))?
                .to_string()
                .map_err(|_| StateError::UnsupportedType("string".to_owned()))?;
            if text.len() > max_string {
                return Err(StateError::StringTooLong {
                    size: text.len(),
                    max: max_string,
                });
            }
            Ok(ReactiveValue::String(text))
        }
        Type::Array => {
            let array = rquickjs::Array::from_js(ctx, value.clone())
                .map_err(|_| StateError::UnsupportedType("array".to_owned()))?;
            let mut items = Vec::new();
            for item in array.iter::<Value>() {
                let item = item.map_err(|_| StateError::UnsupportedType("array".to_owned()))?;
                items.push(capture_at(ctx, &item, is_plain, max_string, depth + 1)?);
            }
            Ok(ReactiveValue::Array(items))
        }
        Type::Object => {
            let plain: bool = is_plain
                .call((value.clone(),))
                .map_err(|_| StateError::UnsupportedType("object".to_owned()))?;
            if !plain {
                return Err(StateError::UnsupportedType(
                    "an object with an exotic prototype".to_owned(),
                ));
            }
            let object = Object::from_js(ctx, value.clone())
                .map_err(|_| StateError::UnsupportedType("object".to_owned()))?;
            let mut entries = Vec::new();
            for entry in object.into_iter() {
                let (key, item) =
                    entry.map_err(|_| StateError::UnsupportedType("object".to_owned()))?;
                let key = key
                    .to_string()
                    .map_err(|_| StateError::UnsupportedType("object key".to_owned()))?;
                let value = capture_at(ctx, &item, is_plain, max_string, depth + 1)?;
                entries.push((key, value));
            }
            Ok(ReactiveValue::Object(entries))
        }
        Type::Function => Err(StateError::UnsupportedType("a function".to_owned())),
        Type::Symbol => Err(StateError::UnsupportedType("a symbol".to_owned())),
        Type::BigInt => Err(StateError::UnsupportedType("a bigint".to_owned())),
        other => Err(StateError::UnsupportedType(format!("{other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness_matches_ecmascript_for_plain_data() {
        assert!(!ReactiveValue::Null.truthy());
        assert!(!ReactiveValue::Bool(false).truthy());
        assert!(ReactiveValue::Bool(true).truthy());
        assert!(!ReactiveValue::Number(0.0).truthy());
        assert!(!ReactiveValue::Number(f64::NAN).truthy());
        assert!(ReactiveValue::Number(-0.5).truthy());
        assert!(!ReactiveValue::String(String::new()).truthy());
        assert!(ReactiveValue::String("0".to_owned()).truthy());
        assert!(ReactiveValue::Array(vec![]).truthy());
        assert!(ReactiveValue::Object(vec![]).truthy());
    }

    #[test]
    fn dotted_paths_read_and_write() {
        let mut state = ReactiveValue::Object(vec![(
            "user".to_owned(),
            ReactiveValue::Object(vec![(
                "name".to_owned(),
                ReactiveValue::String("Ada".into()),
            )]),
        )]);
        assert_eq!(
            state.get_path("user.name"),
            ReactiveValue::String("Ada".into())
        );
        assert_eq!(state.get_path("user.missing"), ReactiveValue::Null);
        assert_eq!(state.get_path("user.name.deeper"), ReactiveValue::Null);

        let previous = state.set_path("gate.lane", ReactiveValue::Number(3.0));
        assert_eq!(previous, ReactiveValue::Null);
        assert_eq!(state.get_path("gate.lane"), ReactiveValue::Number(3.0));
        let previous = state.set_path("gate.lane", ReactiveValue::Number(4.0));
        assert_eq!(previous, ReactiveValue::Number(3.0));
    }
}
