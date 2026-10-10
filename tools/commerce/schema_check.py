"""ECDEV-owned JSON Schema subset for the committed record schemas (tools/commerce/schemas).

Supports exactly the keywords those schemas use: type, enum, const, minimum, pattern, properties,
required, additionalProperties (boolean), allOf, if/then. Any other keyword is refused, so a schema
that outgrows the subset fails loudly instead of validating nothing.
"""
import re

SUPPORTED = {"$schema", "$id", "title", "description", "type", "enum", "const", "minimum", "pattern", "properties",
             "required", "additionalProperties", "allOf", "if", "then", "default"}
TYPES = {"object": dict, "array": list, "string": str, "boolean": bool, "null": type(None)}


def _is(value, name):
    if name == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if name == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if name == "boolean":
        return isinstance(value, bool)
    return isinstance(value, TYPES[name])


def errors(value, schema, path="$"):
    found = []
    for key in schema:
        if key not in SUPPORTED:
            raise ValueError(f"unsupported schema keyword {key!r} at {path}")
    if "type" in schema:
        names = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        if not any(_is(value, n) for n in names):
            return [f"{path}: expected {names}"]
    if "enum" in schema and value not in schema["enum"]:
        found.append(f"{path}: not in enum")
    if "const" in schema and value != schema["const"]:
        found.append(f"{path}: not the constant")
    if "minimum" in schema and _is(value, "number") and value < schema["minimum"]:
        found.append(f"{path}: below minimum")
    if "pattern" in schema and isinstance(value, str) and not re.search(schema["pattern"], value):
        found.append(f"{path}: pattern mismatch")
    if isinstance(value, dict):
        for name in schema.get("required", []):
            if name not in value:
                found.append(f"{path}: missing {name}")
        props = schema.get("properties", {})
        for name, sub in props.items():
            if name in value:
                found += errors(value[name], sub, f"{path}.{name}")
        if schema.get("additionalProperties") is False:
            found += [f"{path}: unexpected {n}" for n in value if n not in props]
    for sub in schema.get("allOf", []):
        found += errors(value, sub, path)
    if "if" in schema and not errors(value, schema["if"], path) and "then" in schema:
        found += errors(value, schema["then"], path)
    return found


def validate(value, schema):
    found = errors(value, schema)
    if found:
        raise ValueError("; ".join(found[:5]))
