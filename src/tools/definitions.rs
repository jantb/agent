use serde_json::json;

use crate::types::{ToolDefinition, ToolSource};

pub fn delegate_task_def() -> ToolDefinition {
    ToolDefinition {
        name: "delegate_task".into(),
        description: "\
Delegate a focused subtask to an isolated sub-agent running in its own fresh context window. \
The sub-agent has NO access to the current conversation history — include all necessary context \
in the prompt. It runs to completion and returns one distilled answer. \
Use when a subtask would generate excessive intermediate output (many reads, searches, etc.) \
that would pollute the current context, or when focused specialization is needed. \
Do NOT use for trivial single-step operations."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Complete, self-contained task description. Include every file path, line range, and context the sub-agent needs. The sub-agent cannot see your conversation history."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional extra instructions appended to the sub-agent's system prompt. The base prompt (sandbox rules, tool docs) is always auto-computed — this adds your specialization on top."
                }
            },
            "required": ["prompt"]
        }),
        source: ToolSource::BuiltIn,
    }
}

pub fn interview_question_def() -> ToolDefinition {
    ToolDefinition {
        name: "interview_question".into(),
        description: "Ask the user a clarifying question with suggested answers. \
Use when you encounter genuine ambiguity, unclear requirements, or need the user to decide \
something with significant implementation impact. The user can pick a suggestion or type their own answer."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "suggestions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "2-5 suggested answers for the user to choose from"
                }
            },
            "required": ["question", "suggestions"]
        }),
        source: ToolSource::BuiltIn,
    }
}

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
        ToolDefinition {
            name: "line_count".into(),
            description: "Count lines in files recursively. Returns files sorted by line count descending, plus a total. Skips hidden files and build dirs.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to scan (default '.')" },
                    "extensions": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter by file extensions, e.g. [\"rs\", \"toml\"]. Omit for all files."
                    }
                },
                "required": []
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "read_image".into(),
            description: "Read an image file and load it into the LLM's vision context. Supports PNG, JPG, GIF, WEBP. Returns a confirmation; the image is attached to the message so the model can see it.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "read_pdf".into(),
            description: "Extract text content from a PDF file. Optionally specify a page range.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the PDF file" },
                    "pages": { "type": "string", "description": "Page range (1-based, e.g. '3' or '1-5'). Omit for all pages." }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        delegate_task_def(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_tool_definitions_count() {
        let defs = built_in_tool_definitions();
        assert_eq!(defs.len(), 18);
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
        assert!(names.contains(&"line_count"));
        assert!(names.contains(&"read_image"));
        assert!(names.contains(&"read_pdf"));
        assert!(names.contains(&"delegate_task"));
    }

    #[test]
    fn delegate_task_has_system_prompt_param() {
        let def = delegate_task_def();
        let props = def.parameters.get("properties").unwrap();
        assert!(
            props.get("system_prompt").is_some(),
            "delegate_task should have system_prompt property"
        );
    }

    #[test]
    fn delegate_task_system_prompt_not_required() {
        let def = delegate_task_def();
        let required = def.parameters.get("required").unwrap().as_array().unwrap();
        let has_system_prompt = required.iter().any(|v| v.as_str() == Some("system_prompt"));
        assert!(!has_system_prompt, "system_prompt should not be required");
    }
}
