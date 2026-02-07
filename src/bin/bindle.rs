use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

use bindle_file::{Bindle, Compress};

#[derive(Parser)]
#[command(name = "bindle")]
#[command(version = "1.0")]
#[command(author = "zshipko")]
#[command(about = "Append-only file collection")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all entries in the archive
    List {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
    },

    /// Add a file to the archive
    Add {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,

        /// Name of the entry inside the archive
        name: String,
        /// Path to the local file to read from
        file_path: PathBuf,
        /// Use zstd compression
        #[arg(short, long)]
        compress: bool,
        /// Run vacuum after adding
        #[arg(long)]
        vacuum: bool,
    },

    /// Extract an entry's data to stdout
    Cat {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
        /// Name of the entry to extract
        name: String,
    },

    /// Remove an entry from the archive
    Remove {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
        /// Name of the entry to remove
        name: String,
        /// Run vacuum after removing
        #[arg(long)]
        vacuum: bool,
    },

    /// Pack an entire directory into the archive
    Pack {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
        /// Local directory to pack
        #[arg(value_name = "SRC_DIR")]
        src_dir: PathBuf,
        /// Use zstd compression
        #[arg(short, long)]
        compress: bool,
        /// Append to existing file
        #[arg(short, long)]
        append: bool,
        /// Run vacuum after packing
        #[arg(long)]
        vacuum: bool,
    },

    /// Unpack the archive to a local directory
    Unpack {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
        /// Destination directory
        #[arg(value_name = "DEST_DIR")]
        dest_dir: PathBuf,
    },

    /// Reclaim space by removing shadowed/deleted data
    Vacuum {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = handle_command(cli.command) {
        eprintln!("ERROR {}", e);
        process::exit(1);
    }
}

fn handle_command(command: Commands) -> io::Result<()> {
    let init = |path: PathBuf| match Bindle::open(&path) {
        Ok(bindle) => bindle,
        Err(e) => {
            eprintln!("ERROR unable to open {}: {}", path.display(), e);
            process::exit(1);
        }
    };

    match command {
        Commands::List { bindle_file } => {
            println!(
                "{:<30} {:<12} {:<12} {:<10}",
                "NAME", "SIZE", "PACKED", "RATIO"
            );
            println!("{}", "-".repeat(70));
            if !bindle_file.exists() {
                return Ok(());
            }
            let b = init(bindle_file);

            for (name, entry) in b.index().iter() {
                let size = entry.uncompressed_size();
                let packed = entry.compressed_size();

                let ratio = if size > 0 {
                    (packed as f64 / size as f64) * 100.0
                } else {
                    100.0
                };

                println!("{:<30} {:<12} {:<12} {:.1}%", name, size, packed, ratio);
            }
        }

        Commands::Add {
            name,
            file_path,
            compress,
            bindle_file,
            vacuum,
        } => {
            let mut b = init(bindle_file.clone());
            let data = std::fs::read(&file_path)?;

            b.add(
                &name,
                &data,
                if compress {
                    Compress::Zstd
                } else {
                    Compress::None
                },
            )?;
            println!(
                "ADD '{}' -> {} ({} bytes)",
                name,
                bindle_file.display(),
                data.len()
            );
            b.save()?;

            if vacuum {
                println!("VACUUM {}", bindle_file.display());
                b.vacuum()?;
            }

            println!("OK");
        }

        Commands::Cat { name, bindle_file } => {
            let b = init(bindle_file.clone());
            match b.read(&name) {
                Some(data) => {
                    io::stdout().write_all(&data)?;
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("ERROR '{}' not found in {}", name, bindle_file.display()),
                    ));
                }
            }
        }

        Commands::Remove {
            name,
            bindle_file,
            vacuum,
        } => {
            let mut b = init(bindle_file.clone());
            if b.remove(&name) {
                println!("REMOVE '{}' from {}", name, bindle_file.display());
                b.save()?;

                if vacuum {
                    println!("VACUUM {}", bindle_file.display());
                    b.vacuum()?;
                }

                println!("OK");
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("ERROR '{}' not found in {}", name, bindle_file.display()),
                ));
            }
        }

        Commands::Pack {
            bindle_file,
            src_dir,
            compress,
            append,
            vacuum,
        } => {
            println!("PACK {} -> {}", src_dir.display(), bindle_file.display());
            let mut b = init(bindle_file.clone());
            if !append {
                b.clear();
            }
            b.pack(
                src_dir,
                if compress {
                    Compress::Zstd
                } else {
                    Compress::None
                },
            )?;
            b.save()?;

            if vacuum {
                println!("VACUUM {}", bindle_file.display());
                b.vacuum()?;
            }

            println!("OK");
        }

        Commands::Unpack {
            bindle_file,
            dest_dir,
        } => {
            println!("UNPACK {} -> {}", bindle_file.display(), dest_dir.display());
            let b = init(bindle_file);
            b.unpack(dest_dir)?;
            println!("OK");
        }

        Commands::Vacuum { bindle_file } => {
            println!("VACUUM {}", bindle_file.display());
            let mut b = init(bindle_file);
            b.vacuum()?;
            println!("OK");
        }
    }
    Ok(())
}
