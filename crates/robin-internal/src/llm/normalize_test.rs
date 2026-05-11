#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::normalize::{strip_fields, reset_strip_cache};

    #[test]
    fn test_strip_fields_top_level() {
        let schema = json!({
            "type": "object",
            "properties": {"x": {"type": "string"}},
            "$ref": "#/defs/foo"
        });
        let (out, diags) = strip_fields("mytool", &schema, &["$ref"]);
        let parsed = out.as_object().unwrap();
        assert!(!parsed.contains_key("$ref"), "$ref must be stripped");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].tool_name, "mytool");
        assert_eq!(diags[0].field, "$ref");
        assert_eq!(diags[0].action, "stripped");
    }

    #[test]
    fn test_strip_fields_nested() {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri"},
                "items": {
                    "type": "array",
                    "items": {"type": "string", "format": "email"}
                }
            }
        });
        let (out, diags) = strip_fields("read", &schema, &["format"]);
        let props = out["properties"].as_object().unwrap();
        assert!(!props["url"].as_object().unwrap().contains_key("format"),
            "nested format under properties.url must be stripped");
        let items_items = &props["items"]["items"];
        assert!(!items_items.as_object().unwrap().contains_key("format"),
            "format under properties.items.items must be stripped");
        assert_eq!(diags.len(), 2);
        // Keys visited in sorted order: "items" < "url" alphabetically
        assert_eq!(diags[0].field, "properties.items.items.format");
        assert_eq!(diags[1].field, "properties.url.format");
    }

    #[test]
    fn test_strip_fields_additional_properties() {
        let schema = json!({
            "type": "object",
            "additionalProperties": {"type": "string", "$ref": "#/x"}
        });
        let (out, diags) = strip_fields("t", &schema, &["$ref"]);
        let addl = out["additionalProperties"].as_object().unwrap();
        assert!(!addl.contains_key("$ref"));
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].field, "additionalProperties.$ref");
    }

    #[test]
    fn test_strip_fields_no_op() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let (out, diags) = strip_fields("t", &schema, &["$ref", "format"]);
        assert_eq!(out, schema, "no fields to strip → structurally unchanged");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_strip_fields_deterministic() {
        let schema = json!({
            "properties": {
                "a": {"format": "uri"},
                "b": {"format": "email"},
                "c": {"format": "date"}
            }
        });
        let (out1, diags1) = strip_fields("t", &schema, &["format"]);
        let (out2, diags2) = strip_fields("t", &schema, &["format"]);
        assert_eq!(out1, out2);
        assert_eq!(diags1.len(), diags2.len());
        for (d1, d2) in diags1.iter().zip(diags2.iter()) {
            assert_eq!(d1.field, d2.field);
        }
    }

    #[test]
    fn test_strip_fields_empty_inputs() {
        // Empty fields list → no-op
        let schema = json!({"x": 1});
        let (out, diags) = strip_fields("t", &schema, &[]);
        assert_eq!(out, schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_strip_fields_malformed_schema() {
        // Null/non-object schema returns unchanged
        let schema = serde_json::Value::Null;
        let (out, diags) = strip_fields("t", &schema, &["$ref"]);
        assert_eq!(out, serde_json::Value::Null);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_reset_strip_cache() {
        reset_strip_cache();
    }
}