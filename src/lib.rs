//! Include large files without the high compile time cost.
//!
//! This crate provides the [`include_blob!`] macro as an alternative to the standard
//! [`include_bytes!`] macro, which embeds a file by copying it to an object file and linking to it.
//! This can reduce the (quite high) compile time cost of [`include_bytes!`].
//!
//! In order for this to work, the user code has to first add a build script that calls the
//! [`make_includable`] function.
//!
//! ```no_run
//! // build.rs
//! fn main() {
//!     include_blob::make_includable("../../directory-with-big-files");
//! }
//! ```
//!
//! ```no_run
//! let bytes: &[u8] = include_blob::include_blob!("test-project/blobs/file.txt");
//! ```

use ar_archive_writer::{
    write_archive_to_stream, ArchiveKind, NewArchiveMember, DEFAULT_OBJECT_READER,
};
use object::{
    write::{Object, StandardSection, Symbol, SymbolSection},
    Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
};
use std::{
    collections::hash_map::DefaultHasher,
    env, error,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Seek, Write},
    path::{Path, PathBuf},
};

pub use include_blob_macros::*;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

/// Call this from your build script to make `path` includable via [`include_blob!`].
///
/// `path` can refer to a file or a directory (which processes every file in the directory).
///
/// `path` is relative to the directory the build script runs in (which is the package's "source
/// directory" according to Cargo's docs, so probably the directory containing `Cargo.toml`).
pub fn make_includable<A: AsRef<Path>>(path: A) {
    make_includable_impl(path.as_ref()).unwrap();
}

fn make_includable_impl(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| {
        panic!(
            "could not find file '{}' (working directory is '{}')",
            path.display(),
            std::env::current_dir().unwrap().display(),
        );
    });
    println!("cargo:rerun-if-changed={}", path.display());
    let metadata = fs::metadata(&path)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            make_includable_impl(&entry.path())?;
        }
        Ok(())
    } else if metadata.is_file() {
        process_file(path, metadata)
    } else {
        panic!(
            "cannot handle file type '{:?}' of '{}'",
            metadata.file_type(),
            path.display()
        );
    }
}

fn process_file(path: PathBuf, metadata: fs::Metadata) -> Result<()> {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.modified()?.hash(&mut hasher);
    let unique_name = format!("include_blob_{:016x}", hasher.finish());

    let content = fs::read(&path)?;

    let (pre, post) = lib_prefix_and_suffix();
    let out_dir = env::var("OUT_DIR")?;
    let out_file_path = format!("{out_dir}/{pre}{unique_name}{post}");
    let mut out_file = File::create(&out_file_path)?;

    let info = TargetInfo::from_build_script_vars();
    let mut obj_buf = Vec::new();
    let mut object = Object::new(info.binfmt, info.arch, info.endian);
    let section = object.add_subsection(StandardSection::ReadOnlyData, unique_name.as_bytes());
    let symbol_name = unique_name.as_bytes().to_vec();
    let sym = object.add_symbol(Symbol {
        name: symbol_name.clone(),
        value: 0,
        size: content.len() as _,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(section),
        flags: SymbolFlags::None,
    });
    object.add_symbol_data(sym, section, &content, 1);
    object.write_stream(&mut obj_buf)?;

    let object_file_name = format!("{unique_name}.o").into_bytes();
    write_archive(&info, &mut out_file, &object_file_name, &obj_buf)?;

    println!("cargo:rustc-link-lib=static={unique_name}");
    println!("cargo:rustc-link-search=native={out_dir}");
    Ok(())
}

fn write_archive(
    target_info: &TargetInfo,
    out_file: &mut (impl Write + Seek),
    object_file_name: &[u8],
    object_file_contents: &[u8],
) -> Result<()> {
    let member = NewArchiveMember {
        buf: Box::new(object_file_contents),
        object_reader: &DEFAULT_OBJECT_READER,
        member_name: String::from_utf8(object_file_name.to_vec()).unwrap(),
        mtime: 0,
        uid: 0,
        gid: 0,
        perms: 0o644,
    };
    write_archive_to_stream(out_file, &[member], target_info.archive_kind, false, false)?;

    Ok(())
}

struct TargetInfo {
    binfmt: BinaryFormat,
    arch: Architecture,
    endian: Endianness,
    archive_kind: ArchiveKind,
}

impl TargetInfo {
    fn from_build_script_vars() -> Self {
        let (binfmt, archive_kind) = match &*env::var("CARGO_CFG_TARGET_OS").unwrap() {
            "macos" | "ios" => (BinaryFormat::MachO, ArchiveKind::Darwin64),
            "windows" => (BinaryFormat::Coff, ArchiveKind::Gnu),
            "linux" | "android" => (BinaryFormat::Elf, ArchiveKind::Gnu),
            unk => panic!("unhandled operating system '{unk}'"),
        };
        let arch = match &*env::var("CARGO_CFG_TARGET_ARCH").unwrap() {
            // NB: this is guesswork, because apparently the Rust team can't be bothered to document
            // the *full* list anywhere (they differ from what the target triples use, which *are*
            // fully documented)
            "x86" => Architecture::I386,
            "x86_64" => Architecture::X86_64,
            "arm" => Architecture::Arm,
            "aarch64" => Architecture::Aarch64,
            "riscv32" => Architecture::Riscv32,
            "riscv64" => Architecture::Riscv64,
            "mips" => Architecture::Mips,
            "mips64" => Architecture::Mips64,
            "powerpc" => Architecture::PowerPc,
            "powerpc64" => Architecture::PowerPc64,
            unk => panic!("unhandled architecture '{unk}'"),
        };
        let endian = match &*env::var("CARGO_CFG_TARGET_ENDIAN").unwrap() {
            "little" => Endianness::Little,
            "big" => Endianness::Big,
            unk => unreachable!("unhandled endianness '{unk}'"),
        };

        Self {
            binfmt,
            arch,
            endian,
            archive_kind,
        }
    }
}

fn lib_prefix_and_suffix() -> (&'static str, &'static str) {
    if env::var_os("CARGO_CFG_UNIX").is_some() {
        ("lib", ".a")
    } else if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        ("", ".lib")
    } else {
        unimplemented!("target platform not supported");
    }
}
