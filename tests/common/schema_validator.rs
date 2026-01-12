use jsonschema::{Draft, JSONSchema};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Load a JSON schema from a file
pub fn load_schema(schema_path: &str) -> JSONSchema {
    let schema_content = fs::read_to_string(schema_path)
        .unwrap_or_else(|_| panic!("Failed to read schema file: {}", schema_path));
    
    let schema_json: Value = serde_json::from_str(&schema_content)
        .unwrap_or_else(|_| panic!("Failed to parse schema JSON: {}", schema_path));
    
    JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema_json)
        .expect("Failed to compile schema")
}

/// Validate a JSON value against a schema
pub fn validate_against_schema(data: &Value, schema: &JSONSchema) -> Result<(), Vec<String>> {
    match schema.validate(data) {
        Ok(_) => Ok(()),
        Err(errors) => {
            let error_messages: Vec<String> = errors
                .map(|e| format!("{} at {}", e, e.instance_path))
                .collect();
            Err(error_messages)
        }
    }
}

/// Load schema from tests/schemas directory
pub fn load_test_schema(schema_name: &str) -> JSONSchema {
    let schema_path = format!("tests/schemas/{}.json", schema_name);
    load_schema(&schema_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_simple_schema_validation() {
        let schema_json = json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"}
            },
            "required": ["id"]
        });

        let schema = JSONSchema::compile(&schema_json).expect("Invalid schema");

        // Valid data
        let valid_data = json!({"id": "123", "name": "Test"});
        assert!(validate_against_schema(&valid_data, &schema).is_ok());

        // Invalid data (missing required field)
        let invalid_data = json!({"name": "Test"});
        assert!(validate_against_schema(&invalid_data, &schema).is_err());
    }
}
