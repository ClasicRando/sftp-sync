use clap::Parser;
use rayon::prelude::*;
use ssh2::{Session, Sftp};
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::exit;

const BUFFER_SIZE: usize = 1024 * 128;
const CLEAR_LINE: &str = "\x1B[2K";

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
    #[arg(long)]
    password: Option<String>,
    #[arg(long)]
    exclude: Option<Vec<String>>,
    #[arg(short, long)]
    local_directory: PathBuf,
    #[arg(short, long)]
    remote_directory: PathBuf,
}

struct SftpSync {
    client: Sftp,
    exclude: Vec<String>,
    local_directory: PathBuf,
    remote_directory: PathBuf,
}

impl SftpSync {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(
        client: Sftp,
        exclude: Option<Vec<String>>,
        local_directory: P,
        remote_directory: Q,
    ) -> Self {
        let exclude = if let Some(mut e) = exclude {
            e.sort();
            e
        } else {
            Default::default()
        };
        Self {
            client,
            exclude,
            local_directory: local_directory.as_ref().to_path_buf(),
            remote_directory: remote_directory.as_ref().to_path_buf(),
        }
    }

    fn copy_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("Copying remote file {remote_path:?} to {local_path:?}");
        let mut remote_file = self.client.open(remote_path)?;
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

    fn find_paths<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        local_directory: P,
        remote_directory: Q,
        result: &mut Vec<(PathBuf, PathBuf)>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let local_directory = local_directory.as_ref();
        let remote_directory = remote_directory.as_ref();
        std::fs::create_dir_all(local_directory)?;
        for (path, stat) in self.client.readdir(remote_directory)? {
            let Some(file_name) = path.file_name().and_then(|p| p.to_str()) else {
                println!(
                    "{CLEAR_LINE}\rCould not extract file name from remote path {path:?}. Skipping to next item."
                );
                continue;
            };

            if self
                .exclude
                .binary_search_by(|e| e.as_str().cmp(file_name))
                .is_ok()
            {
                println!("{CLEAR_LINE}\rSkipping excluded file/directory {file_name}");
                continue;
            }

            if stat.is_dir() {
                let child_local_dir = local_directory.join(file_name);
                self.find_paths(child_local_dir, path, result)?;
                continue;
            }

            print!("{CLEAR_LINE}\rChecking {path:?} for a download or replace");

            let Some(remote_size) = &stat.size else {
                println!(
                    "{CLEAR_LINE}\rCould not extract file size from the remote path {path:?}. Skipping to next item"
                );
                continue;
            };

            let local_path = local_directory.join(file_name);
            if !local_path.exists() {
                result.push((path, local_path));
                continue;
            }

            let local_file = File::open(&local_path)?;
            if local_file.metadata()?.len() != *remote_size {
                result.push((path, local_path));
            }
        }
        Ok(())
    }

    pub fn sync_local_directory(&self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.local_directory.exists() {
            return Err(
                format!("Local directory {:?} does not exist", self.local_directory).into(),
            );
        }
        let mut paths = Vec::new();
        println!("Finding paths that need to files that needs to be added or replaced.");
        self.find_paths(&self.local_directory, &self.remote_directory, &mut paths)?;
        print!("{CLEAR_LINE}\r");

        println!("Need to update {} files", paths.len());
        paths.into_par_iter().for_each(|(remote_path, local_path)| {
            if let Err(error) = self.copy_file(&remote_path, &local_path) {
                println!("Error copying file {remote_path:?} -> {local_path:?}. {error}");
            }
        });
        Ok(())
    }
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

fn main() {
    if let Err(error) = ctrlc::set_handler(terminate) {
        println!("Failed to set handler for SIGTERM. {error}");
        return;
    }
    hide_cursor();
    let args = Args::parse();
    let password = match args.password {
        Some(inner) => inner,
        None => {
            match rpassword::prompt_password(format!("SFTP Password for {}: ", args.username)) {
                Ok(inner) => inner,
                Err(error) => {
                    println!("Error getting password from user. {error}");
                    show_cursor()
                }
            }
        }
    };
    let sftp = match create_sftp_connection(&args.ip, args.port, &args.username, &password) {
        Ok(inner) => inner,
        Err(error) => {
            println!("Error attempting to create an SFTP connection. {error}");
            show_cursor()
        }
    };
    let sync = SftpSync::new(
        sftp,
        args.exclude,
        &args.local_directory,
        &args.remote_directory,
    );
    if let Err(error) = sync.sync_local_directory() {
        println!(
            "Error syncing local directory {:?} with remote directory {:?}. {error}\n",
            args.local_directory, args.remote_directory
        );
        show_cursor()
    }
    show_cursor()
}
