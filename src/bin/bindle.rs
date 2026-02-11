use clap::{Parser, Subcommand};
use std::io::{self};
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
        /// Path to the local file to read from (reads from stdin if omitted)
        file_path: Option<PathBuf>,
        /// Use zstd compression
        #[arg(short, long)]
        compress: bool,
        /// Pass data directly as an argument
        #[arg(short, long, conflicts_with = "file_path")]
        data: Option<String>,
        /// Run vacuum after adding
        #[arg(long)]
        vacuum: bool,
    },

    #[command(visible_alias = "cat")]
    /// Extract an entry's data
    Read {
        /// Bindle archive file
        #[arg(value_name = "BINDLE_FILE")]
        bindle_file: PathBuf,
        /// Name of the entry to extract
        name: String,
        /// Output path
        #[arg(short, long)]
        output: Option<PathBuf>,
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

    let init_load = |path: PathBuf| match Bindle::load(&path) {
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
            let b = init_load(bindle_file);

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
            data: data_arg,
            compress,
            bindle_file,
            vacuum,
        } => {
            let mut b = init(bindle_file.clone());
            let compress_mode = if compress {
                Compress::Zstd
            } else {
                Compress::None
            };

            // Determine data source and method: --data flag, file path, or stdin
            let size = if let Some(d) = data_arg {
                // Direct data from argument
                let bytes = d.into_bytes();
                let len = bytes.len();
                b.add(&name, &bytes, compress_mode)?;
                len
            } else if let Some(path) = file_path {
                // Use add_file to avoid loading entire file into memory
                b.add_file(&name, &path, compress_mode)?;
                std::fs::metadata(&path)?.len() as usize
            } else {
                // Stream from stdin using writer
                let mut writer = b.writer(&name, compress_mode)?;
                let size = io::copy(&mut io::stdin(), &mut writer)?;
                writer.close()?;
                size as usize
            };

            println!(
                "ADD '{}' -> {} ({} bytes)",
                name,
                bindle_file.display(),
                size
            );
            b.save()?;

            if vacuum {
                println!("VACUUM {}", bindle_file.display());
                b.vacuum()?;
            }

            println!("OK");
        }

        Commands::Read {
            name,
            bindle_file,
            output,
        } => {
            let b = init_load(bindle_file.clone());
            let res = if let Some(output) = &output {
                b.read_to(name.as_str(), std::fs::File::create(output)?)
            } else {
                b.read_to(name.as_str(), io::stdout())
            };
            match res {
                Ok(_n) => {
                    if output.is_some() {
                        println!("OK")
                    }
                }
                Err(e) => {
                    return Err(io::Error::new(io::ErrorKind::NotFound, e));
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
            let b = init_load(bindle_file);
            b.unpack(dest_dir)?;
            println!("OK");
        }

        Commands::Vacuum { bindle_file } => {
            println!("VACUUM {}", bindle_file.display());
            let mut b = init_load(bindle_file);
            b.vacuum()?;
            println!("OK");
        }
    }
    Ok(())
}
