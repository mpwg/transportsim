use std::fmt;

use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::map::Entry;
use serde_json::{Map, Value};

use super::v1::Payload;
use crate::{Simulation, ValidationError};

const FORMAT_VERSION: u32 = 1;
const RULES_VERSION: u32 = 1;
const MAX_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug)]
#[non_exhaustive]
pub enum SaveError {
    Json { message: String },
    TooLarge { size: usize, limit: usize },
}

impl SaveError {
    fn json(error: serde_json::Error) -> Self {
        Self::Json {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { message } => write!(out, "could not encode the checkpoint: {message}"),
            Self::TooLarge { size, limit } => {
                write!(out, "checkpoint is {size} bytes; limit is {limit} bytes")
            }
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// Invalid JSON or a value that does not match the wire schema.
    /// Line and column are zero when no source location is available.
    Json {
        message: String,
        line: usize,
        column: usize,
    },
    UnsupportedFormat {
        version: u32,
    },
    UnsupportedRules {
        version: u32,
    },
    ChecksumMismatch,
    TooLarge {
        size: usize,
        limit: usize,
    },
    State(ValidationError),
    InvalidState {
        field: &'static str,
    },
}

impl LoadError {
    fn json(error: serde_json::Error) -> Self {
        Self::Json {
            line: error.line(),
            column: error.column(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { message, .. } => write!(out, "invalid JSON document: {message}"),
            Self::UnsupportedFormat { version } => {
                write!(out, "unsupported checkpoint format version {version}")
            }
            Self::UnsupportedRules { version } => {
                write!(out, "unsupported simulation rules version {version}")
            }
            Self::ChecksumMismatch => out.write_str("checkpoint payload checksum does not match"),
            Self::TooLarge { size, limit } => {
                write!(out, "JSON document is {size} bytes; limit is {limit} bytes")
            }
            Self::State(error) => write!(out, "invalid checkpoint state: {error}"),
            Self::InvalidState { field } => write!(out, "invalid checkpoint state at {field}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format_version: u32,
    rules_version: u32,
    checksum: String,
    payload: Value,
}

impl Simulation {
    /// Encodes the current state as a versioned JSON checkpoint.
    pub fn save(&self) -> Result<Vec<u8>, SaveError> {
        let (payload, hash) = self.checkpoint_payload().map_err(SaveError::json)?;
        let envelope = Envelope {
            format_version: FORMAT_VERSION,
            rules_version: RULES_VERSION,
            checksum: hash.to_hex().to_string(),
            payload,
        };
        let bytes = serde_json::to_vec(&envelope).map_err(SaveError::json)?;
        if bytes.len() > MAX_BYTES {
            return Err(SaveError::TooLarge {
                size: bytes.len(),
                limit: MAX_BYTES,
            });
        }
        Ok(bytes)
    }

    /// Validates a checkpoint and restores a new simulation from it.
    pub fn load(bytes: &[u8]) -> Result<Self, LoadError> {
        let document = read_object(bytes)?;
        let format_version = version_field(&document, "format_version")?;
        if format_version != FORMAT_VERSION {
            return Err(LoadError::UnsupportedFormat {
                version: format_version,
            });
        }
        let rules_version = version_field(&document, "rules_version")?;
        if rules_version != RULES_VERSION {
            return Err(LoadError::UnsupportedRules {
                version: rules_version,
            });
        }
        let mut envelope: Envelope =
            serde_json::from_value(Value::Object(document)).map_err(LoadError::json)?;
        let hash = canonical_hash(&mut envelope.payload).map_err(LoadError::json)?;
        if envelope.checksum != hash.to_hex().as_str() {
            return Err(LoadError::ChecksumMismatch);
        }
        let payload: Payload = decode_object(envelope.payload)?;
        payload.into_simulation()
    }

    /// Returns the BLAKE3 hash of the canonical checkpoint payload.
    pub fn fingerprint(&self) -> Result<[u8; 32], SaveError> {
        let (_, hash) = self.checkpoint_payload().map_err(SaveError::json)?;
        Ok(*hash.as_bytes())
    }

    fn checkpoint_payload(&self) -> Result<(Value, blake3::Hash), serde_json::Error> {
        let mut payload = serde_json::to_value(Payload::from_simulation(self))?;
        let hash = canonical_hash(&mut payload)?;
        Ok((payload, hash))
    }
}

fn canonical_hash(payload: &mut Value) -> Result<blake3::Hash, serde_json::Error> {
    // JSON object keys are lexicographic at every depth. Array order is significant.
    payload.sort_all_objects();
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, payload)?;
    Ok(hasher.finalize())
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, LoadError> {
    serde_json::from_value(Value::Object(read_object(bytes)?)).map_err(LoadError::json)
}

fn read_object(bytes: &[u8]) -> Result<Map<String, Value>, LoadError> {
    if bytes.len() > MAX_BYTES {
        return Err(LoadError::TooLarge {
            size: bytes.len(),
            limit: MAX_BYTES,
        });
    }
    let UniqueJson(value) = serde_json::from_slice(bytes).map_err(LoadError::json)?;
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(expected_object()),
    }
}

fn decode_object<T: DeserializeOwned>(value: Value) -> Result<T, LoadError> {
    if !value.is_object() {
        return Err(expected_object());
    }
    serde_json::from_value(value).map_err(LoadError::json)
}

fn expected_object() -> LoadError {
    LoadError::json(de::Error::custom("expected a JSON object"))
}

fn version_field(document: &Map<String, Value>, field: &'static str) -> Result<u32, LoadError> {
    let value = document
        .get(field)
        .ok_or_else(|| LoadError::json(de::Error::missing_field(field)))?;
    u32::deserialize(value).map_err(LoadError::json)
}

// A plain serde_json::Value would overwrite repeated object keys during parsing.
struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueJsonVisitor).map(Self)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = Value;

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("a JSON value with unique object keys")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("expected a finite JSON number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(UniqueJson(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            match values.entry(key) {
                Entry::Vacant(entry) => {
                    let UniqueJson(value) = map.next_value()?;
                    entry.insert(value);
                }
                Entry::Occupied(entry) => {
                    return Err(de::Error::custom(format!(
                        "duplicate object key {:?}",
                        entry.key()
                    )));
                }
            }
        }
        Ok(Value::Object(values))
    }
}
