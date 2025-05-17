use crate::CLEAR_LINE;
use ssh2::Sftp;
use std::path::{Path, PathBuf};

pub fn find_remote_files<P: AsRef<Path>>(
    client: &Sftp,
    remote_directory: P,
    excluded: &[String],
) -> anyhow::Result<Vec<(PathBuf, u64)>> {
    let mut result = vec![];
    find_remote_files_internal(
        client,
        remote_directory.as_ref().to_path_buf(),
        PathBuf::new(),
        excluded,
        &mut result,
    )?;
    Ok(result)
}

fn find_remote_files_internal(
    client: &Sftp,
    root_directory: PathBuf,
    relative_directory: PathBuf,
    excluded: &[String],
    remote_files: &mut Vec<(PathBuf, u64)>,
) -> anyhow::Result<()> {
    let full_directory_path = root_directory.join(&relative_directory);
    for (path, stat) in client.readdir(&full_directory_path)? {
        let Some(file_name) = path.file_name().and_then(|p| p.to_str()) else {
            println!(
                "{CLEAR_LINE}\rCould not extract file name from remote path {path:?}. Skipping to next item."
            );
            continue;
        };

        if excluded
            .binary_search_by(|e| e.as_str().cmp(file_name))
            .is_ok()
        {
            println!("{CLEAR_LINE}\rSkipping excluded file/directory {file_name}");
            continue;
        }

        print!("{CLEAR_LINE}\rChecking {path:?}");
        let relative_path = relative_directory.join(file_name);
        if stat.is_dir() {
            find_remote_files_internal(
                client,
                root_directory.clone(),
                relative_path,
                excluded,
                remote_files,
            )?;
            continue;
        }

        let Some(remote_size) = &stat.size else {
            println!(
                "{CLEAR_LINE}\rCould not extract file size from the remote path {path:?}. Skipping to next item"
            );
            continue;
        };
        remote_files.push((relative_path, *remote_size))
    }
    Ok(())
}
