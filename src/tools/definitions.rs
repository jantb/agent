use serde_json::json;

use crate::types::{ToolDefinition, ToolSource};

pub fn built_in_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the UTF-8 content of a file. Returns numbered lines. Use start_line/end_line to read specific ranges for large files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative or absolute path to read" },
                    "start_line": { "type": "integer", "description": "First line to read (1-based, inclusive). Omit to start from beginning." },
                    "end_line": { "type": "integer", "description": "Last line to read (1-based, inclusive). Omit to read to end." }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file, creating parent directories as needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to write" },
                    "content": { "type": "string", "description": "File content" }
                },
                "required": ["path", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List contents of a directory. depth=0 (default) lists immediate children only; depth>0 recurses that many levels. Max depth 10. Shows file size and modified time.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list" },
                    "depth": { "type": "integer", "description": "How many levels to recurse (0 = immediate only, max 10)" }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "edit_file".into(),
            description: "Edit a file by replacing an exact substring match. By default old_string must match exactly once. Set replace_all=true to replace every occurrence.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "old_string": { "type": "string", "description": "Exact substring to find" },
                    "new_string": { "type": "string", "description": "Replacement string" },
                    "replace_all": { "type": "boolean", "description": "If true, replace all occurrences (default false)" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "search_files".into(),
            description: "Recursively search files for a pattern. Set is_regex=true to use a regular expression. Returns filename:line:content, max 100 matches.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "String or regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory to search in" },
                    "is_regex": { "type": "boolean", "description": "Treat pattern as a regex (default false)" }
                },
                "required": ["pattern", "path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "replace_lines".into(),
            description: "Replace a range of lines in a file with new content. Lines are 1-based, inclusive.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "start_line": { "type": "integer", "description": "First line to replace (1-based)" },
                    "end_line": { "type": "integer", "description": "Last line to replace (1-based, inclusive)" },
                    "new_content": { "type": "string", "description": "Replacement text (replaces the specified line range)" }
                },
                "required": ["path", "start_line", "end_line", "new_content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "diff_files".into(),
            description: "Show a unified diff between two files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path_a": { "type": "string", "description": "First file" },
                    "path_b": { "type": "string", "description": "Second file" }
                },
                "required": ["path_a", "path_b"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "append_file".into(),
            description: "Append content to the end of a file (creates the file if it doesn't exist).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "content": { "type": "string", "description": "Content to append" }
                },
                "required": ["path", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "delete_path".into(),
            description: "Delete a file or empty directory.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to delete" }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "glob_files".into(),
            description: "Find files matching a glob pattern (e.g. '**/*.rs', 'src/*.txt'). Returns matching paths relative to the working directory. Max 200 results.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern to match" },
                    "path": { "type": "string", "description": "Base directory for the search (default '.')" }
                },
                "required": ["pattern"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "remember".into(),
            description: "Store or update a persistent memory. Use to save knowledge across sessions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short name for this memory" },
                    "content": { "type": "string", "description": "The knowledge to remember" },
                    "description": { "type": "string", "description": "One-line description of what this memory contains" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags for categorization" }
                },
                "required": ["name", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "recall".into(),
            description: "Search stored memories by keyword. Returns matching memories with their content.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "Search term to find in memory names, descriptions, tags, and content" }
                },
                "required": ["keyword"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "forget".into(),
            description: "Delete a stored memory by name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the memory to delete" }
                },
                "required": ["name"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "list_memories".into(),
            description: "List all stored memories with their names and descriptions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
            source: ToolSource::BuiltIn,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_tool_definitions_count() {
        let defs = built_in_tool_definitions();
        assert_eq!(defs.len(), 14);
        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"list_dir"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"replace_lines"));
        assert!(names.contains(&"diff_files"));
        assert!(names.contains(&"append_file"));
        assert!(names.contains(&"delete_path"));
        assert!(names.contains(&"glob_files"));
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"forget"));
        assert!(names.contains(&"list_memories"));
    }
}
