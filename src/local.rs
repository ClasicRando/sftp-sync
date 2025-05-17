use crate::CLEAR_LINE;
use std::fs::read_dir;
use std::path::{Path, PathBuf};

pub fn find_local_files<P: AsRef<Path>>(
    local_directory: P,
    excluded: &[String],
) -> anyhow::Result<Vec<PathBuf>> {
    let mut result = vec![];
    find_local_files_internal(
        local_directory.as_ref().to_path_buf(),
        PathBuf::new(),
        excluded,
        &mut result,
    )?;
    Ok(result)
}

fn find_local_files_internal(
    root_directory: PathBuf,
    relative_directory: PathBuf,
    excluded: &[String],
    result: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let full_directory_path = root_directory.join(&relative_directory);
    for entry in read_dir(full_directory_path)? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|p| p.to_str()) else {
            println!(
                "{CLEAR_LINE}\rCould not extract file name from local path {path:?}. Skipping to next item."
            );
            continue;
        };

        if file_name == "lost+found" {
            continue;
        }

        if excluded
            .binary_search_by(|e| e.as_str().cmp(file_name))
            .is_ok()
        {
            println!("{CLEAR_LINE}\rSkipping excluded file/directory {file_name}");
            continue;
        }

        print!("{CLEAR_LINE}\rChecking {path:?}");
        let relative_path = relative_directory.join(file_name);
        if entry.file_type()?.is_dir() {
            find_local_files_internal(
                root_directory.clone(),
                relative_path,
                excluded,
                result,
            )?;
            continue;
        }

        result.push(relative_path)
    }
    Ok(())
}
