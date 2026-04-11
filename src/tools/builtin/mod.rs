mod file_io;
mod memory_tools;
mod search;
mod text_ops;

pub use file_io::{
    run_append_file, run_delete_path, run_list_dir, run_read_file, run_read_image, run_read_pdf,
    run_write_file,
};
pub use memory_tools::{run_forget, run_list_memories, run_recall, run_remember};
pub use search::{run_glob_files, run_line_count, run_search_files};
pub use text_ops::{run_diff_files, run_edit_file, run_replace_lines};

use std::path::{Path, PathBuf};

pub(crate) fn resolve_path(requested: &str, working_dir: &Path) -> PathBuf {
    let p = Path::new(requested);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
}
