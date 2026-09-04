use std::collections::BTreeSet;

use serde_json::Value;

pub(crate) fn status_fields(value: &Value) -> Option<(String, u32)> {
    let object = value.as_object()?;
    let version = object.get("version")?.as_str()?.to_owned();
    let protocol = as_u32(object.get("protocol")?)?;
    Some((version, protocol))
}

pub(crate) fn schema_fields(value: &Value) -> Option<(u32, BTreeSet<String>)> {
    let object = value.as_object()?;
    let schema_version = as_u32(object.get("schema_version")?)?;
    let request = object
        .get("schemas")?
        .as_object()?
        .get("request")?
        .as_object()?;
    let declarations = request.get("oneOf")?.as_array()?;
    let methods = declarations
        .iter()
        .filter_map(|declaration| declaration.as_object())
        .filter_map(|declaration| declaration.get("properties"))
        .filter_map(Value::as_object)
        .filter_map(|properties| properties.get("method"))
        .filter_map(Value::as_object)
        .filter_map(|method| method.get("const"))
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Some((schema_version, methods))
}

fn as_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}
