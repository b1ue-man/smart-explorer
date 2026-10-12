//! Typed access to a tool call's JSON arguments with messages that name the
//! argument, so an agent can correct its call.
use rmcp::model::JsonObject;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct Args<'a> {
    map: Option<&'a JsonObject>,
}

impl<'a> Args<'a> {
    pub(super) fn new(map: Option<&'a JsonObject>) -> Self {
        Self { map }
    }

    /// The raw JSON of an argument; `None` when absent or null.
    pub(super) fn value(&self, key: &str) -> Option<&'a Value> {
        self.map
            .and_then(|map| map.get(key))
            .filter(|value| !value.is_null())
    }

    /// A required, non-empty string.
    pub(super) fn string(&self, key: &str) -> Result<String, String> {
        match self.opt_string(key)? {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(format!("missing required argument `{key}`")),
        }
    }

    pub(super) fn opt_string(&self, key: &str) -> Result<Option<String>, String> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(format!("argument `{key}` must be a string")),
        }
    }

    /// A boolean that defaults to `false`.
    pub(super) fn flag(&self, key: &str) -> Result<bool, String> {
        match self.value(key) {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(format!("argument `{key}` must be true or false")),
        }
    }

    pub(super) fn opt_u64(&self, key: &str) -> Result<Option<u64>, String> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Number(number)) => number
                .as_u64()
                .or_else(|| {
                    number
                        .as_f64()
                        .filter(|value| *value >= 0.0 && value.fract() == 0.0)
                        .map(|value| value as u64)
                })
                .map(Some)
                .ok_or_else(|| format!("argument `{key}` must be a non-negative integer")),
            Some(_) => Err(format!("argument `{key}` must be a non-negative integer")),
        }
    }

    pub(super) fn opt_strings(&self, key: &str) -> Result<Option<Vec<String>>, String> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("argument `{key}` must be a list of strings"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err(format!("argument `{key}` must be a list of strings")),
        }
    }

    pub(super) fn string_map(&self, key: &str) -> Result<BTreeMap<String, String>, String> {
        match self.value(key) {
            None => Ok(BTreeMap::new()),
            Some(Value::Object(entries)) => entries
                .iter()
                .map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_string()))
                        .ok_or_else(|| format!("argument `{key}` must map names to strings"))
                })
                .collect(),
            Some(_) => Err(format!("argument `{key}` must map names to strings")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use serde_json::json;

    #[test]
    fn mcp_task_arguments_are_typed_and_named_in_errors() {
        let value = json!({
            "target": "@prod:/srv",
            "recursive": true,
            "limit": 25,
            "argv": ["ls", "-la"],
            "env": {"MODE": "x"},
            "null": null
        });
        let map = value.as_object().unwrap();
        let args = Args::new(Some(map));
        assert_eq!(args.string("target").unwrap(), "@prod:/srv");
        assert!(args.flag("recursive").unwrap());
        assert!(!args.flag("missing").unwrap());
        assert_eq!(args.opt_u64("limit").unwrap(), Some(25));
        assert_eq!(args.opt_u64("null").unwrap(), None);
        assert_eq!(args.opt_strings("argv").unwrap().unwrap(), ["ls", "-la"]);
        assert_eq!(args.string_map("env").unwrap()["MODE"], "x");
        assert!(args.string("missing").unwrap_err().contains("`missing`"));
        assert!(args.flag("target").unwrap_err().contains("`target`"));
    }
}
