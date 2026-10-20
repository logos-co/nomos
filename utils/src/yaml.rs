use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_yaml::Value;
use tracing::warn;

const LOG_TARGET: &str = lb_log_targets::utils::YAML;

const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(thiserror::Error, Debug)]
pub enum ValueDeserializationError<Value> {
    #[error("Unrecognized fields in value: {fields:?}")]
    UnrecognizedFields { fields: Vec<String>, value: Value },
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    SerdeError(#[from] serde_yaml::Error),
    #[error("YAML include error: {0}")]
    IncludeError(String),
}

pub enum OnUnknownKeys {
    Fail,
    Warn,
}

pub fn deserialize_value_at_path<Value>(
    path: &Path,
    unknown_keys_strategy: OnUnknownKeys,
) -> Result<Value, ValueDeserializationError<Value>>
where
    Value: for<'de> Deserialize<'de>,
{
    let base_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file = std::fs::File::open(path)?;
    let raw: serde_yaml::Value = serde_yaml::from_reader(file)?;
    let resolved = resolve_includes(raw, &base_dir).map_err(ValueDeserializationError::from)?;
    deserialize_from_value(resolved, unknown_keys_strategy)
}

fn deserialize_from_value<Value>(
    value: serde_yaml::Value,
    unknown_keys_strategy: OnUnknownKeys,
) -> Result<Value, ValueDeserializationError<Value>>
where
    Value: for<'de> Deserialize<'de>,
{
    use serde::de::IntoDeserializer as _;
    let mut ignored_fields = Vec::new();
    let value = serde_ignored::deserialize::<_, _, Value>(value.into_deserializer(), |path| {
        ignored_fields.push(path.to_string());
    })?;
    apply_unknown_keys_strategy(value, ignored_fields, unknown_keys_strategy)
}

pub fn deserialize_value_from_reader<Value, Reader>(
    reader: Reader,
    unknown_keys_strategy: OnUnknownKeys,
) -> Result<Value, ValueDeserializationError<Value>>
where
    Value: for<'de> Deserialize<'de>,
    Reader: Read,
{
    let mut ignored_fields = Vec::new();
    let value = serde_ignored::deserialize::<_, _, Value>(
        serde_yaml::Deserializer::from_reader(reader),
        |path| {
            ignored_fields.push(path.to_string());
        },
    )?;
    apply_unknown_keys_strategy(value, ignored_fields, unknown_keys_strategy)
}

fn apply_unknown_keys_strategy<Value>(
    value: Value,
    ignored_fields: Vec<String>,
    strategy: OnUnknownKeys,
) -> Result<Value, ValueDeserializationError<Value>> {
    match (ignored_fields, strategy) {
        (ignored_fields, _) if ignored_fields.is_empty() => Ok(value),
        (ignored_fields, OnUnknownKeys::Warn) => {
            let message = format!(
                "The following unrecognized fields were found in the value: {ignored_fields:?}."
            );
            warn!(
                target: LOG_TARGET,
                message
            );
            eprintln!("{message}");
            Ok(value)
        }
        (ignored_fields, OnUnknownKeys::Fail) => {
            Err(ValueDeserializationError::UnrecognizedFields {
                fields: ignored_fields,
                value,
            })
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum YamlIncludeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_yaml::Error),
    #[error("!include error: {0}")]
    InvalidInclude(String),
}

impl<Value> From<YamlIncludeError> for ValueDeserializationError<Value> {
    fn from(e: YamlIncludeError) -> Self {
        use YamlIncludeError as E;
        match e {
            E::Io(e) => Self::IoError(e),
            E::Serde(e) => Self::SerdeError(e),
            E::InvalidInclude(msg) => Self::IncludeError(msg),
        }
    }
}

/// Recursively resolves `!include <path>` tags in a YAML value.
///
/// Paths are resolved relative to `base_dir`. Nested includes are supported
/// (each included file's own includes resolve relative to that file's
/// directory). An include cycle or a chain deeper than [`MAX_INCLUDE_DEPTH`]
/// is rejected.
fn resolve_includes(value: Value, base_dir: &Path) -> Result<Value, YamlIncludeError> {
    resolve_includes_within(value, base_dir, &mut HashSet::new())
}

// `include_stack` holds the canonical paths of the files currently being
// included.
fn resolve_includes_within(
    value: Value,
    base_dir: &Path,
    include_stack: &mut HashSet<PathBuf>,
) -> Result<Value, YamlIncludeError> {
    match value {
        Value::Tagged(tagged) if tagged.tag == "include" => {
            let path_str = match &tagged.value {
                Value::String(s) => s.clone(),
                other => {
                    return Err(YamlIncludeError::InvalidInclude(format!(
                        "!include requires a string path, got: {other:?}"
                    )));
                }
            };
            let resolved_path = if Path::new(&path_str).is_absolute() {
                PathBuf::from(&path_str)
            } else {
                base_dir.join(&path_str)
            };
            if include_stack.len() >= MAX_INCLUDE_DEPTH {
                return Err(YamlIncludeError::InvalidInclude(format!(
                    "!include depth exceeds {MAX_INCLUDE_DEPTH} at {}",
                    resolved_path.display()
                )));
            }
            let canonical_path = resolved_path.canonicalize()?;
            if !include_stack.insert(canonical_path.clone()) {
                return Err(YamlIncludeError::InvalidInclude(format!(
                    "!include cycle at {}",
                    resolved_path.display()
                )));
            }
            let contents = std::fs::read_to_string(&resolved_path)?;
            let included: Value = serde_yaml::from_str(&contents)?;
            let new_base = resolved_path.parent().unwrap_or(base_dir).to_path_buf();
            let resolved = resolve_includes_within(included, &new_base, include_stack);
            include_stack.remove(&canonical_path);
            resolved
        }
        Value::Mapping(map) => {
            let resolved = map
                .into_iter()
                .map(|(k, v)| Ok((k, resolve_includes_within(v, base_dir, include_stack)?)))
                .collect::<Result<serde_yaml::Mapping, YamlIncludeError>>()?;
            Ok(Value::Mapping(resolved))
        }
        Value::Sequence(seq) => {
            let resolved = seq
                .into_iter()
                .map(|v| resolve_includes_within(v, base_dir, include_stack))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Sequence(resolved))
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_yaml::{
        Value,
        value::{Tag, TaggedValue},
    };
    use tempfile::TempDir;

    use super::{MAX_INCLUDE_DEPTH, YamlIncludeError, resolve_includes};

    fn include_tag(path: &str) -> Value {
        Value::Tagged(Box::new(TaggedValue {
            tag: Tag::new("include"),
            value: Value::String(path.into()),
        }))
    }

    fn mapping(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        let m = pairs
            .into_iter()
            .map(|(k, v)| (Value::String(k.into()), v))
            .collect();
        Value::Mapping(m)
    }

    #[test]
    fn passthrough_when_no_includes() {
        let input = mapping([
            ("a", Value::Number(1.into())),
            ("b", Value::Bool(true)),
            ("c", Value::String("hello".into())),
        ]);
        let result = resolve_includes(input.clone(), Path::new(".")).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn include_substitutes_file_contents() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("sub.yaml"), "value: 42\n").unwrap();

        let input = mapping([("key", include_tag("sub.yaml"))]);
        let result = resolve_includes(input, dir.path()).unwrap();

        let expected = mapping([("key", mapping([("value", Value::Number(42.into()))]))]);
        assert_eq!(result, expected);
    }

    #[test]
    fn include_in_sequence() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.yaml"), "1\n").unwrap();
        fs::write(dir.path().join("b.yaml"), "2\n").unwrap();

        let input = Value::Sequence(vec![include_tag("a.yaml"), include_tag("b.yaml")]);
        let result = resolve_includes(input, dir.path()).unwrap();

        assert_eq!(
            result,
            Value::Sequence(vec![Value::Number(1.into()), Value::Number(2.into())])
        );
    }

    #[test]
    fn nested_include() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("c.yaml"), "leaf: true\n").unwrap();
        fs::write(dir.path().join("b.yaml"), "nested: !include c.yaml\n").unwrap();
        fs::write(dir.path().join("a.yaml"), "top: !include b.yaml\n").unwrap();

        let input: Value = serde_yaml::from_str("root: !include a.yaml\n").unwrap();
        let result = resolve_includes(input, dir.path()).unwrap();

        let expected = mapping([(
            "root",
            mapping([(
                "top",
                mapping([("nested", mapping([("leaf", Value::Bool(true))]))]),
            )]),
        )]);
        assert_eq!(result, expected);
    }

    #[test]
    fn include_resolves_relative_to_included_file_directory() {
        let dir = TempDir::new().unwrap();
        let sub_dir = dir.path().join("sub");
        fs::create_dir_all(&sub_dir).unwrap();

        fs::write(sub_dir.join("leaf.yaml"), "x: 1\n").unwrap();
        fs::write(sub_dir.join("inner.yaml"), "data: !include leaf.yaml\n").unwrap();

        let input = mapping([("result", include_tag("sub/inner.yaml"))]);
        let result = resolve_includes(input, dir.path()).unwrap();

        let expected = mapping([(
            "result",
            mapping([("data", mapping([("x", Value::Number(1.into()))]))]),
        )]);
        assert_eq!(result, expected);
    }

    #[test]
    fn absolute_path_include() {
        let dir = TempDir::new().unwrap();
        let abs_path = dir.path().join("abs.yaml");
        fs::write(&abs_path, "absolute: true\n").unwrap();

        let input = mapping([("k", include_tag(abs_path.to_str().unwrap()))]);
        let result = resolve_includes(input, Path::new("/nonexistent")).unwrap();

        let expected = mapping([("k", mapping([("absolute", Value::Bool(true))]))]);
        assert_eq!(result, expected);
    }

    #[test]
    fn missing_file_returns_io_error() {
        let input = mapping([("k", include_tag("does_not_exist.yaml"))]);
        let err = resolve_includes(input, Path::new(".")).unwrap_err();
        assert!(matches!(err, YamlIncludeError::Io(_)));
    }

    #[test]
    fn non_string_include_value_returns_invalid_include_error() {
        let input = Value::Tagged(Box::new(TaggedValue {
            tag: Tag::new("include"),
            value: Value::Number(42.into()),
        }));
        let err = resolve_includes(input, Path::new(".")).unwrap_err();
        assert!(matches!(err, YamlIncludeError::InvalidInclude(_)));
    }

    #[test]
    fn self_include_returns_invalid_include_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("self.yaml"), "x: !include self.yaml\n").unwrap();

        let input = mapping([("k", include_tag("self.yaml"))]);
        let err = resolve_includes(input, dir.path()).unwrap_err();
        assert!(matches!(err, YamlIncludeError::InvalidInclude(_)));
    }

    #[test]
    fn include_chain_deeper_than_limit_returns_invalid_include_error() {
        let dir = TempDir::new().unwrap();
        // Distinct files `0.yaml` to `{MAX_INCLUDE_DEPTH}.yaml`, each including the
        // next one.
        for i in 0..=MAX_INCLUDE_DEPTH {
            let next = i.checked_add(1).unwrap();
            fs::write(
                dir.path().join(format!("{i}.yaml")),
                format!("x: !include {next}.yaml\n"),
            )
            .unwrap();
        }
        fs::write(
            dir.path().join(format!(
                "{}.yaml",
                MAX_INCLUDE_DEPTH.checked_add(1).unwrap()
            )),
            "leaf: true\n",
        )
        .unwrap();

        let err = resolve_includes(include_tag("0.yaml"), dir.path()).unwrap_err();
        assert!(matches!(err, YamlIncludeError::InvalidInclude(_)));
    }
}
