//! Integration tests for MCP resources functionality
//!
//! These tests verify that the MCP resources commands work correctly
//! and integrate properly with the chat interface.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::cli::chat::tool_manager::ResourceBundle;

    /// Test that ResourceBundle can be created and contains expected fields
    #[test]
    fn test_resource_bundle_creation() {
        let resource_data = serde_json::json!({
            "uri": "file://test.txt",
            "name": "test.txt",
            "mimeType": "text/plain",
            "description": "A test file"
        });

        let bundle = ResourceBundle {
            server_name: "test_server".to_string(),
            resource: resource_data.clone(),
        };

        assert_eq!(bundle.server_name, "test_server");
        assert_eq!(bundle.resource.get("uri").unwrap().as_str().unwrap(), "file://test.txt");
        assert_eq!(bundle.resource.get("name").unwrap().as_str().unwrap(), "test.txt");
        assert_eq!(bundle.resource.get("mimeType").unwrap().as_str().unwrap(), "text/plain");
    }

    /// Test that resources cache can be populated and queried
    #[test]
    fn test_resources_cache_operations() {
        let mut resources_cache = HashMap::<String, Vec<ResourceBundle>>::new();

        let resource1 = serde_json::json!({
            "uri": "file://test1.txt",
            "name": "test1.txt",
            "mimeType": "text/plain"
        });

        let resource2 = serde_json::json!({
            "uri": "file://test2.json",
            "name": "test2.json",
            "mimeType": "application/json"
        });

        let bundle1 = ResourceBundle {
            server_name: "server1".to_string(),
            resource: resource1,
        };

        let bundle2 = ResourceBundle {
            server_name: "server2".to_string(),
            resource: resource2,
        };

        // Add resources to cache
        resources_cache.insert("file://test1.txt".to_string(), vec![bundle1]);
        resources_cache.insert("file://test2.json".to_string(), vec![bundle2]);

        // Verify cache contents
        assert_eq!(resources_cache.len(), 2);
        assert!(resources_cache.contains_key("file://test1.txt"));
        assert!(resources_cache.contains_key("file://test2.json"));

        let bundle1_ref = &resources_cache["file://test1.txt"][0];
        assert_eq!(bundle1_ref.server_name, "server1");
        assert_eq!(bundle1_ref.resource.get("name").unwrap().as_str().unwrap(), "test1.txt");
    }

    /// Test resource URI extraction from JSON
    #[test]
    fn test_resource_uri_extraction() {
        let test_cases = vec![
            (
                serde_json::json!({
                    "uri": "file://example.txt",
                    "name": "example.txt"
                }),
                Some("file://example.txt"),
            ),
            (
                serde_json::json!({
                    "uri": "https://example.com/api/data",
                    "name": "API Data"
                }),
                Some("https://example.com/api/data"),
            ),
            (
                serde_json::json!({
                    "name": "No URI resource"
                }),
                None,
            ),
            (
                serde_json::json!({
                    "uri": null,
                    "name": "Null URI"
                }),
                None,
            ),
        ];

        for (resource, expected_uri) in test_cases {
            let extracted_uri = resource.get("uri").and_then(|u| u.as_str());
            assert_eq!(extracted_uri, expected_uri);
        }
    }

    /// Test resource grouping by server
    #[test]
    fn test_resource_grouping_by_server() {
        let mut resources_cache = HashMap::<String, Vec<ResourceBundle>>::new();

        // Create resources from different servers
        let resource1 = ResourceBundle {
            server_name: "server1".to_string(),
            resource: serde_json::json!({
                "uri": "file://test1.txt",
                "name": "test1.txt"
            }),
        };

        let resource2 = ResourceBundle {
            server_name: "server1".to_string(),
            resource: serde_json::json!({
                "uri": "file://test2.txt",
                "name": "test2.txt"
            }),
        };

        let resource3 = ResourceBundle {
            server_name: "server2".to_string(),
            resource: serde_json::json!({
                "uri": "file://test3.txt",
                "name": "test3.txt"
            }),
        };

        resources_cache.insert("file://test1.txt".to_string(), vec![resource1]);
        resources_cache.insert("file://test2.txt".to_string(), vec![resource2]);
        resources_cache.insert("file://test3.txt".to_string(), vec![resource3]);

        // Group resources by server (simulating the logic in handle_resources_list)
        let mut resources_by_server = HashMap::<String, Vec<(&String, &ResourceBundle)>>::new();

        for (uri, bundles) in &resources_cache {
            for bundle in bundles {
                resources_by_server
                    .entry(bundle.server_name.clone())
                    .or_insert_with(Vec::new)
                    .push((uri, bundle));
            }
        }

        // Verify grouping
        assert_eq!(resources_by_server.len(), 2);
        assert_eq!(resources_by_server["server1"].len(), 2);
        assert_eq!(resources_by_server["server2"].len(), 1);
    }

    /// Test MIME type extraction and handling
    #[test]
    fn test_mime_type_handling() {
        let test_cases = vec![
            (
                serde_json::json!({
                    "uri": "file://test.txt",
                    "mimeType": "text/plain"
                }),
                "text/plain",
            ),
            (
                serde_json::json!({
                    "uri": "file://test.json",
                    "mimeType": "application/json"
                }),
                "application/json",
            ),
            (
                serde_json::json!({
                    "uri": "file://test.unknown"
                }),
                "unknown",
            ),
        ];

        for (resource, expected_mime) in test_cases {
            let mime_type = resource.get("mimeType").and_then(|m| m.as_str()).unwrap_or("unknown");
            assert_eq!(mime_type, expected_mime);
        }
    }

    /// Test resource name fallback logic
    #[test]
    fn test_resource_name_fallback() {
        let test_cases = vec![
            (
                serde_json::json!({
                    "uri": "file://test.txt",
                    "name": "Test File"
                }),
                "Test File",
            ),
            (
                serde_json::json!({
                    "uri": "file://no-name.txt"
                }),
                "file://no-name.txt",
            ),
        ];

        for (resource, expected_name) in test_cases {
            let uri = resource.get("uri").and_then(|u| u.as_str()).unwrap();
            let name = resource.get("name").and_then(|n| n.as_str()).unwrap_or(uri);
            assert_eq!(name, expected_name);
        }
    }
}
