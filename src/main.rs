mod local;
mod remote;

use crate::local::find_local_files;
use crate::remote::find_remote_files;
use clap::Parser;
use rayon::prelude::*;
use ssh2::{Session, Sftp};
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};
use std::process::exit;

const BUFFER_SIZE: usize = 1024 * 128;
pub const CLEAR_LINE: &str = "\x1B[2K";

fn main() -> anyhow::Result<()> {
    ctrlc::set_handler(terminate)?;
    hide_cursor();
    let mut args = Args::parse();
    args.exclude.sort();
    if args.password.is_empty() {
        match rpassword::prompt_password(format!("SFTP Password for {}: ", args.username)) {
            Ok(inner) => args.password = inner,
            Err(error) => {
                println!("Error getting password from user. {error}");
                show_cursor()
            }
        }
    }
    
    let sftp = match create_sftp_connection(&args.ip, args.port, &args.username, &args.password) {
        Ok(inner) => inner,
        Err(error) => {
            println!("Error attempting to create an SFTP connection. {error}");
            show_cursor()
        }
    };
    if let Err(error) = sync_local_directory(sftp, &args) {
        println!(
            "Error syncing local directory {:?} with remote directory {:?}. {error}\n",
            args.local_directory, args.remote_directory
        );
    }
    show_cursor()
}

fn hide_cursor() {
    print!("\x1B[?25l")
}

fn show_cursor() -> ! {
    print!("\x1B[?25h");
    exit(0)
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long)]
    ip: String,
    #[arg(short, long, default_value_t = 22)]
    port: u16,
    #[arg(long)]
    username: String,
    #[arg(long, default_value_t = String::new())]
    password: String,
    #[arg(long, default_values_t = &[])]
    exclude: Vec<String>,
    #[arg(short, long)]
    local_directory: PathBuf,
    #[arg(short, long)]
    remote_directory: PathBuf,
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

trait VecExt<T>
where
    T: PartialEq,
{
    fn index_of(&self, item: &T) -> Option<usize>;

    fn remove_first(&mut self, item: &T) -> Option<T>;
}

impl<T> VecExt<T> for Vec<T>
where
    T: PartialEq,
{
    fn index_of(&self, item: &T) -> Option<usize> {
        self.iter()
            .enumerate()
            .find(|(_, e)| *e == item)
            .map(|(i, _)| i)
    }

    fn remove_first(&mut self, item: &T) -> Option<T> {
        let Some(i) = self.index_of(item) else {
            return None;
        };
        Some(self.remove(i))
    }
}

enum SyncEntry {
    Copy {
        local_path: PathBuf,
        remote_path: PathBuf,
    },
    Delete {
        local_path: PathBuf,
    },
}

fn sync_local_directory(client: Sftp, args: &Args) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.local_directory)?;

    let mut paths = Vec::new();
    println!("Finding paths to files that need to be added, replaced or removed.");
    find_paths(&client, &args, &mut paths)?;

    println!("\nNeed to update {} files", paths.len());
    paths
        .into_par_iter()
        .for_each(|sync_entry| match sync_entry {
            SyncEntry::Copy {
                remote_path,
                local_path,
            } => {
                if args.dry_run {
                    println!("Copying {remote_path:?} -> {local_path:?}");
                    return;
                }
                if let Err(error) = copy_file(&client, &remote_path, &local_path) {
                    println!("Error copying file {remote_path:?} -> {local_path:?}. {error}");
                }
            }
            SyncEntry::Delete { local_path } => {
                if args.dry_run {
                    println!("Delete {local_path:?}");
                    return;
                }
                if let Err(error) = delete_local_file(&local_path) {
                    println!("Error deleting file {local_path:?}. {error}");
                }
            }
        });
    Ok(())
}

fn find_paths(client: &Sftp, args: &Args, result: &mut Vec<SyncEntry>) -> anyhow::Result<()> {
    println!("Finding local files");
    let mut local_files = find_local_files(&args.local_directory, &args.exclude)?;
    println!("{CLEAR_LINE}\rFinding remote files");
    let remote_files = find_remote_files(client, &args.remote_directory, &args.exclude)?;

    println!("{CLEAR_LINE}\rComparing local vs remote files");
    for (remote_file, remote_size) in remote_files {
        if let Some(local_path) = local_files.remove_first(&remote_file) {
            let local_file = File::open(args.local_directory.join(&local_path))?;
            if local_file.metadata()?.len() != remote_size {
                result.push(SyncEntry::Copy {
                    remote_path: args.remote_directory.join(remote_file),
                    local_path: args.local_directory.join(local_path),
                });
            }
            continue;
        }

        result.push(SyncEntry::Copy {
            remote_path: args.remote_directory.join(&remote_file),
            local_path: args.local_directory.join(remote_file),
        })
    }

    for local_file in local_files {
        if is_excluded_local_file(&local_file) {
            continue;
        }
        result.push(SyncEntry::Delete {
            local_path: args.local_directory.join(local_file),
        })
    }

    Ok(())
}

fn is_excluded_local_file(local_path: &Path) -> bool {
    local_path.components().any(|c| {
        let Component::Normal(part) = c else {
            return false;
        };
        let Some(part) = part.to_str() else {
            return false;
        };
        part.starts_with(".Trash")
    })
}

fn copy_file(client: &Sftp, remote_path: &Path, local_path: &Path) -> anyhow::Result<()> {
    println!("Copying remote file {remote_path:?} to {local_path:?}");
    let mut remote_file = client.open(remote_path)?;
    let mut local_file = File::create(local_path)?;
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        let bytes_read = remote_file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        local_file.write_all(&buffer[0..bytes_read])?;
    }
    Ok(())
}

fn delete_local_file(local_path: &Path) -> anyhow::Result<()> {
    println!("Deleting local file {local_path:?}");
    std::fs::remove_file(&local_path)?;
    Ok(())
}

fn create_sftp_connection(
    ip: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<Sftp, Box<dyn std::error::Error>> {
    let tcp = TcpStream::connect((ip, port))?;
    let mut ssh_session = Session::new()?;
    ssh_session.set_tcp_stream(tcp);
    ssh_session.handshake()?;
    ssh_session.userauth_password(username, password)?;

    let sftp = ssh_session.sftp()?;
    Ok(sftp)
}

fn terminate() {
    println!("\nHandling SIGTERM");
    show_cursor();
}
