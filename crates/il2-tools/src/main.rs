mod names;

use clap::{Parser, Subcommand};
use il2_sfs::Archive;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "il2", about = "OpenIL2 asset tools")]
struct Cli {
    /// Game install directory.
    #[arg(long, env = "IL2_ROOT")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Decode every block of every archive and report failures.
    Validate,
    /// Print summary statistics for each archive.
    Stat,
    /// Write a file from the archives to stdout.
    Cat { path: String },
    /// Parse every named .msh/.him/.mat file and report failures.
    ParseAll {
        /// Name list produced by `il2 names`.
        #[arg(long, default_value = "names.txt")]
        names: PathBuf,
    },
    /// Recover archive path names and write them, one per line, to `out`.
    Names {
        /// Extra candidate names, one per line.
        #[arg(long)]
        seeds: Option<PathBuf>,
        #[arg(long, default_value = "names.txt")]
        out: PathBuf,
    },
}

fn archives(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sfs")))
        .collect();
    v.sort();
    Ok(v)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let paths = match archives(&cli.root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e}", cli.root.display());
            return ExitCode::FAILURE;
        }
    };
    let mut failures = 0usize;
    match cli.cmd {
        Cmd::Validate => {
            let (mut files, mut blocks) = (0usize, 0usize);
            for p in &paths {
                let a = match Archive::open(p) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("FAIL {e}");
                        failures += 1;
                        continue;
                    }
                };
                let mut buf = Vec::new();
                for b in 0..a.block_count() {
                    if let Err(e) = a.read_block(b, &mut buf) {
                        eprintln!("FAIL {}: {e}", p.display());
                        failures += 1;
                    }
                }
                files += a.entries().iter().filter(|e| !e.is_dir()).count();
                blocks += a.block_count();
            }
            println!("{} archives, {files} files, {blocks} blocks, {failures} failures", paths.len());
        }
        Cmd::Stat => {
            for p in &paths {
                match Archive::open(p) {
                    Ok(a) => {
                        let dirs = a.entries().iter().filter(|e| e.is_dir()).count();
                        let bytes: u64 = a.entries().iter().map(|e| e.size as u64).sum();
                        println!(
                            "{:<28} {:>7} files {:>5} dirs {:>12} bytes",
                            p.file_name().unwrap().to_string_lossy(),
                            a.entries().len() - dirs,
                            dirs,
                            bytes
                        );
                    }
                    Err(e) => {
                        eprintln!("FAIL {e}");
                        failures += 1;
                    }
                }
            }
        }
        Cmd::Cat { path } => match il2_vfs::Vfs::open_install(&cli.root).and_then(|v| v.read(&path)) {
            Ok(data) => {
                std::io::stdout().write_all(&data).ok();
            }
            Err(e) => {
                eprintln!("{e}");
                failures += 1;
            }
        },
        Cmd::ParseAll { names } => {
            let vfs = match il2_vfs::Vfs::open_install(&cli.root) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            let list = std::fs::read_to_string(&names).unwrap_or_default();
            let mut stats: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
            for name in list.lines() {
                let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                if !matches!(ext.as_str(), "msh" | "him" | "mat") {
                    continue;
                }
                let result = vfs.read(name).map_err(|e| e.to_string()).and_then(|d| match ext.as_str() {
                    "msh" => il2_formats::msh::Mesh::parse(&d).map(|_| ()).map_err(|e| e.to_string()),
                    "him" => il2_formats::him::Hier::parse(&d).map(|_| ()).map_err(|e| e.to_string()),
                    _ => {
                        il2_formats::mat::MatFile::parse(&d);
                        Ok(())
                    }
                });
                let st = stats.entry(ext).or_default();
                st.0 += 1;
                if let Err(e) = result {
                    st.1 += 1;
                    if st.1 <= 5 {
                        eprintln!("FAIL {name}: {e}");
                    }
                }
            }
            for (ext, (n, f)) in &stats {
                println!("{ext}: {n} parsed, {f} failed");
                failures += f;
            }
        }
        Cmd::Names { seeds, out } => {
            let vfs = match il2_vfs::Vfs::open_install(&cli.root) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            let seeds: Vec<String> = seeds
                .map(|p| std::fs::read_to_string(p).unwrap_or_default().lines().map(str::to_owned).collect())
                .unwrap_or_default();
            let n = names::Names::recover(&vfs, &seeds);
            let mut list: Vec<&String> = n.found.values().collect();
            list.sort();
            let text: String = list.iter().map(|s| format!("{s}\n")).collect();
            if let Err(e) = std::fs::write(&out, text) {
                eprintln!("{}: {e}", out.display());
                failures += 1;
            }
        }
    }
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
