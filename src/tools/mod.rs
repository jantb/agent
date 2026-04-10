mod builtin;
mod definitions;
mod dispatch;

pub(crate) static IGNORE_DIRS: &[&str] = &[
    "target",
    ".git",
    "node_modules",
    ".cache",
    "dist",
    "build",
    "__pycache__",
    ".idea",
    ".vscode",
];

pub use definitions::{built_in_tool_definitions, delegate_task_def};
pub use dispatch::execute_built_in;
